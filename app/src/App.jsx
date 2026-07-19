import { useEffect, useRef, useState } from "react";
import { rpc } from "./api.js";
import TrustPanel from "./components/TrustPanel.jsx";
import Library from "./pages/Library.jsx";
import Detail from "./pages/Detail.jsx";
import Migrate from "./pages/Migrate.jsx";
import Edit from "./pages/Edit.jsx";
import History from "./pages/History.jsx";
import Snapshots from "./pages/Snapshots.jsx";

export default function App() {
  const [view, setView] = useState("library");
  const [env, setEnv] = useState(null);
  const [scan, setScan] = useState(null);
  const [scanning, setScanning] = useState(false);
  const [detail, setDetail] = useState(null);
  const [mig, setMig] = useState(null);
  const [edit, setEdit] = useState(null);
  const scanned = useRef(false);

  const doScan = async () => {
    setScanning(true);
    try { setScan(await rpc("scan")); }
    catch (e) { setScan({ tools: {}, sessions: [], error: e.message }); }
    setScanning(false);
  };

  useEffect(() => {
    rpc("env").then(setEnv).catch(() => {});
    if (!scanned.current) { scanned.current = true; doScan(); }
  }, []);

  const openDetail = async s => {
    const ref = s.tool === "opencode" ? s.id : (s.path || s.id);
    setDetail({ meta: s, ref, data: null });
    setView("detail");
    try {
      const data = await rpc("show", { tool: s.tool, ref });
      setDetail(d => d && d.meta.id === s.id ? { ...d, data } : d);
    } catch (e) {
      setDetail(d => d && d.meta.id === s.id ? { ...d, error: e.message } : d);
    }
  };

  const nav = [["library", "会话库"], ["history", "迁移历史"], ["snapshots", "快照与还原"]];
  const navOn = v => v === view ||
    (v === "library" && ["detail", "migrate", "edit"].includes(view));

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="brand"><span className="logo">F</span>Ferry</div>
        {nav.map(([v, label]) => (
          <a className={`nav ${navOn(v) ? "on" : ""}`} key={v}
            onClick={() => setView(v)}>{label}</a>
        ))}
        <div className="spacer" />
        <TrustPanel env={env} />
      </aside>
      <main>
        {view === "library" &&
          <Library scan={scan} scanning={scanning} env={env}
            onScan={doScan} onOpen={openDetail} />}
        {view === "detail" && detail &&
          <Detail detail={detail} env={env}
            onBack={() => setView("library")}
            onMigrate={() => {
              setMig({ meta: detail.meta, ref: detail.ref, stage: "pick" });
              setView("migrate");
            }}
            onEdit={() => {
              setEdit({ meta: detail.meta, ref: detail.ref, data: detail.data,
                delTurns: new Set(), truncate: false, threshold: 4096,
                rewrite: null, preview: null, applying: false, result: null });
              setView("edit");
            }} />}
        {view === "migrate" && mig &&
          <Migrate mig={mig} setMig={setMig} env={env}
            onBack={() => setView("library")}
            onBackDetail={() => setView("detail")}
            gotoHistory={() => setView("history")} />}
        {view === "edit" && edit &&
          <Edit edit={edit} setEdit={setEdit}
            onBack={() => { setView("library"); doScan(); }}
            onBackDetail={() => setView("detail")}
            gotoSnapshots={() => setView("snapshots")}
            afterApply={() => {}} />}
        {view === "history" && <History />}
        {view === "snapshots" && <Snapshots />}
      </main>
    </div>
  );
}
