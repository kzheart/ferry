// Agent 事件 → 聊天时间线的纯归约:实时事件与 events.replay 共用同一入口,
// 带 seq 的事件按序去重,重载后回放能得到一致的消息与工具状态
import { entitiesFromToolResult } from "./ferryEntities.js";

export const emptyLog = () => ({
  items: [],
  latestSeq: 0,
  status: "idle",
  runId: null,
  provider: null,
  model: null,
});

// 工具权限等级:审批卡与工具行徽章按这里着色
export const TOOL_LEVEL = {
  session_search: "read",
  session_read: "read",
  usage: "read",
  migrate: "mutate",
  session_edit: "mutate",
  session_cleanup: "mutate",
  ask_user: "read",
  bash: "mutate",
  agent_prompt: "mutate",
};

const sealAssistant = items => {
  const last = items[items.length - 1];
  if (last?.kind === "assistant" && last.streaming) {
    items[items.length - 1] = { ...last, streaming: false };
  }
};

export const operationKey = operation => operation?.plan_id || null;

// run 终态后没有任何提问还能被回答:宿主的挂起表已经清空,再点提交只会换来
// request_not_found。一个会话同一时刻只有一个活跃 run,所以此刻仍是 pending 的卡
// 一律是孤儿——包括终态事件之后才补到的那张(Rust 在 run 终态会直接返回
// answered:false 且不发 choice.requested)。不按 runId 过滤,才兜得住这些孤儿。
const endRun = (log, items) => {
  sealAssistant(items);
  for (let i = 0; i < items.length; i += 1) {
    if (items[i].kind === "choice" && items[i].status === "pending") {
      items[i] = { ...items[i], status: "unanswered", answered: false };
    }
  }
  log.status = "idle";
  log.runId = null;
};

const upsertApproval = (items, next, pushIfMissing = true) => {
  const key = operationKey(next.operation);
  const i = key
    ? items.findLastIndex(item => item.kind === "approval"
        && operationKey(item.operation) === key)
    : -1;
  if (i >= 0) {
    // 保留先到那张卡上更完整的 operation 信息,只推进状态字段
    items[i] = { ...items[i], ...next, operation: items[i].operation };
  } else if (pushIfMissing) {
    items.push(next);
  }
};

const choiceIndex = (items, requestId, callId) => items.findLastIndex(item =>
  item.kind === "choice" &&
  ((requestId && item.requestId === requestId) || (callId && item.callId === callId)));

const normalizeSelected = value => Array.isArray(value)
  ? value.filter(option => typeof option === "string")
  : [];

const choiceAnswer = value => {
  if (!value || typeof value !== "object" || Array.isArray(value)
      || typeof value.answered !== "boolean") return null;
  return {
    status: value.answered ? "answered" : "unanswered",
    answered: value.answered,
    selected: normalizeSelected(value.selected),
    customText: typeof value.custom_text === "string" ? value.custom_text : "",
  };
};

const answerFromToolResult = result => {
  const details = result?.details;
  return choiceAnswer(details?.answer || details);
};

const upsertChoice = (items, next) => {
  const i = choiceIndex(items, next.requestId, next.callId);
  if (i >= 0) {
    const current = items[i];
    items[i] = {
      ...current,
      ...next,
      // resolved/tool.completed 可能早于补发的 requested,不能被 pending 覆盖。
      ...(current.status !== "pending" && next.status === "pending"
        ? { status: current.status, answered: current.answered,
          selected: current.selected, customText: current.customText }
        : {}),
    };
  } else {
    items.push(next);
  }
};

// 三个来源(tool.started / tool.request 的 args、choice.requested 的 payload)归一成
// 同一张卡:回放与实时两条路径必须产出形状一致的 item,否则同一张卡会被建两次。
// tool.started 不带 request_id,回放时先用 tool_call_id 顶着,紧随其后的
// tool.request(进事件日志,回放拿得到)会把真正的 request_id 补上。
const choiceCard = (fields, payload, ev) => ({
  kind: "choice",
  requestId: payload.request_id || payload.tool_call_id,
  callId: payload.tool_call_id,
  question: typeof fields.question === "string" ? fields.question : "",
  options: Array.isArray(fields.options) ? fields.options : [],
  multiSelect: !!fields.multi_select,
  allowCustom: !!fields.allow_custom,
  status: "pending",
  answered: false,
  selected: [],
  customText: "",
  runId: ev.run_id,
  requestedAt: ev.timestamp,
});

const choiceFromToolArgs = (payload, ev) => choiceCard(
  payload.args && typeof payload.args === "object" ? payload.args : {},
  payload, ev);

const applyChoiceAnswer = (items, payload, answer, ev) => {
  const next = choiceAnswer(answer);
  if (!next) return;
  const i = choiceIndex(items, payload.request_id, payload.tool_call_id);
  // 答案早于卡片到达时不能丢:先落一张只有答案的卡,后到的 requested/tool.request
  // 会把问题和选项补齐(upsertChoice 不让 pending 覆盖已有答案)
  if (i < 0) items.push({ ...choiceCard({}, payload, ev), ...next });
  else items[i] = { ...items[i], ...next };
};

export function applyEvent(log, ev) {
  if (typeof ev.seq === "number") {
    if (ev.seq <= log.latestSeq) return log;
    log = { ...log, latestSeq: ev.seq };
  } else {
    log = { ...log };
  }
  const items = (log.items = [...log.items]);
  const p = ev.payload || {};
  switch (ev.type) {
    case "session.created":
    case "session.model_changed":
      log.provider = p.provider_id;
      log.model = p.model_id;
      break;
    // seq 记在用户消息上:编辑重发时靠它告诉 runtime 从哪一条截断
    case "run.started":
      items.push({ kind: "user", text: p.prompt ?? "",
        imageCount: p.image_count || 0, seq: ev.seq });
      log.status = "running";
      log.runId = ev.run_id;
      break;
    case "user.message":
      items.push({ kind: "user", text: p.text ?? "", sub: p.kind, seq: ev.seq });
      break;
    case "content.delta": {
      const last = items[items.length - 1];
      if (last?.kind === "assistant" && last.streaming) {
        items[items.length - 1] = { ...last, text: last.text + (p.delta || "") };
      } else {
        items.push({ kind: "assistant", text: p.delta || "", streaming: true, runId: ev.run_id });
      }
      break;
    }
    case "tool.started":
      sealAssistant(items);
      items.push({ kind: "tool", callId: p.tool_call_id, name: p.name, args: p.args,
        status: "running", startedAt: ev.timestamp });
      if (p.name === "ask_user") upsertChoice(items, choiceFromToolArgs(p, ev));
      break;
    // 回放里唯一带 request_id 的一条:host 的 choice.requested 不进事件日志,
    // 重载后全靠它把应答用的 request_id 补回卡片上
    case "tool.request":
      if (p.name === "ask_user") upsertChoice(items, choiceFromToolArgs(p, ev));
      break;
    case "tool.completed": {
      const i = items.findLastIndex(it => it.kind === "tool" && it.callId === p.tool_call_id);
      if (i >= 0) {
        const current = items[i];
        items[i] = { ...current, status: p.is_error ? "error" : "ok",
          endedAt: ev.timestamp, result: p.result,
          entities: p.is_error ? [] : entitiesFromToolResult(current.name, p.result, current.args) };
        if (current.name === "ask_user") {
          applyChoiceAnswer(items, p,
            p.is_error ? { answered: false, selected: [] }
              : answerFromToolResult(p.result), ev);
        }
        const envelope = p.result?.details;
        const operation = envelope?.operation;
        const key = operationKey(operation);
        // 自动执行(信封已是 applied)不出卡:工具行本身就是执行痕迹,只有待审批才值得打断
        if (current.name !== "agent_prompt" && !p.is_error && key
            && envelope.status !== "applied" &&
            !items.some(item => item.kind === "approval" &&
              operationKey(item.operation) === key)) {
          items.push({
            kind: "approval", tool: current.name, operation,
            runId: ev.run_id, status: "pending",
          });
        }
      }
      break;
    }
    case "choice.requested":
      upsertChoice(items, choiceCard(p, p, ev));
      break;
    case "choice.resolved":
      applyChoiceAnswer(items, p, p, ev);
      break;
    // Rust 可信边界补发,无 seq,不进事件日志;审批状态由前端本地推进。
    // 按 key 去重原地更新;自动执行不出卡(无既有卡时不新增),失败始终可见
    case "operation.proposed":
      upsertApproval(items, { kind: "approval", tool: p.tool,
        operation: p.operation || {}, runId: ev.run_id, status: "pending" });
      break;
    case "operation.applied":
      upsertApproval(items, { kind: "approval", tool: p.tool,
        operation: p.operation || {}, runId: ev.run_id, status: "applied",
        result: p.result, auto: !!p.auto }, !p.auto);
      break;
    case "operation.failed":
      upsertApproval(items, { kind: "approval", tool: p.tool,
        operation: p.operation || {}, runId: ev.run_id, status: "failed",
        error: p.error, auto: !!p.auto });
      break;
    case "run.completed":
      endRun(log, items);
      break;
    case "run.failed":
      endRun(log, items);
      items.push({ kind: "status", type: ev.type, message: p.message });
      break;
    case "run.cancelled":
    case "run.interrupted":
      endRun(log, items);
      items.push({ kind: "status", type: ev.type });
      break;
  }
  return log;
}

export function patchApproval(log, operationId, patch) {
  const i = log.items.findLastIndex(
    it => it.kind === "approval" && operationKey(it.operation) === operationId);
  if (i < 0) return log;
  const items = [...log.items];
  items[i] = { ...items[i], ...patch };
  return { ...log, items };
}

export function patchChoice(log, requestId, patch) {
  const i = choiceIndex(log.items, requestId, null);
  if (i < 0) return log;
  const items = [...log.items];
  items[i] = { ...items[i], ...patch };
  return { ...log, items };
}

export function titleOf(log) {
  const first = log?.items.find(it => it.kind === "user" && it.text);
  return first ? first.text.split("\n")[0].slice(0, 60) : null;
}
