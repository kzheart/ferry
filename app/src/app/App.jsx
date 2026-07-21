// Ferry 主壳:标题栏 / 导航轨 / 资源栏 / 详情区 + 全部弹层(按原型复刻)
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { canReveal, openTerminal, revealPath, rpc } from "../api/transport/rpc.js";
import { TOOLS, TOOL_NAME } from "../api/contract/tools.js";
import { ACCENT } from "../domain/tools/toolDisplay.js";
import { BUCKETS, bucketOf, fmtTime, repoOf, sessionRef } from "../domain/sessions/sessionModel.js";
import { histStatus, STATUS_CODE } from "../features/migration/migrationModel.js";
import { RailGlyph, RescanIcon, SidebarIcon, Spinner } from "../components/ui/icons.jsx";
import { HistoryList, LibraryList, Pane, SnapList } from "../components/layout/ResourcePane.jsx";
import Overview from "../features/overview/Overview.jsx";
import SessionDetail from "../features/browser/SessionDetail.jsx";
import HistoryDetail from "../features/migration/HistoryDetail.jsx";
import SnapshotDetail from "../features/snapshots/SnapshotDetail.jsx";
import FirstRun from "../features/onboarding/FirstRun.jsx";
import MigrateSheet from "../features/migration/MigrateSheet.jsx";
import SettingsPage from "../features/settings/Settings.jsx";
import { BatchDeleteConfirm, ContextMenu, DiffSheet, Guide, HistoryFilter,
  ApplyConfirm, LibraryFilter, PromptBox, SessionDeleteConfirm, SnapFilter,
  SnapRestoreConfirm, Toast } from "../components/ui/Overlays.jsx";
import { useSettings } from "../features/settings/useSettings.js";
import { useAppUpdater } from "../features/settings/useAppUpdater.js";
import { useBrowserData } from "../features/browser/useBrowserData.js";
import { useSessionEditing } from "../features/editing/useSessionEditing.js";
import { useSnapshotState } from "../features/snapshots/useSnapshotState.js";

export default function App() {
  const { t } = useTranslation();
  // ----- 数据 -----
  const { env, scan, scanning, lastScan, historyRows, snapRows, pricing,
    doScan, loadHistory, loadSnaps } = useBrowserData();

  // ----- 导航与选中 -----
  const [view, setView] = useState(
    () => localStorage.getItem("ferry-first-done") ? "library" : "firstrun");
  const [selId, setSelId] = useState(null);
  const [selHist, setSelHist] = useState(null);
  const [selSnap, setSelSnap] = useState(null);
  const [detail, setDetail] = useState(null);   // {id, data, error}

  // ----- 编辑 -----
  // ----- 迁移 / 快照 -----
  const [mig, setMig] = useState(null);         // {scope}

  // ----- 布局 -----
  const [collapsed, setCollapsed] = useState(false);
  const [paneW, setPaneW] = useState(328);
  const [dragging, setDragging] = useState(false);

  // ----- 搜索与筛选 -----
  const [q, setQ] = useState("");
  const [hq, setHq] = useState("");
  const [sq, setSq] = useState("");
  const [libF, setLibF] = useState(
    { src: [...TOOLS], time: "all", dir: null, mig: false, sub: false, arch: false, tag: null });
  const [histF, setHistF] = useState({ src: [...TOOLS], target: "all", status: "all", time: "all" });
  const [snapF, setSnapF] = useState(
    { src: [...TOOLS], reason: "all", session: "all", time: "all" });
  const [popover, setPopover] = useState(null); // 'lib'|'hist'|'snap'
  const [ctxMenu, setCtxMenu] = useState(null); // {x, y, id, multi?}
  const [delConfirm, setDelConfirm] = useState(null);
  const [batchDel, setBatchDel] = useState(null);   // 待批量删除的会话数组
  const [renameFor, setRenameFor] = useState(null); // 待重命名的会话
  const [tagFor, setTagFor] = useState(null);       // {ids} 待编辑标签的会话
  const [multiSel, setMultiSel] = useState([]);     // 多选中的会话 id
  const [metaMap, setMetaMap] = useState({});       // 会话元数据 sidecar
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settings, setSettings] = useSettings();
  const updater = useAppUpdater(settings.autoCheckUpdates);
  const [railTip, setRailTip] = useState(null);  // {label, top}
  const tipTimer = useRef(null);
  const [guideStep, setGuideStep] = useState(0);
  const [guideSeen, setGuideSeen] = useState(() => localStorage.getItem("ferry-guide-seen") === "1");
  const [collapsedGroups, setCollapsedGroups] = useState({ earlier: true });
  const visibleIds = useRef({});

  const sessions = scan?.sessions || [];
  const byId = useMemo(() => Object.fromEntries(sessions.map(s => [s.id, s])), [sessions]);
  const migratedIds = useMemo(() => new Set(historyRows.map(h => h.source_id)), [historyRows]);
  const cur = selId ? byId[selId] : null;
  const savedAsRef = useRef(null);
  const editing = useSessionEditing({ current: cur,
    runtimeProbe: !!settings.runtimeProbe, doScan, loadSnaps,
    onInplaceApplied: () => select(selId),
    onSavedAs: result => savedAsRef.current?.(result) });
  const { ops, setOps, saveMode, setSaveMode, diff, setDiff,
    confirmApply, setConfirmApply, toast, setToast, applying, scope, setScope,
    editCaps, authoringCaps, resetSelection, loadCapabilities, addOp, startReplyEdit,
    removeOp, updateOp, authoringError, openDiff, applyEdit } = editing;
  const snapshots = useSnapshotState({ snapRows, sessionsById: byId,
    runtimeProbe: settings.runtimeProbe, loadSnaps, doScan, setToast });
  const { items: snapItems, confirm: snapConfirm, setConfirm: setSnapConfirm,
    restoring: snapRestoring, results: snapResults, confirmRestore } = snapshots;

  // 首次扫描完成后默认选中第一个会话
  useEffect(() => {
    if (!selId && sessions.length) select(sessions[0].id);
  }, [sessions]);

  // ----- 会话元数据(重命名/置顶/归档/标签,sidecar 存储) -----
  useEffect(() => {
    rpc("session_meta_list").then(m => setMetaMap(m || {})).catch(() => {});
  }, []);
  const setMetaFor = async (id, patch) => {
    try {
      const entry = await rpc("session_meta_set", { id, patch });
      setMetaMap(m => {
        const next = { ...m };
        if (entry && Object.keys(entry).length) next[id] = entry;
        else delete next[id];
        return next;
      });
    } catch (e) {
      setToast({ kind: "fail", title: t("app:toast.metaSaveFail"), desc: e.message });
    }
  };
  const batchMeta = async patch => {
    for (const id of multiSel) await setMetaFor(id, patch);
    setToast({ kind: "ok", title: t("app:toast.metaUpdated"), desc: t("app:toast.metaUpdatedDesc", { n: multiSel.length }) });
  };
  const manualSnapshot = async s => {
    setToast({ kind: "run", title: t("app:toast.snapshotCreating"), desc: s.title || s.id });
    try {
      await rpc("session_snapshot", { tool: s.tool, ref: sessionRef(s) });
      loadSnaps();
      setToast({ kind: "ok", title: t("app:toast.snapshotCreated"), desc: t("app:toast.snapshotCreatedDesc") });
    } catch (e) {
      setToast({ kind: "fail", title: t("app:toast.snapshotCreateFail"), desc: e.message });
    }
  };

  const select = id => {
    setSelId(id); resetSelection();
    const s = byId[id] || sessions.find(x => x.id === id);
    if (!s) return;
    setDetail({ id, data: null });
    rpc("show", { tool: s.tool, ref: sessionRef(s) })
      .then(data => setDetail(d => d?.id === id ? { ...d, data } : d))
      .catch(e => setDetail(d => d?.id === id ? { ...d, error: e.message } : d));
    loadCapabilities(s.tool);
  };

  // 另存为新会话后从 toast 打开:扫描结果已含新会话则正常选中,否则按返回的路径直接读
  savedAsRef.current = result => {
    const id = result.session_id;
    if (byId[id]) { select(id); return; }
    setSelId(id); resetSelection();
    setDetail({ id, data: null });
    rpc("show", { tool: result.tool, ref: result.saved_as || id })
      .then(data => setDetail(d => d?.id === id ? { ...d, data } : d))
      .catch(e => setDetail(d => d?.id === id ? { ...d, error: e.message } : d));
    doScan();
  };

  // ----- 会话删除(回收站语义:先快照,可撤销) -----
  const undoDelete = async snapshot => {
    setToast({ kind: "run", title: t("app:toast.restoring"), desc: t("app:toast.restoringDesc") });
    try {
      await rpc("session_undelete", { snapshot });
      doScan(); loadSnaps();
      setToast({ kind: "ok", title: t("app:toast.restoreDone"), desc: t("app:toast.restoreDoneDesc") });
    } catch (e) {
      setToast({ kind: "fail", title: t("app:toast.restoreFail"), desc: e.message });
    }
  };
  const deleteSession = async s => {
    setDelConfirm(null);
    setToast({ kind: "run", title: t("app:toast.deleting"), desc: t("app:toast.deletingDesc") });
    try {
      const r = await rpc("session_delete", { tool: s.tool, ref: sessionRef(s) });
      if (selId === s.id) { setSelId(null); setDetail(null); }
      doScan(); loadSnaps();
      setToast({ kind: "ok", title: t("app:toast.deleteDone"),
        desc: t("app:toast.deleteDoneDesc", { title: s.title || s.id }),
        action: r.undoable
          ? { label: t("app:toast.undo"), onClick: () => undoDelete(r.snapshot) } : undefined });
    } catch (e) {
      setToast({ kind: "fail", title: t("app:toast.deleteFail"), desc: e.message });
    }
  };
  const askDelete = s => {
    if (s.tool === "opencode" || (s.tree_count || 1) > 1) setDelConfirm(s);
    else deleteSession(s);
  };
  const doBatchDelete = async () => {
    const targets = batchDel;
    setBatchDel(null);
    let done = 0, fail = 0;
    for (const s of targets) {
      setToast({ kind: "run", title: t("app:toast.batchDeleting"),
        desc: t("app:toast.batchProgress", { done: done + fail, total: targets.length }) });
      try {
        await rpc("session_delete", { tool: s.tool, ref: sessionRef(s) });
        done++;
      } catch { fail++; }
    }
    if (targets.some(s => s.id === selId)) { setSelId(null); setDetail(null); }
    setMultiSel([]); doScan(); loadSnaps();
    setToast(fail
      ? { kind: "fail", title: t("app:toast.batchPartialFail"), desc: t("app:toast.batchPartialFailDesc", { done, fail }) }
      : { kind: "ok", title: t("app:toast.batchDone"),
          desc: t("app:toast.batchDoneDesc", { done }) });
  };

  const ctxSess = ctxMenu ? byId[ctxMenu.id] : null;
  const ctxMeta = ctxSess ? metaMap[ctxSess.id] || {} : {};
  const multiSess = multiSel.map(id => byId[id]).filter(Boolean);
  const ctxItems = ctxMenu?.multi ? [
    { label: t("app:ctx.batchArchive"), onClick: () => batchMeta({ archived: true }) },
    { label: t("app:ctx.batchUnarchive"), onClick: () => batchMeta({ archived: false }) },
    { label: t("app:ctx.addTags"), onClick: () => setTagFor({ ids: [...multiSel], batch: true }) },
    { sep: true },
    { label: t("app:ctx.deleteN", { n: multiSess.length }), danger: true,
      onClick: () => setBatchDel(multiSess) },
    { sep: true },
    { label: t("app:ctx.cancelMulti"), onClick: () => setMultiSel([]) },
  ] : ctxSess ? [
    { label: t("app:ctx.resumeTerminal"), hint: "↩", onClick: () => resumeDescriptor(
        ctxSess.tool, ctxSess.id, ctxSess.dir).then(openTerminal).catch(() => {}) },
    { label: t("app:ctx.migrateTo"), onClick: () => {
        if (ctxSess.id !== selId) select(ctxSess.id);
        setMig({ scope: null }); } },
    { sep: true },
    { label: t("app:ctx.rename"), hint: "F2", onClick: () => setRenameFor(ctxSess) },
    { label: ctxMeta.pinned ? t("app:ctx.unpin") : t("app:ctx.pin"),
      onClick: () => setMetaFor(ctxSess.id, { pinned: !ctxMeta.pinned }) },
    { label: ctxMeta.archived ? t("app:ctx.unarchive") : t("app:ctx.archive"),
      onClick: () => setMetaFor(ctxSess.id, { archived: !ctxMeta.archived }) },
    { label: t("app:ctx.tags"), onClick: () => setTagFor({ ids: [ctxSess.id] }) },
    { label: t("app:ctx.snapshot"), onClick: () => manualSnapshot(ctxSess) },
    { sep: true },
    { label: t("app:ctx.copyId"), onClick: () => navigator.clipboard?.writeText(ctxSess.id) },
    { label: t("app:ctx.copyResume"), onClick: () => resumeDescriptor(
        ctxSess.tool, ctxSess.id, ctxSess.dir)
        .then(d => navigator.clipboard?.writeText(d.display_command))
        .catch(() => {}) },
    { label: t("app:ctx.revealInFinder"), disabled: !ctxSess.path || !canReveal(),
      disabledHint: ctxSess.path ? t("app:ctx.onlyDesktop") : t("app:ctx.noSessionFile"),
      onClick: () => revealPath(ctxSess.path).catch(() => {}) },
    { sep: true },
    { label: t("app:ctx.deleteSession"), hint: "⌫", danger: true, onClick: () => askDelete(ctxSess) },
  ] : null;

  // ----- 键盘 -----
  useEffect(() => {
    const onKey = e => {
      if ((e.metaKey || e.ctrlKey) && (e.key === "b" || e.key === "B")) {
        e.preventDefault(); setCollapsed(v => !v); return;
      }
      if (e.key === "Escape") {
        if (ctxMenu) setCtxMenu(null);
        else if (renameFor) setRenameFor(null);
        else if (tagFor) setTagFor(null);
        else if (batchDel) setBatchDel(null);
        else if (delConfirm) setDelConfirm(null);
        else if (settingsOpen) setSettingsOpen(false);
        else if (popover) setPopover(null);
        else if (snapConfirm) setSnapConfirm(null);
        else if (confirmApply) setConfirmApply(false);
        else if (diff) setDiff(null);
        else if (mig) setMig(null);
        else if (multiSel.length) setMultiSel([]);
        else if (guideStep) finishGuide();
        return;
      }
      if (document.activeElement &&
          ["INPUT", "TEXTAREA"].includes(document.activeElement.tagName)) return;
      // 会话库快捷键:仅在没有弹层时生效
      const overlayOpen = ctxMenu || delConfirm || batchDel || renameFor || tagFor ||
        settingsOpen || popover || snapConfirm || confirmApply || diff || mig || guideStep;
      if (!overlayOpen && view === "library" && cur) {
        if (e.key === "F2") { e.preventDefault(); setRenameFor(cur); return; }
        if (e.key === "Backspace" || e.key === "Delete") {
          e.preventDefault();
          if (multiSel.length > 1) setBatchDel(multiSel.map(id => byId[id]).filter(Boolean));
          else askDelete(cur);
          return;
        }
        if (e.key === "Enter") {
          e.preventDefault();
          resumeDescriptor(cur.tool, cur.id, cur.dir)
            .then(openTerminal).catch(() => {});
          return;
        }
      }
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
    setView("library"); setSettingsOpen(false);
    setMig(null); setGuideStep(1);
  };

  // ----- 导航轨悬停提示(延迟 450ms,与原型一致) -----
  const railEnter = (label, e) => {
    const el = e.currentTarget.getBoundingClientRect();
    const root = document.querySelector("[data-ferry-win]");
    if (!root) return;
    const top = el.top - root.getBoundingClientRect().top + el.height / 2;
    clearTimeout(tipTimer.current);
    tipTimer.current = setTimeout(() => setRailTip({ label, top }), 450);
  };
  const railLeave = () => { clearTimeout(tipTimer.current); setRailTip(null); };
  useEffect(() => () => clearTimeout(tipTimer.current), []);
  const finishGuide = () => {
    setGuideStep(0); setGuideSeen(true);
    localStorage.setItem("ferry-guide-seen", "1");
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
  const timeBuckets = { all: [...BUCKETS], today: ["today"],
    last7: ["today", "yesterday", "last7"],
    last30: ["today", "yesterday", "last7", "last30"] }[libF.time];
  const matchLib = s => {
    const m = metaMap[s.id] || {};
    return libF.src.includes(s.tool) &&
      (libF.arch || !m.archived) &&
      (!libF.tag || (m.tags || []).includes(libF.tag)) &&
      (!libF.dir || repoOf(s.dir) === libF.dir) &&
      (!libF.mig || migratedIds.has(s.id)) &&
      (!libF.sub || (s.tree_count || 1) > 1) &&
      (!ql || (s.title || "").toLowerCase().includes(ql) ||
        (m.name || "").toLowerCase().includes(ql) ||
        (m.tags || []).some(t => t.toLowerCase().includes(ql)) ||
        (s.dir || "").toLowerCase().includes(ql) || s.id.toLowerCase().includes(ql));
  };

  const libGroups = useMemo(() => {
    const rowOf = s => {
      const m = metaMap[s.id] || {};
      return { id: s.id, title: m.name || s.title || t("app:library.untitled"), repo: repoOf(s.dir),
        dir: s.dir, active: fmtTime(s.updated, t), tool: s.tool, dot: "var(--ok)",
        pinned: !!m.pinned, archived: !!m.archived, tags: m.tags,
        hasSub: (s.tree_count || 1) > 1, subLabel: t("app:library.subLabel", { n: (s.tree_count || 1) - 1 }),
        hasMig: migratedIds.has(s.id), selected: s.id === selId,
        multi: multiSel.includes(s.id),
        onClick: e => {
          if (e.metaKey || e.ctrlKey) {           // ⌘点击:切换多选
            setMultiSel(sel => {
              const base = sel.length ? sel : (selId ? [selId] : []);
              return base.includes(s.id)
                ? base.filter(x => x !== s.id) : [...base, s.id];
            });
            return;
          }
          if (e.shiftKey && selId) {              // Shift 点击:按可见顺序范围选
            const ids = visibleIds.current.library || [];
            const a = ids.indexOf(selId), b = ids.indexOf(s.id);
            if (a >= 0 && b >= 0) {
              setMultiSel(ids.slice(Math.min(a, b), Math.max(a, b) + 1));
              return;
            }
          }
          setMultiSel([]); select(s.id);
        },
        onContext: e => {
          e.preventDefault();
          if (multiSel.length > 1 && multiSel.includes(s.id)) {
            setCtxMenu({ x: e.clientX, y: e.clientY, id: s.id, multi: true });
            return;
          }
          setMultiSel([]);
          if (s.id !== selId) select(s.id);
          setCtxMenu({ x: e.clientX, y: e.clientY, id: s.id });
        } };
    };
    const isPinned = s => !!(metaMap[s.id] || {}).pinned;
    const groups = [];
    const pinnedRows = sessions.filter(s => isPinned(s) && matchLib(s));
    if (pinnedRows.length) {
      groups.push({ key: "pinned", label: t("app:library.pinned"), count: pinnedRows.length,
        expanded: !(collapsedGroups.pinned ?? false),
        onToggle: () => setCollapsedGroups(g => ({ ...g, pinned: !(g.pinned ?? false) })),
        rows: pinnedRows.map(rowOf) });
    }
    BUCKETS.filter(k => timeBuckets.includes(k)).forEach(key => {
      const rows = sessions.filter(s =>
        !isPinned(s) && bucketOf(s.updated) === key && matchLib(s));
      if (!rows.length) return;
      groups.push({ key, label: t(`common:bucket.${key}`), count: rows.length,
        expanded: !(collapsedGroups[key] ?? false),
        onToggle: () => setCollapsedGroups(g => ({ ...g, [key]: !(g[key] ?? false) })),
        rows: rows.map(rowOf) });
    });
    return groups;
  }, [sessions, libF, ql, collapsedGroups, selId, migratedIds, metaMap, multiSel]);
  visibleIds.current.library = libGroups.filter(g => g.expanded)
    .flatMap(g => g.rows.map(r => r.id));

  const libTokens = [];
  if (libF.src.length < TOOLS.length) libF.src.forEach(tool => libTokens.push({ label: TOOL_NAME[tool],
    onRemove: () => setLibF(v => ({ ...v, src: v.src.filter(x => x !== tool).length
      ? v.src.filter(x => x !== tool) : [...TOOLS] })) }));
  if (libF.time !== "all") libTokens.push({
    label: t(`common:bucket.${libF.time === "last7" ? "last7" : libF.time === "last30" ? "last30" : libF.time}`),
    onRemove: () => setLibF(v => ({ ...v, time: "all" })) });
  if (libF.dir) libTokens.push({ label: t("app:library.tokenDir", { dir: libF.dir }),
    onRemove: () => setLibF(v => ({ ...v, dir: null })) });
  if (libF.mig) libTokens.push({ label: t("app:library.tokenOnlyMigrated"),
    onRemove: () => setLibF(v => ({ ...v, mig: false })) });
  if (libF.sub) libTokens.push({ label: t("app:library.tokenOnlySub"),
    onRemove: () => setLibF(v => ({ ...v, sub: false })) });
  if (libF.arch) libTokens.push({ label: t("app:library.tokenIncludeArchived"),
    onRemove: () => setLibF(v => ({ ...v, arch: false })) });
  if (libF.tag) libTokens.push({ label: t("app:library.tokenTag", { tag: libF.tag }),
    onRemove: () => setLibF(v => ({ ...v, tag: null })) });

  const allTags = useMemo(
    () => [...new Set(Object.values(metaMap).flatMap(m => m.tags || []))].slice(0, 12),
    [metaMap]);

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
  const histGroups = [["today", t("app:historyToken.today")], ["yesterday", t("app:historyToken.yesterday")], ["earlier", t("app:historyToken.earlier")]].map(([k, label]) => ({
    label,
    rows: histFiltered.filter(h => k === "earlier"
      ? !["today", "yesterday"].includes(bucketOf(h.time)) : bucketOf(h.time) === k)
      .map(h => ({ id: h._id, title: h.title || h.source_id, short: fmtTime(h.time, t),
        from: TOOL_NAME[h.src], to: TOOL_NAME[h.dst], status: h.status,
        statusLabel: t(`common:${h.status}`),
        stColor: { [STATUS_CODE.success]: "var(--ok)", [STATUS_CODE.failed]: "var(--err)",
          [STATUS_CODE.rolledBack]: "var(--tx3b)", [STATUS_CODE.dryRun]: "var(--warn)" }[h.status],
        tool: h.src, selected: h._id === (selHist ?? histFiltered[0]?._id),
        onClick: () => setSelHist(h._id) })),
  })).filter(g => g.rows.length);
  const histSel = histItems.find(h => h._id === selHist) || histFiltered[0] || null;
  const histTokens = [];
  if (histF.target !== "all") histTokens.push({ label: t("app:historyToken.target", { tool: TOOL_NAME[histF.target] }),
    onRemove: () => setHistF(v => ({ ...v, target: "all" })) });
  if (histF.status !== "all") histTokens.push({ label: t(`common:${histF.status}`),
    onRemove: () => setHistF(v => ({ ...v, status: "all" })) });
  if (histF.time !== "all") histTokens.push({
    label: t(`app:historyToken.${histF.time}`),
    onRemove: () => setHistF(v => ({ ...v, time: "all" })) });

  // ----- 资源栏数据:快照 -----
  const snapReasons = useMemo(
    () => [...new Set(snapItems.map(s => s.trigger))], [snapItems]);
  const sql = sq.trim().toLowerCase();
  const matchSnap = s => snapF.src.includes(s.tool) &&
    (snapF.reason === "all" || s.trigger === snapF.reason) &&
    (snapF.session === "all" || s.title === snapF.session) &&
    (snapF.time === "all" || (snapF.time === "earlier"
      ? !["today", "yesterday"].includes(bucketOf(s.time)) : bucketOf(s.time) === snapF.time)) &&
    (!sql || s.id.toLowerCase().includes(sql) || s.title.toLowerCase().includes(sql));
  const snapFiltered = snapItems.filter(matchSnap);
  visibleIds.current.snapshots = snapFiltered.map(s => s.id);
  const snapSel = snapItems.find(s => s.id === selSnap) || snapFiltered[0] || null;
  const snapListRows = snapFiltered.map(s => {
    const rst = snapRestoring[s.id];
    const status = rst === "done" ? t("app:snapStatus.restored")
      : rst ? t("app:snapStatus.restoring") : t("app:snapStatus.restorable");
    return { id: s.id, title: s.title, short: fmtTime(s.time, t), trigger: s.trigger, status,
      stColor: rst && rst !== "done" ? "var(--warn)" : "var(--ok)", tool: s.tool,
      selected: s.id === (selSnap ?? snapFiltered[0]?.id), onClick: () => setSelSnap(s.id) };
  });
  const snapTokens = [];
  if (snapF.reason !== "all") snapTokens.push({ label: snapF.reason,
    onRemove: () => setSnapF(v => ({ ...v, reason: "all" })) });
  if (snapF.session !== "all") snapTokens.push({ label: t("app:snapshotsToken.session", { session: snapF.session }),
    onRemove: () => setSnapF(v => ({ ...v, session: "all" })) });
  if (snapF.time !== "all") snapTokens.push({
    label: t(`app:snapshotsToken.${snapF.time}`),
    onRemove: () => setSnapF(v => ({ ...v, time: "all" })) });

  // ----- 资源栏骨架配置 -----
  const paneCfg = {
    library: { title: t("app:pane.libraryTitle"), count: String(sessions.length), placeholder: t("app:pane.libraryPlaceholder"),
      query: q, onQuery: e => setQ(e.target.value), sortLabel: t("app:pane.librarySort"),
      filterCount: (libF.src.length < 3 ? 1 : 0) + (libF.time !== "all" ? 1 : 0) +
        (libF.dir ? 1 : 0) + (libF.mig ? 1 : 0) + (libF.sub ? 1 : 0) +
        (libF.arch ? 1 : 0) + (libF.tag ? 1 : 0),
      tokens: libTokens,
      footer: scan?.error ? t("app:pane.libraryFooterError", { error: scan.error })
        : multiSel.length > 1 ? t("app:pane.libraryFooterMulti", { n: multiSel.length })
        : t("app:pane.libraryFooterBrowsing", { n: sessions.length, lastScan: lastScan ? t("app:pane.libraryFooterLastScan", { time: fmtTime(lastScan, t) }) : "" }) },
    history: { title: t("app:pane.historyTitle"), count: String(histItems.length), placeholder: t("app:pane.historyPlaceholder"),
      query: hq, onQuery: e => setHq(e.target.value), sortLabel: t("app:pane.historySort"),
      filterCount: (histF.src.length < 3 ? 1 : 0) + (histF.target !== "all" ? 1 : 0) +
        (histF.status !== "all" ? 1 : 0) + (histF.time !== "all" ? 1 : 0),
      tokens: histTokens, footer: t("app:pane.historyFooter", { n: histItems.length }) },
    snapshots: { title: t("app:pane.snapshotsTitle"), count: String(snapItems.length), placeholder: t("app:pane.snapshotsPlaceholder"),
      query: sq, onQuery: e => setSq(e.target.value), sortLabel: t("app:pane.snapshotsSort"),
      filterCount: (snapF.src.length < TOOLS.length ? 1 : 0) + (snapF.reason !== "all" ? 1 : 0) +
        (snapF.session !== "all" ? 1 : 0) + (snapF.time !== "all" ? 1 : 0),
      tokens: snapTokens, footer: t("app:pane.snapshotsFooter", { n: snapItems.length }) },
  }[view] || null;

  const clearLibF = () => {
    setLibF({ src: [...TOOLS], time: "all", dir: null, mig: false, sub: false,
      arch: false, tag: null });
    setQ("");
  };
  const railItems = [
    { k: "overview", label: t("app:rail.overview") },
    { k: "library", label: t("app:rail.library") },
    { k: "history", label: t("app:rail.history") },
    { k: "snapshots", label: t("app:rail.snapshots") }];

  const firstDone = () => {
    localStorage.setItem("ferry-first-done", "1");
    setView("library"); doScan();
    if (!guideSeen) setTimeout(() => setGuideStep(1), 300);
  };

  return (
    <div data-ferry-win="1" style={{ height: "100vh", display: "flex", flexDirection: "column",
      background: "var(--bg)", position: "relative", overflow: "hidden", fontSize: 13 }}>
      {/* 标题栏:红绿灯旁只放伸缩按钮,其余留白可拖拽窗口 */}
      <div data-tauri-drag-region style={{ height: 44, flex: "none",
        display: "flex", alignItems: "center", gap: 12, padding: "0 12px 0 78px",
        background: "var(--titlebar)", borderBottom: "1px solid var(--line)" }}>
        <button className="hov" onClick={() => setCollapsed(v => !v)}
          title={collapsed ? t("app:titlebar.expand") : t("app:titlebar.collapse")}
          style={{ width: 28, height: 26, display: "inline-flex", alignItems: "center",
            justifyContent: "center", background: "transparent", border: "none", borderRadius: 6,
            cursor: "pointer", color: "var(--tx3b)" }}>
          <SidebarIcon />
        </button>
        <div data-tauri-drag-region style={{ flex: 1, alignSelf: "stretch" }} />
        {view === "library" && (
          <button onClick={doScan} style={{ height: 26, display: "flex", alignItems: "center",
            gap: 7, padding: "0 11px", background: "var(--surface)", border: "1px solid var(--line)",
            borderRadius: 7, fontSize: 12.5, color: "var(--tx2)", cursor: "pointer" }}>
            {scanning ? <Spinner /> : <RescanIcon />}
            {scanning ? t("app:titlebar.scanning") : t("app:titlebar.rescan")}
          </button>
        )}
      </div>

      {/* 主体 */}
      <div style={{ flex: 1, display: "flex", minHeight: 0 }}>
        {/* 导航轨 */}
        <div style={{ width: 56, flex: "none", background: "var(--rail)", borderRight: "1px solid var(--rail-line)",
          display: "flex", flexDirection: "column", alignItems: "center", padding: "11px 0 12px",
          gap: 4, zIndex: 5 }}>
          {railItems.map(n => {
            const on = view === n.k;
            return (
              <button key={n.k} className="hov-rail"
                data-guide={n.k === "library" ? "rail" : undefined}
                onMouseEnter={e => railEnter(n.label, e)} onMouseLeave={railLeave}
                onClick={() => { setView(n.k); setSettingsOpen(false); setPopover(null); railLeave(); }}
                style={{ width: 40, height: 40, border: "none", borderRadius: 9,
                  background: on ? "var(--acc-soft2)" : "transparent", display: "flex", alignItems: "center",
                  justifyContent: "center", cursor: "pointer", transition: "background .12s ease" }}>
                <RailGlyph name={n.k} color={on ? ACCENT : "var(--tx4b)"} />
              </button>
            );
          })}
          <div style={{ flex: 1 }} />
          <button className="hov-rail"
            onMouseEnter={e => railEnter(t("app:rail.settings"), e)} onMouseLeave={railLeave}
            onClick={() => { setSettingsOpen(v => !v); railLeave(); }}
            style={{ width: 40, height: 40, border: "none", borderRadius: 9,
              background: settingsOpen ? "var(--acc-soft2)" : "transparent", display: "flex",
              alignItems: "center", justifyContent: "center", cursor: "pointer",
              transition: "background .12s ease" }}>
            <RailGlyph name="settings" color={settingsOpen ? ACCENT : "var(--tx4b)"} />
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
                ? <div style={{ padding: "34px 12px", textAlign: "center", color: "var(--tx5)",
                    fontSize: 12.5, display: "flex", alignItems: "center", justifyContent: "center",
                    gap: 8 }}><Spinner /> {t("app:detail.scanningSessions")}</div>
                : <LibraryList groups={libGroups}
                    empty={libGroups.length === 0} onClear={clearLibF} />)}
            {view === "history" && (
              <HistoryList groups={histGroups} empty={histFiltered.length === 0}
                onClear={() => { setHistF({ src: [...TOOLS], target: "all", status: "all", time: "all" }); setHq(""); }} />)}
            {view === "snapshots" && (
              <SnapList rows={snapListRows} empty={snapFiltered.length === 0}
                onClear={() => { setSnapF({ src: [...TOOLS], reason: "all", session: "all", time: "all" }); setSq(""); }} />)}
          </Pane>
        )}

        {/* 拖拽分隔条 */}
        {paneCfg && !collapsed && (
          <div onMouseDown={startDrag} onDoubleClick={() => setPaneW(328)}
            title={t("app:drag.hint")}
            style={{ width: 9, flex: "none", cursor: "col-resize", position: "relative",
              background: dragging ? "var(--acc-soft2)" : "transparent", zIndex: 6 }}>
            <div style={{ position: "absolute", left: 0, top: 0, bottom: 0, width: 1,
              background: dragging ? ACCENT : "var(--line)" }} />
          </div>
        )}

        {/* 详情区 */}
        {view === "overview" && (
          <Overview sessions={sessions} historyRows={historyRows} snapItems={snapItems}
            prices={pricing?.prices || {}} />)}
        {view === "library" && (cur ? (
          <SessionDetail key={selId}
            meta={metaMap[cur.id]?.name ? { ...cur, title: metaMap[cur.id].name } : cur}
            data={detail?.data} error={detail?.error}
            onDiscardAll={() => setOps([])}
            scope={scope} setScope={setScope}
            ops={ops} addOp={addOp} removeOp={removeOp} updateOp={updateOp}
            editCaps={editCaps} authoringCaps={authoringCaps}
            startReplyEdit={startReplyEdit} authoringError={authoringError}
            onOpenDiff={openDiff} onApply={() => setConfirmApply(true)} applying={applying}
            onOpenMigrate={sc => setMig({ scope: sc ?? scope })} />
        ) : (
          <div style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center",
            color: "var(--tx5)", fontSize: 13 }}>
            {scanning ? t("app:detail.scanningSessions") : t("app:detail.noSessionToDisplay")}</div>
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
          defaultProbe={!!settings.runtimeProbe}
          onClose={() => setMig(null)}
          onDone={() => { loadHistory(); loadSnaps(); }} />)}
      {diff && <DiffSheet ops={ops} preview={diff.preview} loading={diff.loading} error={diff.error}
        onClose={() => setDiff(null)} />}
      {confirmApply && <ApplyConfirm ops={ops} saveMode={saveMode} setSaveMode={setSaveMode}
        editCaps={editCaps} onCancel={() => setConfirmApply(false)} onConfirm={applyEdit} />}
      {snapConfirm && <SnapRestoreConfirm snap={snapConfirm}
        onCancel={() => setSnapConfirm(null)} onConfirm={confirmRestore} />}
      {ctxMenu && ctxItems && (
        <ContextMenu x={ctxMenu.x} y={ctxMenu.y} items={ctxItems}
          onClose={() => setCtxMenu(null)} />)}
      {delConfirm && (
        <SessionDeleteConfirm sess={delConfirm}
          onCancel={() => setDelConfirm(null)}
          onConfirm={() => deleteSession(delConfirm)} />)}
      {batchDel && (
        <BatchDeleteConfirm sessions={batchDel}
          onCancel={() => setBatchDel(null)} onConfirm={doBatchDelete} />)}
      {renameFor && (
        <PromptBox title={t("app:prompt.renameTitle")}
          desc={t("app:prompt.renameDesc", { title: renameFor.title || renameFor.id })}
          placeholder={t("app:prompt.renamePlaceholder")} confirmLabel={t("app:prompt.save")}
          initial={metaMap[renameFor.id]?.name || renameFor.title || ""}
          onCancel={() => setRenameFor(null)}
          onConfirm={v => { setRenameFor(null); setMetaFor(renameFor.id, { name: v }); }} />)}
      {tagFor && (
        <PromptBox
          title={tagFor.batch ? t("app:prompt.tagsBatchTitle", { n: tagFor.ids.length }) : t("app:prompt.tagsTitle")}
          desc={tagFor.batch ? t("app:prompt.tagsBatchDesc")
            : t("app:prompt.tagsDesc")}
          placeholder={t("app:prompt.tagsPlaceholder")} confirmLabel={t("app:prompt.save")}
          initial={tagFor.batch ? "" : (metaMap[tagFor.ids[0]]?.tags || []).join(", ")}
          onCancel={() => setTagFor(null)}
          onConfirm={async v => {
            setTagFor(null);
            const tags = v.split(/[,，]/).map(t => t.trim()).filter(Boolean);
            for (const id of tagFor.ids) {
              const merged = tagFor.batch
                ? [...new Set([...(metaMap[id]?.tags || []), ...tags])] : tags;
              await setMetaFor(id, { tags: merged });
            }
          }} />)}
      {toast && <Toast toast={toast} onDismiss={() => setToast(null)} />}
      {railTip && (
        <div style={{ position: "absolute", left: 62, top: railTip.top,
          transform: "translateY(-50%)", zIndex: 60, background: "var(--tooltip)", color: "#fff",
          fontSize: 11.5, padding: "5px 9px", borderRadius: 6,
          boxShadow: "0 6px 16px -6px rgba(0,0,0,.4)", pointerEvents: "none",
          whiteSpace: "nowrap", animation: "ffade .1s ease" }}>{railTip.label}</div>)}
      {settingsOpen && (
        <SettingsPage settings={settings} setSettings={setSettings}
          updater={updater}
          scan={scan} env={env} scanning={scanning} onRescan={doScan}
          guideSeen={guideSeen}
          onOpenGuide={() => { setSettingsOpen(false); openGuide(); }}
          onFirstRun={() => { setSettingsOpen(false); setView("firstrun"); }}
          onClose={() => setSettingsOpen(false)} />)}
      {popover === "lib" && (
        <LibraryFilter f={libF} setF={setLibF} counts={counts} dirs={dirs} tags={allTags}
          onClose={() => setPopover(null)} onClear={clearLibF} />)}
      {popover === "hist" && (
        <HistoryFilter f={histF} setF={setHistF} onClose={() => setPopover(null)}
          onClear={() => { setHistF({ src: [...TOOLS], target: "all", status: "all", time: "all" }); setHq(""); }} />)}
      {popover === "snap" && (
        <SnapFilter f={snapF} setF={setSnapF} reasons={snapReasons}
          sessions={[...new Set(snapItems.map(s => s.title))].slice(0, 6)}
          onClose={() => setPopover(null)}
          onClear={() => { setSnapF({ src: [...TOOLS], reason: "all", session: "all", time: "all" }); setSq(""); }} />)}
      {guideStep > 0 && (
        <Guide step={guideStep} onGo={setGuideStep} onFinish={finishGuide} />)}
    </div>
  );
}
