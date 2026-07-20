import { useState } from "react";
import { rpc } from "../../api/transport/rpc.js";
import { ACCENT } from "../../domain/tools/toolDisplay.js";
import { sessionRef } from "../../domain/sessions/sessionModel.js";

const roundBytes = round => (round.user?.length || 0) + round.ai.join("").length +
  round.tools.reduce((sum, tool) => sum + (tool.size || 0), 0);

export function useSessionEditing({ current, runtimeProbe, doScan, loadSnaps, onInplaceApplied }) {
  const [mode, setMode] = useState("view");
  const [ops, setOps] = useState([]);
  const [saveMode, setSaveMode] = useState("saveas");
  const [diff, setDiff] = useState(null);
  const [confirmInplace, setConfirmInplace] = useState(false);
  const [toast, setToast] = useState(null);
  const [applying, setApplying] = useState(false);
  const [scope, setScope] = useState(null);
  const [editCaps, setEditCaps] = useState(null);

  const resetSelection = () => { setMode("view"); setScope(null); setOps([]); };
  const loadCapabilities = tool => {
    setEditCaps(null);
    rpc("edit_capabilities", { tool }).then(caps => {
      setEditCaps(caps);
      setSaveMode(caps.save_as ? "saveas" : "inplace");
    }).catch(() => setEditCaps({ operations: [], inplace: false, save_as: false }));
  };
  const addOp = (type, round) => {
    if (ops.some(op => op.type === type && op.n === round.n)) return;
    let op;
    if (type === "delete") {
      op = { type, n: round.n, label: `删除 第 ${round.n} 轮`, dot: "var(--err)", bytes: roundBytes(round),
        before: `第 ${round.n} 轮 用户与 AI 消息、工具调用`, after: "",
        rpc: { op: "delete-turn", turn: round.n } };
    } else {
      op = { type, n: round.n, label: `改写 第 ${round.n} 轮`, dot: ACCENT, bytes: 0,
        before: "原始用户措辞", after: "改写后的等价指令(可在下方编辑)",
        text: round.user, locator: round.locator };
    }
    const backendOp = type === "delete" ? "delete-turn" : "rewrite";
    const allowed = editCaps?.operation_modes?.[backendOp] || [];
    if (allowed.length && !allowed.includes(saveMode)) setSaveMode(allowed[0]);
    setOps(currentOps => [...currentOps, { id: `${type}-${round.n}-${Date.now()}`, ...op }]);
  };
  const removeOp = id => setOps(currentOps => currentOps.filter(op => op.id !== id));
  const updateOp = (id, patch) => setOps(currentOps => currentOps.map(op => op.id === id ? { ...op, ...patch } : op));
  const rpcOps = () => ops.map(op => op.type === "rewrite"
    ? { op: "rewrite", locator: op.locator, text: op.text } : op.rpc);
  const openDiff = async () => {
    setDiff({ loading: true, preview: null });
    if (!current || !ops.length) { setDiff({ loading: false, preview: null }); return; }
    try {
      const preview = await rpc("edit_preview", { tool: current.tool, ref: sessionRef(current), ops: rpcOps() });
      setDiff(value => value && { ...value, loading: false, preview });
    } catch (error) {
      setDiff(value => value && { ...value, loading: false, preview: null, error: error.message });
    }
  };
  const applyEdit = async () => {
    if (!ops.length) return;
    if (saveMode === "inplace" && !confirmInplace) { setConfirmInplace(true); return; }
    setConfirmInplace(false); setApplying(true);
    setToast({ kind: "run", title: "正在应用…",
      desc: `创建快照 → 应用操作 → ${runtimeProbe ? "结构验证 + 隔离探针" : "结构验证"}` });
    try {
      const result = await rpc("edit_apply", { tool: current.tool, ref: sessionRef(current), ops: rpcOps(),
        probe: runtimeProbe, save_as: saveMode === "saveas" });
      if (result.ok) {
        const verdict = runtimeProbe ? "隔离探针通过" : "结构验证通过";
        setToast({ kind: "ok", title: (saveMode === "saveas" ? "已另存为新会话 · " : "已原地应用 · ") + verdict,
          desc: saveMode === "saveas" ? "原会话保持不变。" : "原会话已更新，快照已保存到「快照与还原」。" });
        setOps([]); setMode("view"); doScan(); loadSnaps();
        if (saveMode === "inplace") onInplaceApplied();
      } else setToast({ kind: "fail", title: "验收未通过 · 已自动还原",
        desc: result.error || "应用后验收未通过,已自动还原,未保留改动。" });
    } catch (error) { setToast({ kind: "fail", title: "应用失败", desc: error.message }); }
    setApplying(false);
  };

  return { mode, setMode, ops, setOps, saveMode, setSaveMode, diff, setDiff,
    confirmInplace, setConfirmInplace, toast, setToast, applying, scope, setScope,
    editCaps, resetSelection, loadCapabilities, addOp, removeOp, updateOp, openDiff, applyEdit };
}
