import { useRef, useState } from "react";
import { rpc } from "../../api/transport/rpc.js";
import { ACCENT } from "../../domain/tools/toolDisplay.js";
import { sessionRef } from "../../domain/sessions/sessionModel.js";

export function useSessionEditing({ current, runtimeProbe, doScan, loadSnaps,
  onInplaceApplied, onSavedAs }) {
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

  const resetSelection = () => { setScope(null); setOps([]); };
  const loadCapabilities = tool => {
    const request = ++capabilityRequest.current;
    setEditCaps(null);
    setAuthoringCaps(null);
    rpc("edit_capabilities", { tool }).then(caps => {
      if (request !== capabilityRequest.current) return;
      setEditCaps(caps);
      setSaveMode(caps.save_as ? "saveas" : "inplace");
    }).catch(() => {
      if (request === capabilityRequest.current)
        setEditCaps({ operations: [], inplace: false, save_as: false });
    });
    rpc("authoring_capabilities", { tool }).then(caps => {
      if (request === capabilityRequest.current) setAuthoringCaps(caps);
    }).catch(() => {
      if (request === capabilityRequest.current)
        setAuthoringCaps({ inplace: false, save_as: false, operation_modes: {} });
    });
  };
  const addOp = (type, round) => {
    let op;
    if (type === "delete") {
      op = { type, n: round.n, label: `删除 第 ${round.n} 轮`, dot: "var(--err)",
        orig: round.user,
        summary: `整轮移除:用户与 AI 消息${round.tools.length ? ` · ${round.tools.length} 次工具调用` : ""}`,
        rpc: { op: "delete-turn", turn: round.n } };
    } else {
      op = { type, n: round.n, label: `改写 第 ${round.n} 轮`, dot: ACCENT,
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
    if (!op.items?.length) return "AI 回复至少需要一个内容块";
    for (const item of op.items) {
      if (item.kind === "text" && !item.text) return "文本内容不能为空";
      if (item.kind === "tool" && !item.name) return "工具名称不能为空";
      if (item.kind === "tool" && item.inputFormat === "json") {
        try {
          const value = JSON.parse(item.inputText);
          if (!value || Array.isArray(value) || typeof value !== "object")
            return `工具 ${item.name || "(未命名)"} 的 JSON 参数必须是对象`;
        } catch { return `工具 ${item.name || "(未命名)"} 的参数不是有效 JSON`; }
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
    setToast({ kind: "run", title: "正在应用…",
      desc: `创建快照 → 应用操作 → ${runtimeProbe ? "结构验证 + 隔离探针" : "结构验证"}` });
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
        const verdict = runtimeProbe ? "隔离探针通过" : "结构验证通过";
        const savedAs = saveMode === "saveas" && result.session_id
          ? { ...result, tool: current.tool } : null;
        setToast({ kind: "ok", title: (saveMode === "saveas" ? "已另存为新会话 · " : "已原地应用 · ") + verdict,
          desc: saveMode === "saveas" ? "原会话保持不变。" : "原会话已更新，快照已保存到「快照与还原」。",
          action: savedAs ? { label: "打开新会话", onClick: () => onSavedAs(savedAs) } : undefined });
        setOps([]); doScan(); loadSnaps();
        if (saveMode === "inplace") onInplaceApplied();
      } else setToast({ kind: "fail", title: "验收未通过 · 已自动还原",
        desc: result.error || "应用后验收未通过,已自动还原,未保留改动。" });
    } catch (error) { setToast({ kind: "fail", title: "应用失败", desc: error.message }); }
    setApplying(false);
  };

  return { ops, setOps, saveMode, setSaveMode, diff, setDiff,
    confirmApply, setConfirmApply, toast, setToast, applying, scope, setScope,
    editCaps, authoringCaps, resetSelection, loadCapabilities, addOp, startReplyEdit,
    removeOp, updateOp, authoringError, openDiff, applyEdit };
}
