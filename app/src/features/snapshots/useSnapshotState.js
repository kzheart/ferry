import { useMemo, useState } from "react";

import { renderSnapshotReason } from "../../api/contract/events.js";
import { rpc } from "../../api/transport/rpc.js";

export function useSnapshotState({ snapRows, sessionsById, runtimeProbe, loadSnaps, doScan, setToast }) {
  const [confirm, setConfirm] = useState(null);
  const [restoring, setRestoring] = useState({});
  const [results, setResults] = useState({});

  const items = useMemo(() => snapRows.map(snapshot => {
    const id = (snapshot.path || "").split("/").pop()?.replace(/\.jsonl$/, "") ||
      `${snapshot.session}-${snapshot.time}`;
    const meta = sessionsById[snapshot.session];
    return { ...snapshot, id, title: meta?.title || snapshot.session,
      tool: snapshot.tool || meta?.tool || "claude",
      trigger: renderSnapshotReason(snapshot) };
  }), [snapRows, sessionsById]);

  const confirmRestore = async () => {
    const snapshot = confirm;
    setConfirm(null);
    if (!snapshot) return;
    setRestoring(value => ({ ...value, [snapshot.id]: true }));
    try {
      const result = await rpc("snapshot_restore", {
        session: snapshot.source || snapshot.session,
        tool: snapshot.tool,
        probe: !!runtimeProbe,
      });
      setResults(value => ({ ...value, [snapshot.id]: result }));
      setRestoring(value => ({ ...value, [snapshot.id]: result.ok ? "done" : false }));
      setToast(result.ok
        ? { kind: "ok", title: "已还原到快照", desc: "还原前状态已另存为保护快照。" }
        : { kind: "fail", title: "还原未生效", desc: result.error || "验收未通过,已保持当前状态" });
      loadSnaps();
      doScan();
    } catch (error) {
      setRestoring(value => ({ ...value, [snapshot.id]: false }));
      setToast({ kind: "fail", title: "还原失败", desc: error.message });
    }
  };

  return { items, confirm, setConfirm, restoring, results, confirmRestore };
}
