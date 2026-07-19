// Ferry 主壳:标题栏 / 导航轨 / 资源栏 / 详情区 + 全部弹层(按原型复刻)
import { useEffect, useMemo, useRef, useState } from "react";
import { ACCENT, BUCKETS, TOOLS, TOOL_NAME, bucketOf, fmtTime,
  histStatus, repoOf, rpc, sessionRef } from "./api.js";
import { RailGlyph, RescanIcon, SidebarIcon, Spinner } from "./icons.jsx";
import { HistoryList, LibraryList, Pane, SnapList } from "./components/Pane.jsx";
import SessionDetail from "./pages/SessionDetail.jsx";
import HistoryDetail from "./pages/HistoryDetail.jsx";
import SnapshotDetail from "./pages/SnapshotDetail.jsx";
import FirstRun from "./pages/FirstRun.jsx";
import MigrateSheet from "./overlays/MigrateSheet.jsx";
import { DataSourceSheet, DiffSheet, Guide, HistoryFilter, InplaceConfirm,
  LibraryFilter, SettingsPopover, SnapFilter, SnapRestoreConfirm, Toast } from "./overlays/Overlays.jsx";

const TRIM_THRESHOLD = 4096;
const roundBytes = r => (r.user?.length || 0) + r.ai.join("").length +
  r.tools.reduce((a, t) => a + (t.size || 0), 0);

export default function App() {
  // ----- 数据 -----
  const [env, setEnv] = useState(null);
  const [scan, setScan] = useState(null);
  const [scanning, setScanning] = useState(false);
  const [lastScan, setLastScan] = useState(null);
  const [historyRows, setHistoryRows] = useState([]);
  const [snapRows, setSnapRows] = useState([]);

  // ----- 导航与选中 -----
  const [view, setView] = useState(
    () => localStorage.getItem("ferry-first-done") ? "library" : "firstrun");
  const [selId, setSelId] = useState(null);
  const [selHist, setSelHist] = useState(null);
  const [selSnap, setSelSnap] = useState(null);
  const [detail, setDetail] = useState(null);   // {id, data, error}

  // ----- 编辑 -----
  const [mode, setMode] = useState("view");
  const [ops, setOps] = useState([]);
  const [saveMode, setSaveMode] = useState("saveas");
  const [diff, setDiff] = useState(null);       // {preview, loading}
  const [confirmInplace, setConfirmInplace] = useState(false);
  const [toast, setToast] = useState(null);
  const [applying, setApplying] = useState(false);
  const [scope, setScope] = useState(null);

  // ----- 迁移 / 快照 -----
  const [mig, setMig] = useState(null);         // {scope}
  const [snapConfirm, setSnapConfirm] = useState(null);
  const [snapRestoring, setSnapRestoring] = useState({});
  const [snapResults, setSnapResults] = useState({});

  // ----- 布局 -----
  const [collapsed, setCollapsed] = useState(false);
  const [paneW, setPaneW] = useState(328);
  const [dragging, setDragging] = useState(false);

  // ----- 搜索与筛选 -----
  const [q, setQ] = useState("");
  const [hq, setHq] = useState("");
  const [sq, setSq] = useState("");
  const [libF, setLibF] = useState({ src: [...TOOLS], time: "all", dir: null, mig: false, sub: false });
  const [histF, setHistF] = useState({ src: [...TOOLS], target: "all", status: "all", time: "all" });
  const [snapF, setSnapF] = useState({ session: "all", time: "all" });
  const [popover, setPopover] = useState(null); // 'lib'|'hist'|'snap'
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [dataSourceOpen, setDataSourceOpen] = useState(false);
  const [guideStep, setGuideStep] = useState(0);
  const [guideSeen, setGuideSeen] = useState(() => localStorage.getItem("ferry-guide-seen") === "1");
  const [collapsedGroups, setCollapsedGroups] = useState({ earlier: true });
  const visibleIds = useRef({});
  const booted = useRef(false);

  // ----- 加载 -----
  const doScan = async () => {
    if (scanning) return;
    setScanning(true);
    try { setScan(await rpc("scan")); setLastScan(Date.now()); }
    catch (e) { setScan(s => ({ tools: {}, sessions: [], error: e.message, ...(s || {}) })); }
    setScanning(false);
  };
  const loadHistory = () => rpc("history").then(setHistoryRows).catch(() => {});
  const loadSnaps = () => rpc("snapshots").then(setSnapRows).catch(() => {});

  useEffect(() => {
    if (booted.current) return;
    booted.current = true;
    rpc("env").then(setEnv).catch(() => {});
    doScan(); loadHistory(); loadSnaps();
  }, []);

  const sessions = scan?.sessions || [];
  const byId = useMemo(() => Object.fromEntries(sessions.map(s => [s.id, s])), [sessions]);
  const migratedIds = useMemo(() => new Set(historyRows.map(h => h.source_id)), [historyRows]);

  // 首次扫描完成后默认选中第一个会话
  useEffect(() => {
    if (!selId && sessions.length) select(sessions[0].id);
  }, [sessions]);

  const select = id => {
    setSelId(id); setMode("view"); setScope(null); setOps([]);
    const s = byId[id] || sessions.find(x => x.id === id);
    if (!s) return;
    setDetail({ id, data: null });
    rpc("show", { tool: s.tool, ref: sessionRef(s) })
      .then(data => setDetail(d => d?.id === id ? { ...d, data } : d))
      .catch(e => setDetail(d => d?.id === id ? { ...d, error: e.message } : d));
  };

  const cur = selId ? byId[selId] : null;

  // ----- 键盘 -----
  useEffect(() => {
    const onKey = e => {
      if ((e.metaKey || e.ctrlKey) && (e.key === "b" || e.key === "B")) {
        e.preventDefault(); setCollapsed(v => !v); return;
      }
      if (e.key === "Escape") {
        if (settingsOpen) setSettingsOpen(false);
        else if (popover) setPopover(null);
        else if (dataSourceOpen) setDataSourceOpen(false);
        else if (snapConfirm) setSnapConfirm(null);
        else if (confirmInplace) setConfirmInplace(false);
        else if (diff) setDiff(null);
        else if (mig) setMig(null);
        else if (guideStep) finishGuide();
        return;
      }
      if (document.activeElement &&
          ["INPUT", "TEXTAREA"].includes(document.activeElement.tagName)) return;
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const ids = visibleIds.current[view] || [];
        if (!ids.length) return;
        const curSel = view === "library" ? selId : view === "history" ? selHist : selSnap;
        let i = ids.indexOf(curSel);
        i = i < 0 ? 0 : Math.max(0, Math.min(ids.length - 1, i + (e.key === "ArrowDown" ? 1 : -1)));
        if (view === "library") select(ids[i]);
        else if (view === "history") setSelHist(ids[i]);
        else setSelSnap(ids[i]);
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  });

  // ----- 拖拽分栏 -----
  const startDrag = e => {
    if (collapsed) return;
    const sx = e.clientX, sw = paneW;
    const move = ev => setPaneW(Math.max(280, Math.min(420, sw + (ev.clientX - sx))));
    const up = () => {
      document.removeEventListener("mousemove", move);
      document.removeEventListener("mouseup", up);
      setDragging(false);
    };
    document.addEventListener("mousemove", move);
    document.addEventListener("mouseup", up);
    setDragging(true); e.preventDefault();
  };

  // ----- 引导 -----
  const openGuide = () => {
    setView("library"); setMode("view"); setSettingsOpen(false);
    setDataSourceOpen(false); setMig(null); setGuideStep(1);
  };
  const finishGuide = () => {
    setGuideStep(0); setGuideSeen(true);
    localStorage.setItem("ferry-guide-seen", "1");
  };

  // ----- 编辑操作 -----
  const addOp = (type, r) => {
    if (ops.some(o => o.type === type && (type === "trim" || o.n === r.n))) return;
    let op;
    if (type === "delete") {
      op = { type, n: r.n, label: `删除 第 ${r.n} 轮`, dot: "#D5544A", bytes: roundBytes(r),
        before: `第 ${r.n} 轮 用户与 AI 消息、工具调用`, after: "",
        rpc: { op: "delete-turn", turn: r.n } };
    } else if (type === "trim") {
      const bytes = r.tools.reduce((a, t) => a + Math.max(0, (t.size || 0) - TRIM_THRESHOLD), 0);
      op = { type, n: r.n, label: `裁剪超长工具输出`, dot: "#E09112", bytes,
        before: `完整工具输出(超过 ${TRIM_THRESHOLD} 字符的部分)`, after: "保留前段 + 截断标记",
        rpc: { op: "truncate", threshold: TRIM_THRESHOLD } };
    } else {
      op = { type, n: r.n, label: `改写 第 ${r.n} 轮`, dot: ACCENT, bytes: 0,
        before: "原始用户措辞", after: "改写后的等价指令(可在下方编辑)",
        text: r.user, uuid: r.uuid };
    }
    setOps(v => [...v, { id: `${type}-${r.n}-${Date.now()}`, ...op }]);
  };
  const removeOp = id => setOps(v => v.filter(o => o.id !== id));
  const updateOp = (id, patch) => setOps(v => v.map(o => o.id === id ? { ...o, ...patch } : o));
  const rpcOps = () => ops.map(o =>
    o.type === "rewrite" ? { op: "rewrite", uuid: o.uuid, text: o.text } : o.rpc);

  const detailRef = cur ? sessionRef(cur) : null;

  const openDiff = async () => {
    setDiff({ loading: true, preview: null });
    if (cur?.tool === "claude" && ops.length) {
      try {
        const preview = await rpc("edit_preview", { ref: detailRef, ops: rpcOps() });
        setDiff(d => d && { ...d, loading: false, preview });
      } catch (e) {
        setDiff(d => d && { ...d, loading: false, preview: null, error: e.message });
      }
    } else setDiff({ loading: false, preview: null });
  };

  const applyEdit = async () => {
    if (!ops.length) return;
    if (saveMode === "inplace" && !confirmInplace) { setConfirmInplace(true); return; }
    setConfirmInplace(false); setApplying(true);
    setToast({ kind: "run", title: "正在应用…", desc: "创建快照 → 应用操作 → 探针验收" });
    try {
      const r = await rpc("edit_apply", { ref: detailRef, ops: rpcOps(),
        save_as: saveMode === "saveas" });
      if (r.ok) {
        setToast({ kind: "ok",
          title: saveMode === "saveas" ? "已另存为新会话 · 探针通过" : "已原地应用 · 探针通过",
          desc: (saveMode === "saveas" ? "原会话保持不变," : "原会话已更新,") +
            "快照已保存到「快照与还原」。" });
        setOps([]); setMode("view");
        doScan(); loadSnaps();
        if (saveMode === "inplace") select(selId);
      } else {
        setToast({ kind: "fail", title: "探针失败 · 已自动还原",
          desc: r.error || "应用后探针未通过,已自动还原,未保留改动。" });
      }
    } catch (e) {
      setToast({ kind: "fail", title: "应用失败", desc: e.message });
    }
    setApplying(false);
  };

  // ----- 快照还原 -----
  const confirmRestore = async () => {
    const snap = snapConfirm;
    setSnapConfirm(null);
    if (!snap) return;
    setSnapRestoring(v => ({ ...v, [snap.id]: true }));
    try {
      const r = await rpc("snapshot_restore", { session: snap.session });
      setSnapResults(v => ({ ...v, [snap.id]: r }));
      setSnapRestoring(v => ({ ...v, [snap.id]: r.ok ? "done" : false }));
      if (!r.ok) setToast({ kind: "fail", title: "还原未生效", desc: r.error || "探针未通过,已保持当前状态" });
      else setToast({ kind: "ok", title: "已还原到快照", desc: "还原前状态已另存为保护快照。" });
      loadSnaps(); doScan();
    } catch (e) {
      setSnapRestoring(v => ({ ...v, [snap.id]: false }));
      setToast({ kind: "fail", title: "还原失败", desc: e.message });
    }
  };

  // ----- 资源栏数据:会话库 -----
  const counts = useMemo(() => {
    const c = {};
    sessions.forEach(s => { c[s.tool] = (c[s.tool] || 0) + 1; });
    return c;
  }, [sessions]);
  const dirs = useMemo(
    () => [...new Set(sessions.map(s => repoOf(s.dir)).filter(Boolean))].slice(0, 6),
    [sessions]);

  const ql = q.trim().toLowerCase();
  const timeBuckets = { all: BUCKETS.map(b => b[0]), today: ["today"],
    last7: ["today", "yesterday", "last7"],
    last30: ["today", "yesterday", "last7", "last30"] }[libF.time];
  const matchLib = s => libF.src.includes(s.tool) &&
    (!libF.dir || repoOf(s.dir) === libF.dir) &&
    (!libF.mig || migratedIds.has(s.id)) &&
    (!libF.sub || (s.tree_count || 1) > 1) &&
    (!ql || (s.title || "").toLowerCase().includes(ql) ||
      (s.dir || "").toLowerCase().includes(ql) || s.id.toLowerCase().includes(ql));

  const libGroups = useMemo(() => {
    const groups = BUCKETS.filter(([k]) => timeBuckets.includes(k)).map(([key, label]) => {
      const rows = sessions.filter(s => bucketOf(s.updated) === key && matchLib(s));
      const isCollapsed = collapsedGroups[key] ?? false;
      return { key, label, count: rows.length, expanded: !isCollapsed,
        onToggle: () => setCollapsedGroups(g => ({ ...g, [key]: !(g[key] ?? false) })),
        rows: rows.map(s => ({ id: s.id, title: s.title || "(无标题会话)", repo: repoOf(s.dir),
          dir: s.dir, active: fmtTime(s.updated), tool: s.tool, dot: "#1C9E5A",
          hasSub: (s.tree_count || 1) > 1, subLabel: `含 ${(s.tree_count || 1) - 1} 个子会话`,
          hasMig: migratedIds.has(s.id), selected: s.id === selId,
          onClick: () => select(s.id) })) };
    }).filter(g => g.rows.length > 0);
    return groups;
  }, [sessions, libF, ql, collapsedGroups, selId, migratedIds]);
  visibleIds.current.library = libGroups.filter(g => g.expanded)
    .flatMap(g => g.rows.map(r => r.id));

  const libTokens = [];
  if (libF.src.length < 3) libF.src.forEach(t => libTokens.push({ label: TOOL_NAME[t],
    onRemove: () => setLibF(v => ({ ...v, src: v.src.filter(x => x !== t).length
      ? v.src.filter(x => x !== t) : [...TOOLS] })) }));
  if (libF.time !== "all") libTokens.push({
    label: { today: "今天", last7: "最近 7 天", last30: "最近 30 天" }[libF.time],
    onRemove: () => setLibF(v => ({ ...v, time: "all" })) });
  if (libF.dir) libTokens.push({ label: `目录 ${libF.dir}`,
    onRemove: () => setLibF(v => ({ ...v, dir: null })) });
  if (libF.mig) libTokens.push({ label: "仅含迁移",
    onRemove: () => setLibF(v => ({ ...v, mig: false })) });
  if (libF.sub) libTokens.push({ label: "仅含子会话",
    onRemove: () => setLibF(v => ({ ...v, sub: false })) });

  // ----- 资源栏数据:迁移历史 -----
  const histItems = useMemo(() => historyRows.map((h, i) => ({
    ...h, _id: `h${i}-${h.time}`, status: histStatus(h),
  })), [historyRows]);
  const hql = hq.trim().toLowerCase();
  const matchHist = h => histF.src.includes(h.src) &&
    (histF.target === "all" || h.dst === histF.target) &&
    (histF.status === "all" || h.status === histF.status) &&
    (histF.time === "all" || bucketOf(h.time) === histF.time ||
      (histF.time === "earlier" && !["today", "yesterday"].includes(bucketOf(h.time)))) &&
    (!hql || (h.title || "").toLowerCase().includes(hql) ||
      (h.session_id || "").toLowerCase().includes(hql));
  const histFiltered = histItems.filter(matchHist);
  visibleIds.current.history = histFiltered.map(h => h._id);
  const histGroups = [["today", "今天"], ["yesterday", "昨天"], ["earlier", "更早"]].map(([k, label]) => ({
    label,
    rows: histFiltered.filter(h => k === "earlier"
      ? !["today", "yesterday"].includes(bucketOf(h.time)) : bucketOf(h.time) === k)
      .map(h => ({ id: h._id, title: h.title || h.source_id, short: fmtTime(h.time),
        from: TOOL_NAME[h.src], to: TOOL_NAME[h.dst], status: h.status,
        stColor: { "成功": "#1C9E5A", "失败": "#D5544A", "已回滚": "#6B7682", "预演": "#E09112" }[h.status],
        tool: h.src, selected: h._id === (selHist ?? histFiltered[0]?._id),
        onClick: () => setSelHist(h._id) })),
  })).filter(g => g.rows.length);
  const histSel = histItems.find(h => h._id === selHist) || histFiltered[0] || null;
  const histTokens = [];
  if (histF.target !== "all") histTokens.push({ label: `目标 ${TOOL_NAME[histF.target]}`,
    onRemove: () => setHistF(v => ({ ...v, target: "all" })) });
  if (histF.status !== "all") histTokens.push({ label: histF.status,
    onRemove: () => setHistF(v => ({ ...v, status: "all" })) });
  if (histF.time !== "all") histTokens.push({
    label: { today: "今天", yesterday: "昨天", earlier: "更早" }[histF.time],
    onRemove: () => setHistF(v => ({ ...v, time: "all" })) });

  // ----- 资源栏数据:快照 -----
  const snapItems = useMemo(() => snapRows.map(s => {
    const id = (s.path || "").split("/").pop()?.replace(/\.jsonl$/, "") || `${s.session}-${s.time}`;
    const meta = byId[s.session];
    return { ...s, id, title: meta?.title || s.session, trigger: "编辑/迁移前自动" };
  }), [snapRows, byId]);
  const sql = sq.trim().toLowerCase();
  const matchSnap = s => (snapF.session === "all" || s.title === snapF.session) &&
    (snapF.time === "all" || (snapF.time === "earlier"
      ? !["today", "yesterday"].includes(bucketOf(s.time)) : bucketOf(s.time) === snapF.time)) &&
    (!sql || s.id.toLowerCase().includes(sql) || s.title.toLowerCase().includes(sql));
  const snapFiltered = snapItems.filter(matchSnap);
  visibleIds.current.snapshots = snapFiltered.map(s => s.id);
  const snapSel = snapItems.find(s => s.id === selSnap) || snapFiltered[0] || null;
  const snapListRows = snapFiltered.map(s => {
    const rst = snapRestoring[s.id];
    const status = rst === "done" ? "已还原" : rst ? "还原中" : "可还原";
    return { id: s.id, title: s.title, short: fmtTime(s.time), trigger: s.trigger, status,
      stColor: rst && rst !== "done" ? "#E09112" : "#1C9E5A", tool: "claude",
      selected: s.id === (selSnap ?? snapFiltered[0]?.id), onClick: () => setSelSnap(s.id) };
  });
  const snapTokens = [];
  if (snapF.session !== "all") snapTokens.push({ label: `会话 ${snapF.session}`,
    onRemove: () => setSnapF(v => ({ ...v, session: "all" })) });
  if (snapF.time !== "all") snapTokens.push({
    label: { today: "今天", yesterday: "昨天", earlier: "更早" }[snapF.time],
    onRemove: () => setSnapF(v => ({ ...v, time: "all" })) });

  // ----- 资源栏骨架配置 -----
  const paneCfg = {
    library: { title: "会话", count: String(sessions.length), placeholder: "搜索会话、目录、命令…",
      query: q, onQuery: e => setQ(e.target.value), sortLabel: "最近活跃",
      filterCount: (libF.src.length < 3 ? 1 : 0) + (libF.time !== "all" ? 1 : 0) +
        (libF.dir ? 1 : 0) + (libF.mig ? 1 : 0) + (libF.sub ? 1 : 0),
      tokens: libTokens,
      footer: scan?.error ? `扫描出错:${scan.error}`
        : `正在浏览 ${sessions.length} 个会话${lastScan ? ` · 上次扫描 ${fmtTime(lastScan)}` : ""}` },
    history: { title: "迁移历史", count: String(histItems.length), placeholder: "搜索迁移记录…",
      query: hq, onQuery: e => setHq(e.target.value), sortLabel: "按时间",
      filterCount: (histF.src.length < 3 ? 1 : 0) + (histF.target !== "all" ? 1 : 0) +
        (histF.status !== "all" ? 1 : 0) + (histF.time !== "all" ? 1 : 0),
      tokens: histTokens, footer: `${histItems.length} 条迁移记录` },
    snapshots: { title: "快照与还原", count: String(snapItems.length), placeholder: "搜索快照…",
      query: sq, onQuery: e => setSq(e.target.value), sortLabel: "按时间",
      filterCount: (snapF.session !== "all" ? 1 : 0) + (snapF.time !== "all" ? 1 : 0),
      tokens: snapTokens, footer: `${snapItems.length} 个快照 · 源会话只读` },
  }[view] || null;

  const crumb = {
    library: cur ? `会话 / ${cur.title || cur.id}` : "会话",
    history: histSel ? `迁移 / ${histSel.session_id || histSel.source_id}` : "迁移历史",
    snapshots: snapSel ? `快照 / ${snapSel.id}` : "快照与还原",
    firstrun: "首次启动",
  }[view];

  const clearLibF = () => {
    setLibF({ src: [...TOOLS], time: "all", dir: null, mig: false, sub: false }); setQ("");
  };
  const railItems = [
    { k: "library", label: "会话" }, { k: "history", label: "迁移" }, { k: "snapshots", label: "快照" }];

  const firstDone = () => {
    localStorage.setItem("ferry-first-done", "1");
    setView("library"); doScan();
    if (!guideSeen) setTimeout(() => setGuideStep(1), 300);
  };

  return (
    <div data-ferry-win="1" style={{ height: "100vh", display: "flex", flexDirection: "column",
      background: "#FBFCFD", position: "relative", overflow: "hidden", fontSize: 13 }}>
      {/* 标题栏 */}
      <div className="drag-region" style={{ height: 44, flex: "none", display: "flex",
        alignItems: "center", gap: 12, padding: "0 12px 0 86px", background: "#F1F4F7",
        borderBottom: "1px solid #E1E7EC" }}>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <b style={{ fontSize: 13, fontWeight: 600 }}>Ferry</b>
          <span style={{ color: "#9AA3AD" }}>/</span>
          <span style={{ color: "#6B7682", fontSize: 12.5, whiteSpace: "nowrap", overflow: "hidden",
            textOverflow: "ellipsis", maxWidth: 420 }}>{crumb}</span>
        </div>
        <button className="hov" onClick={() => setCollapsed(v => !v)}
          title={collapsed ? "展开侧边栏 ⌘B" : "收起侧边栏 ⌘B"}
          style={{ width: 28, height: 26, display: "inline-flex", alignItems: "center",
            justifyContent: "center", background: "transparent", border: "none", borderRadius: 6,
            cursor: "pointer", color: "#6B7682", marginLeft: 2 }}>
          <SidebarIcon />
        </button>
        <div style={{ flex: 1 }} />
        {view === "library" && (
          <button onClick={doScan} style={{ height: 26, display: "flex", alignItems: "center",
            gap: 7, padding: "0 11px", background: "#fff", border: "1px solid #E1E7EC",
            borderRadius: 7, fontSize: 12.5, color: "#334155", cursor: "pointer" }}>
            {scanning ? <Spinner /> : <RescanIcon />}
            {scanning ? "扫描中…" : "重新扫描"}
          </button>
        )}
      </div>

      {/* 主体 */}
      <div style={{ flex: 1, display: "flex", minHeight: 0 }}>
        {/* 导航轨 */}
        <div style={{ width: 56, flex: "none", background: "#E8ECF0", borderRight: "1px solid #DEE4E9",
          display: "flex", flexDirection: "column", alignItems: "center", padding: "11px 0 12px",
          gap: 4, zIndex: 5 }}>
          {railItems.map(n => {
            const on = view === n.k;
            return (
              <button key={n.k} className="hov-rail" title={n.label}
                data-guide={n.k === "library" ? "rail" : undefined}
                onClick={() => { setView(n.k); setSettingsOpen(false); setPopover(null); }}
                style={{ width: 40, height: 40, border: "none", borderRadius: 9,
                  background: on ? "#E4EDFB" : "transparent", display: "flex", alignItems: "center",
                  justifyContent: "center", cursor: "pointer", transition: "background .12s ease" }}>
                <RailGlyph name={n.k} color={on ? ACCENT : "#7A8591"} />
              </button>
            );
          })}
          <div style={{ flex: 1 }} />
          <button className="hov-rail" data-guide="scan" title="数据来源"
            onClick={() => setDataSourceOpen(true)}
            style={{ width: 40, height: 40, border: "none", borderRadius: 9, background: "transparent",
              display: "flex", alignItems: "center", justifyContent: "center", cursor: "pointer",
              position: "relative" }}>
            <span style={{ position: "relative", display: "inline-flex" }}>
              <RailGlyph name="data" size={18} />
              <span style={{ position: "absolute", right: -3, bottom: -2, width: 8, height: 8,
                borderRadius: "50%", background: "#1C9E5A", boxShadow: "0 0 0 2px #E8ECF0" }} />
            </span>
          </button>
          <button className="hov-rail" title={guideSeen ? "重新查看引导" : "快速上手"} onClick={openGuide}
            style={{ width: 40, height: 40, border: "none", borderRadius: 9, background: "transparent",
              display: "flex", alignItems: "center", justifyContent: "center", cursor: "pointer" }}>
            <RailGlyph name="guide" />
          </button>
          <button className="hov-rail" title="设置" onClick={() => setSettingsOpen(v => !v)}
            style={{ width: 40, height: 40, border: "none", borderRadius: 9,
              background: settingsOpen ? "#E4EDFB" : "transparent", display: "flex",
              alignItems: "center", justifyContent: "center", cursor: "pointer" }}>
            <RailGlyph name="settings" />
          </button>
        </div>

        {/* 资源栏 */}
        {paneCfg && (
          <Pane collapsed={collapsed} width={paneW} dragging={dragging}
            title={paneCfg.title} count={paneCfg.count} placeholder={paneCfg.placeholder}
            query={paneCfg.query} onQuery={paneCfg.onQuery}
            filterCount={paneCfg.filterCount}
            filterOn={popover === { library: "lib", history: "hist", snapshots: "snap" }[view] ||
              paneCfg.filterCount > 0}
            onFilter={() => setPopover(p => {
              const key = { library: "lib", history: "hist", snapshots: "snap" }[view];
              return p === key ? null : key;
            })}
            sortLabel={paneCfg.sortLabel} footer={paneCfg.footer} tokens={paneCfg.tokens}
            listKey={view}>
            {view === "library" && (
              scanning && !sessions.length
                ? <div style={{ padding: "34px 12px", textAlign: "center", color: "#9AA3AD",
                    fontSize: 12.5, display: "flex", alignItems: "center", justifyContent: "center",
                    gap: 8 }}><Spinner /> 正在扫描本机会话…</div>
                : <LibraryList groups={libGroups}
                    empty={libGroups.length === 0} onClear={clearLibF} />)}
            {view === "history" && (
              <HistoryList groups={histGroups} empty={histFiltered.length === 0}
                onClear={() => { setHistF({ src: [...TOOLS], target: "all", status: "all", time: "all" }); setHq(""); }} />)}
            {view === "snapshots" && (
              <SnapList rows={snapListRows} empty={snapFiltered.length === 0}
                onClear={() => { setSnapF({ session: "all", time: "all" }); setSq(""); }} />)}
          </Pane>
        )}

        {/* 拖拽分隔条 */}
        {paneCfg && !collapsed && (
          <div onMouseDown={startDrag} onDoubleClick={() => setPaneW(328)}
            title="拖动调整宽度 · 双击复位"
            style={{ width: 9, flex: "none", cursor: "col-resize", position: "relative",
              background: dragging ? "rgba(11,103,245,.10)" : "transparent", zIndex: 6 }}>
            <div style={{ position: "absolute", left: 0, top: 0, bottom: 0, width: 1,
              background: dragging ? ACCENT : "#E1E7EC" }} />
          </div>
        )}

        {/* 详情区 */}
        {view === "library" && (cur ? (
          <SessionDetail key={selId} meta={cur} data={detail?.data} error={detail?.error}
            mode={mode} onEnterEdit={() => setMode("edit")}
            onExitEdit={() => { setMode("view"); setOps([]); }}
            scope={scope} setScope={setScope}
            ops={ops} addOp={addOp} removeOp={removeOp} updateOp={updateOp}
            saveMode={saveMode} setSaveMode={setSaveMode}
            onOpenDiff={openDiff} onApply={applyEdit} applying={applying}
            onOpenMigrate={sc => setMig({ scope: sc ?? scope })} />
        ) : (
          <div style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center",
            color: "#9AA3AD", fontSize: 13 }}>
            {scanning ? "正在扫描本机会话…" : "没有可显示的会话 —— 点右上角「重新扫描」"}</div>
        ))}
        {view === "history" && <HistoryDetail h={histSel} />}
        {view === "snapshots" && (
          <SnapshotDetail s={snapSel ? { ...snapSel, result: snapResults[snapSel.id] } : null}
            restoring={snapSel ? snapRestoring[snapSel.id] : false}
            onRestore={() => snapSel && setSnapConfirm(snapSel)} />)}
        {view === "firstrun" && <FirstRun env={env} scan={scan} onStart={firstDone} />}
      </div>

      {/* 弹层 */}
      {mig && cur && (
        <MigrateSheet meta={cur} scope={mig.scope} env={env}
          onClose={() => setMig(null)}
          onDone={() => { loadHistory(); loadSnaps(); }} />)}
      {diff && <DiffSheet ops={ops} preview={diff.preview} loading={diff.loading}
        onClose={() => setDiff(null)} />}
      {confirmInplace && <InplaceConfirm onCancel={() => setConfirmInplace(false)}
        onConfirm={applyEdit} />}
      {snapConfirm && <SnapRestoreConfirm snap={snapConfirm}
        onCancel={() => setSnapConfirm(null)} onConfirm={confirmRestore} />}
      {toast && <Toast toast={toast} onDismiss={() => setToast(null)} />}
      {settingsOpen && (
        <SettingsPopover onClose={() => setSettingsOpen(false)}
          onOpenGuide={() => { setSettingsOpen(false); openGuide(); }}
          onFirstRun={() => { setSettingsOpen(false); setView("firstrun"); }}
          guideSeen={guideSeen} />)}
      {dataSourceOpen && (
        <DataSourceSheet scan={scan} env={env} scanning={scanning} onRescan={doScan}
          onClose={() => setDataSourceOpen(false)}
          onOpenGuide={() => { setDataSourceOpen(false); openGuide(); }}
          onFirstRun={() => { setDataSourceOpen(false); setView("firstrun"); }}
          guideSeen={guideSeen} />)}
      {popover === "lib" && (
        <LibraryFilter f={libF} setF={setLibF} counts={counts} dirs={dirs}
          onClose={() => setPopover(null)} onClear={clearLibF} />)}
      {popover === "hist" && (
        <HistoryFilter f={histF} setF={setHistF} onClose={() => setPopover(null)}
          onClear={() => { setHistF({ src: [...TOOLS], target: "all", status: "all", time: "all" }); setHq(""); }} />)}
      {popover === "snap" && (
        <SnapFilter f={snapF} setF={setSnapF}
          sessions={[...new Set(snapItems.map(s => s.title))].slice(0, 6)}
          onClose={() => setPopover(null)}
          onClear={() => { setSnapF({ session: "all", time: "all" }); setSq(""); }} />)}
      {guideStep > 0 && (
        <Guide step={guideStep} onGo={setGuideStep} onFinish={finishGuide} />)}
    </div>
  );
}
