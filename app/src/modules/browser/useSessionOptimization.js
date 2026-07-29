// 会话优化编排:魔法棒 → 会话绑定的优化器角色(headless Agent)生成改写候选 →
// 内联 diff 逐条/批量取舍 → 接受项合成一个批次经 operations 写回(与手动改写同通道)。
// Agent 只负责"想",写回不经过 Agent:临时 runtime 会话仅做 session_read +
// session_edit(preview),跑完即删,用户接受的子集由前端直接 plan/apply。
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { onRuntimeEvent, runtime } from "../../platform/desktop/client.js";
import { operations } from "../operations/public.js";
import { buildSessionPrompt, sessionAttachment, sessionIdentity }
  from "./sessionAttachment.js";

export const SESSION_OPTIMIZATION_PURPOSE = "session-optimization";
export const SESSION_OPTIMIZER_ROLE_ID = "session-optimizer";

const ROLE_BINDING_KEY = "ferry-session-optimizer-roles";
const REASONS_PREFIX = "REASONS:";

/** 优化器是显式白名单:角色必须勾选「用作会话优化器」,并具备读写会话的
 *  工具(否则跑不出 preview 候选)。 */
export const isOptimizerRole = role =>
  role?.optimizer === true
  && Array.isArray(role?.tools)
  && role.tools.includes("session_read")
  && role.tools.includes("session_edit");

function readBindings() {
  try {
    const parsed = JSON.parse(localStorage.getItem(ROLE_BINDING_KEY) || "{}");
    return parsed && typeof parsed === "object" ? parsed : {};
  } catch {
    return {};
  }
}

function writeBinding(identity, roleId) {
  if (!identity) return;
  const bindings = readBindings();
  bindings[identity] = roleId;
  try {
    localStorage.setItem(ROLE_BINDING_KEY, JSON.stringify(bindings));
  } catch {}
}

/**
 * 生成给优化 Agent 的指令。范围(整段/若干轮)写进自然语言;只允许 preview,
 * execute 被明确禁止——写回由用户在浏览界面逐条决定后走前端通道。
 * 末尾要求单独一行 REASONS: JSON,前端据此给每条候选配一句改写理由。
 */
export function buildOptimizationInstruction(turns) {
  const scope = Array.isArray(turns) && turns.length
    ? `只处理第 ${turns.join("、")} 轮的用户提问`
    : "通读全部轮次,找出表述不清、缺少上下文或容易被误解的用户提问";
  return [
    `请优化附件会话中的用户提问:${scope}。`,
    "步骤:",
    "1. 用 session_read 读取目标消息(必要时用 from_message 分页),只有 editable=true 的用户消息可以改写;",
    "2. 把全部改写作为一个批次调用 session_edit(intent:\"preview\");禁止调用 execute,最终是否写回由用户在界面上逐条决定;",
    "3. 改写必须忠实原意,不得虚构原文没有的背景、需求或约束;没有值得改写的消息就不要调用 session_edit。",
    "全部完成后,最后单独输出一行(不要包裹代码块、不要附加其他文字):",
    'REASONS: {"reasons":[{"locator":"fml_...","reason":"一句话中文理由"}]}',
    "没有候选时输出 REASONS: {\"reasons\":[]}。",
  ].join("\n");
}

/** 从 Agent 全部输出文本里解析最后一行 REASONS: JSON;解析不出就当没有理由。 */
export function parseReasons(text) {
  const lines = String(text || "").split("\n");
  for (let i = lines.length - 1; i >= 0; i--) {
    const line = lines[i].trim();
    if (!line.startsWith(REASONS_PREFIX)) continue;
    try {
      const parsed = JSON.parse(line.slice(REASONS_PREFIX.length).trim());
      const map = {};
      for (const item of parsed?.reasons || []) {
        if (typeof item?.locator === "string" && typeof item?.reason === "string") {
          map[item.locator] = item.reason;
        }
      }
      return map;
    } catch {
      return {};
    }
  }
  return {};
}

export function useSessionOptimization({
  current,
  roles,
  runtimeProbe,
  doScan,
  onApplied,
}) {
  const identity = current ? sessionIdentity(current) : "";
  const eligibleRoles = useMemo(
    () => (roles || []).filter(isOptimizerRole),
    [roles],
  );

  // 会话级角色绑定:默认内置优化器;绑定的角色被删则回落默认
  const [bindingVersion, setBindingVersion] = useState(0);
  const roleId = useMemo(() => {
    void bindingVersion;
    const bound = identity ? readBindings()[identity] : null;
    if (bound && eligibleRoles.some(role => role.id === bound)) return bound;
    if (eligibleRoles.some(role => role.id === SESSION_OPTIMIZER_ROLE_ID)) {
      return SESSION_OPTIMIZER_ROLE_ID;
    }
    return eligibleRoles[0]?.id || null;
  }, [identity, eligibleRoles, bindingVersion]);
  const setRoleId = useCallback(nextRoleId => {
    writeBinding(identity, nextRoleId);
    setBindingVersion(version => version + 1);
  }, [identity]);
  const role = eligibleRoles.find(item => item.id === roleId) || null;

  const [status, setStatus] = useState("idle"); // idle|running|reviewing|applying
  const [progressTool, setProgressTool] = useState(null);
  const [candidates, setCandidates] = useState([]); // [{locator,text,reason}]
  const [error, setError] = useState(null);

  // 运行期可变状态集中在 ref:事件监听器生命周期独立于渲染
  const runRef = useRef(null);
  const statusRef = useRef(status); statusRef.current = status;

  const cleanupRun = useCallback(async (deleteSession = true) => {
    const run = runRef.current;
    runRef.current = null;
    setProgressTool(null);
    if (!run) return;
    run.unlisten?.();
    if (deleteSession && run.sessionId) {
      await runtime("session.delete", { session_id: run.sessionId })
        .catch(() => {});
    }
  }, []);
  useEffect(() => () => { cleanupRun(); }, [cleanupRun]);
  // 切换浏览的会话时丢弃进行中/待审的一切
  useEffect(() => {
    cleanupRun();
    setCandidates([]);
    setStatus("idle");
    setError(null);
  }, [identity, cleanupRun]);

  const finishRun = useCallback(run => {
    const reasons = parseReasons(run.finalText);
    const ops = run.lastPreview || [];
    const next = ops
      .filter(op => op.op === "rewrite" && typeof op.locator === "string"
        && typeof op.text === "string")
      .map(op => ({
        locator: op.locator,
        text: op.text,
        reason: reasons[op.locator] || "",
      }));
    setCandidates(next);
    setStatus(next.length ? "reviewing" : "idle");
    if (!next.length) setError({ kind: "empty" });
  }, []);

  const start = useCallback(async turns => {
    if (!current || !role || statusRef.current === "running"
      || statusRef.current === "applying") return;
    const attachment = sessionAttachment(current);
    if (!attachment) return;
    setError(null);
    setCandidates([]);
    setStatus("running");
    setProgressTool(null);
    let sessionId = null;
    try {
      const state = await runtime("session.create", {
        role_id: role.id,
        purpose: SESSION_OPTIMIZATION_PURPOSE,
      });
      sessionId = state.session_id;
      const run = {
        sessionId,
        finalText: "",
        lastPreview: null,
        pendingPreviews: new Map(), // tool_call_id -> ops
        unlisten: null,
      };
      runRef.current = run;
      run.unlisten = await onRuntimeEvent(event => {
        if (runRef.current !== run || event.session_id !== sessionId) return;
        const payload = event.payload || {};
        switch (event.type) {
          case "content.delta":
            run.finalText += payload.delta || "";
            break;
          case "tool.started":
            setProgressTool(payload.name || null);
            if (payload.name === "session_edit"
              && payload.args?.intent === "preview"
              && Array.isArray(payload.args?.ops)) {
              run.pendingPreviews.set(payload.tool_call_id, payload.args.ops);
            }
            break;
          case "tool.completed": {
            setProgressTool(null);
            const ops = run.pendingPreviews.get(payload.tool_call_id);
            if (ops && !payload.is_error) run.lastPreview = ops;
            run.pendingPreviews.delete(payload.tool_call_id);
            break;
          }
          case "run.completed":
            cleanupRun();
            finishRun(run);
            break;
          case "run.failed":
          case "run.cancelled":
          case "run.interrupted":
            cleanupRun();
            setStatus("idle");
            if (event.type === "run.failed") {
              setError({ kind: "failed", message: payload.message || "" });
            }
            break;
          default:
            break;
        }
      });
      // 订阅期间可能已被切换会话清理掉
      if (runRef.current !== run) { run.unlisten(); return; }
      await runtime("prompt", {
        session_id: sessionId,
        text: buildSessionPrompt(
          buildOptimizationInstruction(turns), [attachment]),
        display_text: buildOptimizationInstruction(turns),
      });
    } catch (err) {
      await cleanupRun();
      if (!runRef.current && sessionId) {
        await runtime("session.delete", { session_id: sessionId })
          .catch(() => {});
      }
      setStatus("idle");
      setError({ kind: "failed", message: String(err?.message || err) });
    }
  }, [current, role, cleanupRun, finishRun]);

  const stop = useCallback(async () => {
    const run = runRef.current;
    if (!run) return;
    // abort 触发 run.cancelled,监听器里做清理;这里兜底直接清
    await runtime("abort", { session_id: run.sessionId }).catch(() => {});
    setStatus("idle");
  }, []);

  const applyBatch = useCallback(async accepted => {
    if (!accepted.length) { setStatus("idle"); return; }
    setStatus("applying");
    try {
      await operations.execute({
        kind: "edit",
        tool: current.tool,
        ref: current.ref,
        ops: accepted.map(item =>
          ({ op: "rewrite", locator: item.locator, text: item.text })),
        probe: !!runtimeProbe,
      });
      doScan?.();
      onApplied?.();
      setStatus("idle");
    } catch (err) {
      setStatus("reviewing");
      setError({ kind: "apply_failed", message: String(err?.message || err) });
      // 写回失败:候选留在界面上,用户可重试或放弃
      setCandidates(accepted);
      return;
    }
  }, [current, runtimeProbe, doScan, onApplied]);

  // 逐条/批量取舍。接受的暂存,全部处理完后一次性作为一个批次写回。
  const acceptedRef = useRef([]);
  useEffect(() => { if (status === "running") acceptedRef.current = []; }, [status]);
  const resolve = useCallback((locator, accept) => {
    setCandidates(previous => {
      const target = previous.find(item => item.locator === locator);
      if (!target) return previous;
      if (accept) acceptedRef.current = [...acceptedRef.current, target];
      const remaining = previous.filter(item => item.locator !== locator);
      if (!remaining.length) {
        const accepted = acceptedRef.current;
        acceptedRef.current = [];
        // setState 内不做副作用,推到微任务
        queueMicrotask(() => applyBatch(accepted));
      }
      return remaining;
    });
  }, [applyBatch]);
  const acceptAll = useCallback(() => {
    setCandidates(previous => {
      if (previous.length) {
        const accepted = [...acceptedRef.current, ...previous];
        acceptedRef.current = [];
        queueMicrotask(() => applyBatch(accepted));
      }
      return [];
    });
  }, [applyBatch]);
  const rejectAll = useCallback(() => {
    setCandidates(previous => {
      if (previous.length || acceptedRef.current.length) {
        const accepted = acceptedRef.current;
        acceptedRef.current = [];
        queueMicrotask(() => applyBatch(accepted));
      }
      return [];
    });
  }, [applyBatch]);
  const discard = useCallback(() => {
    acceptedRef.current = [];
    setCandidates([]);
    setStatus("idle");
    setError(null);
  }, []);

  const clearError = useCallback(() => setError(null), []);

  return useMemo(() => ({
    available: eligibleRoles.length > 0,
    eligibleRoles,
    roleId,
    role,
    setRoleId,
    status,
    progressTool,
    candidates,
    error,
    clearError,
    start,
    stop,
    resolve,
    acceptAll,
    rejectAll,
    discard,
  }), [eligibleRoles, roleId, role, setRoleId, status, progressTool,
    candidates, error, clearError, start, stop, resolve, acceptAll,
    rejectAll, discard]);
}
