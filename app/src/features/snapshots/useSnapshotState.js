import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { renderSnapshotReason } from "../../api/contract/events.js";
import { rpc } from "../../api/transport/rpc.js";

export function useSnapshotState({ snapRows, sessionsById, runtimeProbe, loadSnaps, doScan, setToast }) {
  const { t } = useTranslation();
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
        ? { kind: "ok", title: t("snapshots:toast.ok"), desc: t("snapshots:toast.okDesc") }
        : { kind: "fail", title: t("snapshots:toast.fail"), desc: result.error || t("snapshots:toast.failDesc") });
      loadSnaps();
      doScan();
    } catch (error) {
      setRestoring(value => ({ ...value, [snapshot.id]: false }));
      setToast({ kind: "fail", title: t("snapshots:toast.error"), desc: error.message });
    }
  };

  return { items, confirm, setConfirm, restoring, results, confirmRestore };
}
