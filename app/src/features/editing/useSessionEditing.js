import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { operationApply, operationPlan, rpc } from "../../api/transport/rpc.js";
import { ACCENT } from "../../domain/tools/toolDisplay.js";
import { sessionRef } from "../../domain/sessions/sessionModel.js";

export function useSessionEditing({ current, runtimeProbe, doScan,
  onInplaceApplied }) {
  const { t } = useTranslation();
  const [ops, setOps] = useState([]);
  const [diff, setDiff] = useState(null);
  const [confirmApply, setConfirmApply] = useState(false);
  const [toast, setToast] = useState(null);
  const [applying, setApplying] = useState(false);
  const [scope, setScope] = useState(null);
  const [editCaps, setEditCaps] = useState(null);
  const [plannedEdit, setPlannedEdit] = useState(null);
  const capabilityRequest = useRef(0);
  const capsCache = useRef({});   // tool -> edit capabilities

  const invalidateEditPlan = () => {
    setPlannedEdit(null);
    setDiff(null);
  };
  const replaceOps = value => {
    invalidateEditPlan();
    setOps(value);
  };
  const resetSelection = () => {
    setScope(null);
    replaceOps([]);
  };
  const loadCapabilities = tool => {
    const request = ++capabilityRequest.current;
    const cached = capsCache.current[tool];
    if (cached) {
      setEditCaps(cached);
      return;
    }
    setEditCaps(null);
    rpc("edit_capabilities", { tool }).then(caps => {
      capsCache.current[tool] = caps;
      if (request !== capabilityRequest.current) return;
      setEditCaps(caps);
    }).catch(() => {
      if (request === capabilityRequest.current)
        setEditCaps({ operations: [], inplace: false, operation_modes: {} });
    });
  };
  const addOp = (type, round) => {
    let op;
    if (type === "delete") {
      op = { type, n: round.n,
        labelKey: "browser:pendingBar.labelDelete", labelParams: { n: round.n },
        label: `删除 第 ${round.n} 轮`, dot: "var(--err)",
        orig: round.user,
        summary: round.tools.length
          ? t("browser:edit.summaryDeleteWithTools", { n: round.tools.length })
          : t("browser:edit.summaryDelete"),
        rpc: { op: "delete-turn", turn: round.n } };
    } else {
      op = { type, n: round.n,
        labelKey: "browser:pendingBar.labelRewrite", labelParams: { n: round.n },
        label: `改写 第 ${round.n} 轮`, dot: ACCENT,
        orig: round.user, text: round.user, locator: round.locator };
    }
    const backendOp = type === "delete" ? "delete-turn" : "rewrite";
    invalidateEditPlan();
    setOps(currentOps => {
      if (currentOps.some(item => item.type === type && item.n === round.n) ||
          currentOps.some(item => item.type === "assistant-reply")) return currentOps;
      return [...currentOps, { id: `${type}-${round.n}-${Date.now()}`,
        backendOp, ...op }];
    });
  };
  const draftItem = item => ({ ...item,
    id: globalThis.crypto?.randomUUID?.() || `item-${Date.now()}-${Math.random()}`,
    ...(item.kind === "tool" ? {
      inputText: typeof item.input === "object"
        ? JSON.stringify(item.input, null, 2) : String(item.input ?? ""),
      inputFormat: typeof item.input === "object" ? "json" : "text",
    } : {}) });
  const startReplyEdit = turn => {
    if (!turn || ops.length) return;
    const allowed = editCaps?.operation_modes?.["replace-assistant-reply"] || [];
    if (!allowed.includes("inplace")) return;
    invalidateEditPlan();
    const source = turn.assistant_reply?.items || [];
    const items = source.length ? source.map(draftItem) : [draftItem({ kind: "text", text: "" })];
    setOps([{ id: `assistant-reply-${turn.turn}-${Date.now()}`, type: "assistant-reply",
      backendOp: "replace-assistant-reply", n: turn.turn,
      turn: turn.turn_locator || turn.turn,
      labelKey: "browser:pendingBar.labelAuthor", labelParams: { n: turn.turn },
      label: `编排 第 ${turn.turn} 轮 AI 回复`, dot: ACCENT,
      origItems: source, items, baseKey: replyKey(items) }]);
  };
  // op 只是「进入编辑」的载体,只有内容真正偏离原始才算待应用(否则底部保存条不该出现)
  const replyKey = items => JSON.stringify((items || []).map(item => item.kind === "tool"
    ? { k: "tool", name: item.name, inputText: item.inputText, inputFormat: item.inputFormat, output: item.output }
    : { k: "text", text: item.text }));
  const isDirty = op => {
    if (op.type === "rewrite") return op.text !== op.orig;
    if (op.type === "assistant-reply") return replyKey(op.items) !== op.baseKey;
    return true;   // delete 本身即改动
  };
  const dirtyOps = ops.filter(isDirty);

  const removeOp = id => {
    invalidateEditPlan();
    setOps(currentOps => currentOps.filter(op => op.id !== id));
  };
  const updateOp = (id, patch) => {
    invalidateEditPlan();
    setOps(currentOps => currentOps.map(op => op.id === id ? { ...op, ...patch } : op));
  };
  const replacementReply = op => ({ items: op.items.map(item => item.kind === "text"
    ? { kind: "text", text: item.text }
    : { kind: "tool", name: item.name,
        input: item.inputFormat === "json" ? JSON.parse(item.inputText) : item.inputText,
        output: item.output }) });
  const rpcOps = () => dirtyOps.map(op => {
    if (op.type === "rewrite")
      return { op: "rewrite", locator: op.locator, text: op.text };
    if (op.type === "assistant-reply")
      return { op: "replace-assistant-reply", turn: op.turn, reply: replacementReply(op) };
    return op.rpc;
  });
  const replyEditError = op => {
    if (!op) return null;
    if (!op.items?.length) return t("browser:edit.errNoItems");
    for (const item of op.items) {
      if (item.kind === "text" && !item.text) return t("browser:edit.errEmptyText");
      if (item.kind === "tool" && !item.name) return t("browser:edit.errNoToolName");
      if (item.kind === "tool" && item.inputFormat === "json") {
        try {
          const value = JSON.parse(item.inputText);
          if (!value || Array.isArray(value) || typeof value !== "object")
            return t("browser:edit.errToolJsonNotObject", { name: item.name || t("browser:edit.errToolUnnamed") });
        } catch { return t("browser:edit.errToolJsonInvalid", { name: item.name || t("browser:edit.errToolUnnamed") }); }
      }
    }
    return null;
  };
  const editPlanInput = () => ({
    kind: "edit",
    tool: current.tool,
    ref: sessionRef(current),
    ops: rpcOps(),
    probe: !!runtimeProbe,
  });
  const editPlanKey = input => JSON.stringify(input);
  const ensureEditPlan = async () => {
    const input = editPlanInput();
    const key = editPlanKey(input);
    if (plannedEdit?.key === key) return plannedEdit.plan;
    const plan = await operationPlan(input);
    setPlannedEdit({ key, plan });
    return plan;
  };
  const openDiff = async () => {
    setDiff({ loading: true, preview: null });
    if (!current || !dirtyOps.length) { setDiff({ loading: false, preview: null }); return; }
    try {
      const replyEdit = dirtyOps.find(op => op.type === "assistant-reply");
      const invalid = replyEdit ? replyEditError(replyEdit) : null;
      if (invalid) throw new Error(invalid);
      const plan = await ensureEditPlan();
      setDiff(value => value && { ...value, loading: false, preview: plan.preview });
    } catch (error) {
      setDiff(value => value && { ...value, loading: false, preview: null, error: error.message });
    }
  };
  const prepareApply = async () => {
    if (!dirtyOps.length) return;
    const replyEdit = dirtyOps.find(op => op.type === "assistant-reply");
    setApplying(true);
    try {
      const invalid = replyEdit ? replyEditError(replyEdit) : null;
      if (invalid) throw new Error(invalid);
      await ensureEditPlan();
      setConfirmApply(true);
    } catch (error) {
      setToast({
        kind: "fail",
        title: t("browser:edit.toastApplyFail"),
        desc: error.message,
      });
    }
    setApplying(false);
  };
  const applyEdit = async () => {
    if (!dirtyOps.length) return;
    setConfirmApply(false); setApplying(true);
    setToast({ kind: "run", title: t("browser:edit.toastApplying"),
      desc: runtimeProbe ? t("browser:edit.toastApplyingDescProbe") : t("browser:edit.toastApplyingDescStructure") });
    try {
      const replyEdit = dirtyOps.find(op => op.type === "assistant-reply");
      const invalid = replyEdit ? replyEditError(replyEdit) : null;
      if (invalid) throw new Error(invalid);
      const result = (await operationApply((await ensureEditPlan()).plan_id)).result;
      if (result.ok) {
        const verdict = runtimeProbe ? t("browser:edit.verdictProbe") : t("browser:edit.verdictStructure");
        setToast({ kind: "ok",
          title: t("browser:edit.toastInplace", { verdict }),
          desc: t("browser:edit.toastInplaceDesc") });
        setOps([]);
        setPlannedEdit(null);
        doScan();
        onInplaceApplied();
      } else setToast({ kind: "fail", title: t("browser:edit.toastVerifyFail"),
        desc: result.error || t("browser:edit.toastVerifyFailDesc") });
    } catch (error) { setToast({ kind: "fail", title: t("browser:edit.toastApplyFail"), desc: error.message }); }
    setApplying(false);
  };

  return { ops, dirtyOps, setOps: replaceOps, diff, setDiff,
    confirmApply, setConfirmApply, toast, setToast, applying, scope, setScope,
    editCaps, resetSelection, loadCapabilities, addOp, startReplyEdit,
    removeOp, updateOp, replyEditError, openDiff, prepareApply, applyEdit };
}
