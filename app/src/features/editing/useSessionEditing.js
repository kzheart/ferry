import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { rpc } from "../../api/transport/rpc.js";
import { ACCENT } from "../../domain/tools/toolDisplay.js";
import { sessionRef } from "../../domain/sessions/sessionModel.js";

export function useSessionEditing({ current, runtimeProbe, doScan,
  onInplaceApplied, onSavedAs }) {
  const { t } = useTranslation();
  const [ops, setOps] = useState([]);
  const [saveMode, setSaveMode] = useState("saveas");
  const [diff, setDiff] = useState(null);
  const [confirmApply, setConfirmApply] = useState(false);
  const [toast, setToast] = useState(null);
  const [applying, setApplying] = useState(false);
  const [scope, setScope] = useState(null);
  const [editCaps, setEditCaps] = useState(null);
  const [authoringCaps, setAuthoringCaps] = useState(null);
  const capabilityRequest = useRef(0);
  const capsCache = useRef({});   // tool -> {edit, authoring},能力按工具固定,切会话不重复请求

  const resetSelection = () => { setScope(null); setOps([]); };
  const loadCapabilities = tool => {
    const request = ++capabilityRequest.current;
    const cached = capsCache.current[tool];
    if (cached?.edit && cached?.authoring) {
      setEditCaps(cached.edit);
      setSaveMode(cached.edit.save_as ? "saveas" : "inplace");
      setAuthoringCaps(cached.authoring);
      return;
    }
    setEditCaps(null);
    setAuthoringCaps(null);
    rpc("edit_capabilities", { tool }).then(caps => {
      (capsCache.current[tool] ||= {}).edit = caps;
      if (request !== capabilityRequest.current) return;
      setEditCaps(caps);
      setSaveMode(caps.save_as ? "saveas" : "inplace");
    }).catch(() => {
      if (request === capabilityRequest.current)
        setEditCaps({ operations: [], inplace: false, save_as: false });
    });
    rpc("authoring_capabilities", { tool }).then(caps => {
      (capsCache.current[tool] ||= {}).authoring = caps;
      if (request === capabilityRequest.current) setAuthoringCaps(caps);
    }).catch(() => {
      if (request === capabilityRequest.current)
        setAuthoringCaps({ inplace: false, save_as: false, operation_modes: {} });
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
    const allowed = editCaps?.operation_modes?.[backendOp] || [];
    if (allowed.length && !allowed.includes(saveMode)) setSaveMode(allowed[0]);
    setOps(currentOps => {
      if (currentOps.some(item => item.type === type && item.n === round.n) ||
          currentOps.some(item => item.type === "assistant-reply")) return currentOps;
      return [...currentOps, { id: `${type}-${round.n}-${Date.now()}`,
        backendOp, modes: allowed, ...op }];
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
    const allowed = authoringCaps?.operation_modes?.["replace-assistant-reply"] || [];
    if (allowed.length && !allowed.includes(saveMode)) setSaveMode(allowed[0]);
    const source = turn.assistant_reply?.items || [];
    const items = source.length ? source.map(draftItem) : [draftItem({ kind: "text", text: "" })];
    setOps([{ id: `assistant-reply-${turn.turn}-${Date.now()}`, type: "assistant-reply",
      backendOp: "replace-assistant-reply", modes: allowed, n: turn.turn,
      turn: turn.turn_locator || turn.turn,
      labelKey: "browser:pendingBar.labelAuthor", labelParams: { n: turn.turn },
      label: `编排 第 ${turn.turn} 轮 AI 回复`, dot: ACCENT,
      origItems: source, items }]);
  };
  const removeOp = id => setOps(currentOps => currentOps.filter(op => op.id !== id));
  const updateOp = (id, patch) => {
    setDiff(null);
    setOps(currentOps => currentOps.map(op => op.id === id ? { ...op, ...patch } : op));
  };
  const rpcOps = () => ops.map(op => op.type === "rewrite"
    ? { op: "rewrite", locator: op.locator, text: op.text } : op.rpc);
  const authoredReply = op => ({ items: op.items.map(item => item.kind === "text"
    ? { kind: "text", text: item.text }
    : { kind: "tool", name: item.name,
        input: item.inputFormat === "json" ? JSON.parse(item.inputText) : item.inputText,
        output: item.output }) });
  const authoringError = op => {
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
  const openDiff = async () => {
    setDiff({ loading: true, preview: null });
    if (!current || !ops.length) { setDiff({ loading: false, preview: null }); return; }
    try {
      const authored = ops.find(op => op.type === "assistant-reply");
      if (authored) {
        const invalid = authoringError(authored);
        if (invalid) throw new Error(invalid);
        const preview = await rpc("authoring_preview", { tool: current.tool,
          ref: sessionRef(current), turn: authored.turn, reply: authoredReply(authored) });
        setDiff(value => value && { ...value, loading: false, preview });
      } else {
        const preview = await rpc("edit_preview", { tool: current.tool, ref: sessionRef(current), ops: rpcOps() });
        setDiff(value => value && { ...value, loading: false, preview });
      }
    } catch (error) {
      setDiff(value => value && { ...value, loading: false, preview: null, error: error.message });
    }
  };
  const applyEdit = async () => {
    if (!ops.length) return;
    setConfirmApply(false); setApplying(true);
    setToast({ kind: "run", title: t("browser:edit.toastApplying"),
      desc: runtimeProbe ? t("browser:edit.toastApplyingDescProbe") : t("browser:edit.toastApplyingDescStructure") });
    try {
      const authored = ops.find(op => op.type === "assistant-reply");
      const invalid = authored ? authoringError(authored) : null;
      if (invalid) throw new Error(invalid);
      const reply = authored ? authoredReply(authored) : null;
      const latest = authored ? await rpc("authoring_preview", { tool: current.tool,
        ref: sessionRef(current), turn: authored.turn, reply }) : null;
      const result = authored
        ? await rpc("authoring_apply", { tool: current.tool, ref: sessionRef(current),
            turn: authored.turn, reply, revision: latest.revision,
            probe: runtimeProbe, save_as: saveMode === "saveas" })
        : await rpc("edit_apply", { tool: current.tool, ref: sessionRef(current), ops: rpcOps(),
            probe: runtimeProbe, save_as: saveMode === "saveas" });
      if (result.ok) {
        const verdict = runtimeProbe ? t("browser:edit.verdictProbe") : t("browser:edit.verdictStructure");
        const savedAs = saveMode === "saveas" && result.session_id
          ? { ...result, tool: current.tool } : null;
        setToast({ kind: "ok",
          title: (saveMode === "saveas" ? t("browser:edit.toastSavedAs", { verdict }) : t("browser:edit.toastInplace", { verdict })),
          desc: saveMode === "saveas" ? t("browser:edit.toastSavedAsDesc") : t("browser:edit.toastInplaceDesc"),
          action: savedAs ? { label: t("browser:edit.toastOpenNew"), onClick: () => onSavedAs(savedAs) } : undefined });
        setOps([]); doScan();
        if (saveMode === "inplace") onInplaceApplied();
      } else setToast({ kind: "fail", title: t("browser:edit.toastVerifyFail"),
        desc: result.error || t("browser:edit.toastVerifyFailDesc") });
    } catch (error) { setToast({ kind: "fail", title: t("browser:edit.toastApplyFail"), desc: error.message }); }
    setApplying(false);
  };

  return { ops, setOps, saveMode, setSaveMode, diff, setDiff,
    confirmApply, setConfirmApply, toast, setToast, applying, scope, setScope,
    editCaps, authoringCaps, resetSelection, loadCapabilities, addOp, startReplyEdit,
    removeOp, updateOp, authoringError, openDiff, applyEdit };
}
