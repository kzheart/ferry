// Ferry 主壳:标题栏 / 导航轨 / 资源栏 / 详情区 + 全部弹层(按原型复刻)
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { canReveal, onMenu, openTerminal, revealPath, rpc,
  preloadWindow, startWindowDrag, toggleWindowMaximize,
  writeClipboardText } from "../api/transport/rpc.js";
import { TOOLS, TOOL_NAME, onToolsHydrated, resumeDescriptor,
  toolHasCapability } from "../api/contract/tools.js";
import { ACCENT } from "../domain/tools/toolDisplay.js";
import { BUCKETS, bucketOf, fmtTime, repoOf, sessionRef } from "../domain/sessions/sessionModel.js";
import { addSessionAttachment, serializeSessionAttachment }
  from "../domain/sessions/sessionAttachment.js";
import { histStatus, STATUS_CODE } from "../features/migration/migrationModel.js";
import { RailGlyph, RescanIcon, SidebarIcon, Spinner } from "../components/ui/icons.jsx";
import { Sheet } from "../components/ui/primitives.jsx";
import { HistoryList, LibraryList, Pane } from "../components/layout/ResourcePane.jsx";
import Overview from "../features/overview/Overview.jsx";
import SessionDetail from "../features/browser/SessionDetail.jsx";
import HistoryDetail from "../features/migration/HistoryDetail.jsx";
import FirstRun from "../features/onboarding/FirstRun.jsx";
import MigrateSheet from "../features/migration/MigrateSheet.jsx";
import SettingsPage from "../features/settings/Settings.jsx";
import { BatchDeleteConfirm, ContextMenu, DiffSheet, Guide, HistoryDeleteConfirm,
  HistoryFilter, ApplyConfirm, LibraryFilter, PromptBox, SearchPalette,
  SessionDeleteConfirm, Toast } from "../components/ui/Overlays.jsx";
import AskFerry from "../features/askferry/AskFerry.jsx";
import AgentSessionList from "../features/askferry/AgentSessionList.jsx";
import { useAskFerry } from "../features/askferry/useAskFerry.js";
import { useSettings } from "../features/settings/useSettings.js";
import { useAppUpdater } from "../features/settings/useAppUpdater.js";
import { useBrowserData } from "../features/browser/useBrowserData.js";
import { useSessionEditing } from "../features/editing/useSessionEditing.js";
import OrganizationPanel from "../features/organizing/OrganizationPanel.jsx";
import { generateOrganizationProposal } from "../features/organizing/organizationService.js";

const RAIL_ORDER_KEY = "ferry-rail-order";
const DEFAULT_RAIL_ORDER = ["overview", "askferry", "library", "history"];

function loadRailOrder() {
  try {
    const saved = JSON.parse(localStorage.getItem(RAIL_ORDER_KEY) || "null");
    if (!Array.isArray(saved)) return [...DEFAULT_RAIL_ORDER];
    const known = new Set(DEFAULT_RAIL_ORDER);
    const order = saved.filter((key, index) => known.has(key) && saved.indexOf(key) === index);
    return [...order, ...DEFAULT_RAIL_ORDER.filter(key => !order.includes(key))];
  } catch {
    return [...DEFAULT_RAIL_ORDER];
  }
}

export default function App() {
  const { t, i18n } = useTranslation();
  // ----- 数据 -----
  const { env, scan, scanning, lastScan, historyRows, pricing,
    doScan, loadHistory, deleteHistory } = useBrowserData();

  // ----- 导航与选中 -----
  const [view, setView] = useState(
    () => localStorage.getItem("ferry-first-done") ? "overview" : "firstrun");
  const [selId, setSelId] = useState(null);
  const [selHist, setSelHist] = useState(null);
  const [detail, setDetail] = useState(null);   // {id, data, error}
  const [refreshing, setRefreshing] = useState(false);
  const [navigationTarget, setNavigationTarget] = useState(null);
  const [organizerOpen, setOrganizerOpen] = useState(false);
  const [peekId, setPeekId] = useState(null);  // Ask Ferry 卡片就地预览的会话 id

  // ----- 编辑 -----
  // ----- 迁移 -----
  const [mig, setMig] = useState(null);         // {scope}

  // ----- 布局 -----
  const [collapsed, setCollapsed] = useState(false);
  const [paneW, setPaneW] = useState(232);
  const [dragging, setDragging] = useState(false);

  // ----- Ask Ferry -----
  const ferry = useAskFerry();
  const [agentAttachments, setAgentAttachments] = useState([]);
  const [settingsSection, setSettingsSection] = useState("prefs");
  const [agentRenameFor, setAgentRenameFor] = useState(null);
  const [aq, setAq] = useState("");

  // ----- 搜索与筛选 -----
  const [q, setQ] = useState("");
  const [hq, setHq] = useState("");
  const [libF, setLibF] = useState(
    { src: [...TOOLS], time: "all", dir: null, mig: false, sub: false, tag: null });
  const [histF, setHistF] = useState({ src: [...TOOLS], target: "all", status: "all", time: "all" });
  const [popover, setPopover] = useState(null); // 'lib'|'hist'
  const popAnchor = useRef(null); // 筛选按钮 rect,弹层锚定用
  const [searchOpen, setSearchOpen] = useState(false); // 搜索命令面板
  const [ctxMenu, setCtxMenu] = useState(null); // {x, y, id, multi?}
  const [delConfirm, setDelConfirm] = useState(null);
  const [histDel, setHistDel] = useState(null);
  const [batchDel, setBatchDel] = useState(null);   // 待批量删除的会话数组
  const [renameFor, setRenameFor] = useState(null); // 待重命名的会话
  const [tagFor, setTagFor] = useState(null);       // {ids} 待编辑标签的会话
  const [multiSel, setMultiSel] = useState([]);     // 多选中的会话 id
  const [metaMap, setMetaMap] = useState({});       // 会话元数据 sidecar
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settings, setSettings] = useSettings();
  const updater = useAppUpdater(settings.autoCheckUpdates);
  const [railTip, setRailTip] = useState(null);  // {label, top}
  const [railOrder, setRailOrder] = useState(loadRailOrder);
  const [railDragging, setRailDragging] = useState(null);
  const [railDrop, setRailDrop] = useState(null); // {key, position}
  const tipTimer = useRef(null);
  const railPointer = useRef(null);
  const suppressRailClick = useRef(false);
  const [guideStep, setGuideStep] = useState(0);
  const [guideSeen, setGuideSeen] = useState(() => localStorage.getItem("ferry-guide-seen") === "1");
  const [collapsedGroups, setCollapsedGroups] = useState({ earlier: true });
  const visibleIds = useRef({});
  const knownSessions = useRef(null);
  const organizedSessions = useRef(new Set());
  const pendingOrganizationSessions = useRef(new Map());

  const sessions = scan?.sessions || [];
  const byId = useMemo(() => Object.fromEntries(sessions.map(s => [s.id, s])), [sessions]);
  const migratedIds = useMemo(() => new Set(historyRows.map(h => h.source_id)), [historyRows]);
  const cur = selId ? byId[selId] : null;
  const editing = useSessionEditing({ current: cur,
    runtimeProbe: !!settings.runtimeProbe, doScan,
    onInplaceApplied: () => select(selId) });

  useEffect(() => {
    const ids = new Set(sessions.map(session => `${session.tool}\0${session.id}`));
    if (knownSessions.current === null) {
      knownSessions.current = ids;
    }
    const additions = sessions.filter(session =>
      !knownSessions.current.has(`${session.tool}\0${session.id}`));
    knownSessions.current = ids;
    const opened = cur && !organizedSessions.current.has(`${cur.tool}\0${cur.id}`)
      ? [cur] : [];
    for (const session of [...additions, ...opened]) {
      const key = `${session.tool}\0${session.id}`;
      if (!organizedSessions.current.has(key)) {
        pendingOrganizationSessions.current.set(key, session);
      }
    }
    const pending = [...pendingOrganizationSessions.current.values()]
      .filter(session => !organizedSessions.current.has(`${session.tool}\0${session.id}`));
    if (!pending.length || ferry.health?.credential !== "available") return;
    pending.forEach(session => {
      const key = `${session.tool}\0${session.id}`;
      organizedSessions.current.add(key);
      pendingOrganizationSessions.current.delete(key);
    });
    void (async () => {
      for (const session of pending) {
        try {
          await generateOrganizationProposal([{
            ...session, project: repoOf(session.dir),
          }], i18n.language);
        } catch {
          const key = `${session.tool}\0${session.id}`;
          organizedSessions.current.delete(key);
          pendingOrganizationSessions.current.set(key, session);
        }
      }
    })();
  }, [sessions, cur, ferry.health?.credential, i18n.language]);
  const { ops, dirtyOps, setOps, diff, setDiff,
    confirmApply, setConfirmApply, toast, setToast, applying, scope, setScope,
    editCaps, resetSelection, loadCapabilities, addOp, startReplyEdit,
    removeOp, updateOp, authoringError, openDiff, prepareApply, applyEdit } = editing;

  // 清单水合(首启无缓存 / 引擎清单与缓存不一致)后,把"全选"态筛选器扩展到新全集
  useEffect(() => onToolsHydrated(() => {
    setLibF(v => ({ ...v, src: [...TOOLS] }));
    setHistF(v => ({ ...v, src: [...TOOLS] }));
  }), []);

  // 首次扫描完成后默认选中第一个会话
  useEffect(() => {
    if (!selId && sessions.length) select(sessions[0].id);
  }, [sessions]);

  // macOS 惯例快捷键:⌘, 打开设置(桌面端由原生菜单接管,避免与菜单加速键重复触发)
  useEffect(() => {
    if (canReveal()) return;
    const onKey = e => {
      if (e.metaKey && !e.shiftKey && !e.altKey && e.key === ",") {
        e.preventDefault(); setSettingsSection("prefs"); setSettingsOpen(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // 原生菜单栏事件:经 ref 转发,回调始终拿到最新闭包
  const menuActs = useRef({});
  menuActs.current = {
    settings: () => { setSettingsSection("prefs"); setSettingsOpen(true); },
    "toggle-sidebar": () => setCollapsed(v => !v),
    rescan: () => doScan(),
  };
  useEffect(() => {
    let un;
    onMenu(id => menuActs.current[id]?.()).then(u => { un = u; });
    return () => un?.();
  }, []);

  // 透明窗口下内建拖拽区失效,手动补上:主键点在 data-tauri-drag-region 元素本体上才触发,
  // 避免落到里面的按钮;双击标题栏切换最大化。窗口句柄需先预加载,否则同步栈里抓不到手势。
  useEffect(() => {
    preloadWindow();
    const onDown = e => {
      if (e.button !== 0) return;
      const el = e.target;
      if (!el?.hasAttribute?.("data-tauri-drag-region")) return;
      if (e.detail === 2) toggleWindowMaximize();
      else startWindowDrag();
    };
    window.addEventListener("mousedown", onDown);
    return () => window.removeEventListener("mousedown", onDown);
  }, []);

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
  // 详情 LRU 缓存:切回看过的会话立即渲染旧内容,后台再取最新
  const detailCache = useRef(new Map());
  const cacheDetail = (id, data) => {
    const c = detailCache.current;
    c.delete(id); c.set(id, data);
    if (c.size > 30) c.delete(c.keys().next().value);
  };

  const select = id => {
    setSelId(id); resetSelection();
    const s = byId[id] || sessions.find(x => x.id === id);
    if (!s) return;
    setDetail({ id, data: detailCache.current.get(id) || null });
    rpc("show", { tool: s.tool, ref: sessionRef(s) })
      .then(data => { cacheDetail(id, data);
        setDetail(d => d?.id === id ? { ...d, data } : d); })
      .catch(e => setDetail(d => d?.id === id ? { ...d, error: e.message } : d));
    loadCapabilities(s.tool);
  };

  // 把实体对应的会话装入选中态与详情缓存,不切换主视图。返回装入的会话 id。
  const loadEntitySession = (action, entity) => {
    const candidate = sessions.find(session =>
      (action.sessionId && session.id === action.sessionId) ||
      (action.ref && sessionRef(session) === action.ref) ||
      (entity?.title && session.tool === action.tool &&
        session.title === entity.title &&
        (!entity.project || repoOf(session.dir) === entity.project)));
    if (candidate) { select(candidate.id); return candidate.id; }
    if (action.tool && (action.ref || action.sessionId)) {
      const id = action.sessionId || action.ref;
      setSelId(id); resetSelection();
      setDetail({ id, data: null });
      rpc("show", { tool: action.tool, ref: action.ref || action.sessionId })
        .then(data => setDetail(current =>
          current?.id === id ? { ...current, data } : current))
        .catch(error => setDetail(current =>
          current?.id === id ? { ...current, error: error.message } : current));
      loadCapabilities(action.tool);
      doScan();
      return id;
    }
    return null;
  };

  // 会话卡片默认点击:就地在覆盖浮层里预览,不整页跳走(对话留在背景)。
  // usage / 迁移历史等无会话可预览的动作,在对话里不做导航。
  const peekEntity = (action, entity) => {
    if (action?.view !== "library") return;
    setSettingsOpen(false);
    setPopover(null);
    setNavigationTarget({ ...action, nonce: Date.now() });
    const id = loadEntitySession(action, entity);
    if (id) setPeekId(id);
  };

  // 显式动作(预览浮层里的「在会话库中打开」)才切换主视图。
  const navigateEntity = (action, entity) => {
    if (!action?.view) return;
    setSettingsOpen(false);
    setPopover(null);
    setNavigationTarget({ ...action, nonce: Date.now() });
    if (action.view === "library") {
      setView("library");
      loadEntitySession(action, entity);
      return;
    }
    if (action.view === "history") {
      setView("history");
      const candidate = histItems.find(item =>
        (action.migrationId && (item.id === action.migrationId ||
          item._id === action.migrationId)) ||
        (action.ref && (item.source_ref === action.ref || item.ref === action.ref)));
      if (candidate) setSelHist(candidate._id);
      return;
    }
    setView(action.view);
  };

  // 刷新当前会话:只重读这一个会话的正文,不触发全量扫描
  const refreshDetail = async () => {
    const s = selId && (byId[selId] || sessions.find(x => x.id === selId));
    if (!s || refreshing) return;
    setRefreshing(true);
    try {
      const data = await rpc("show", { tool: s.tool, ref: sessionRef(s) });
      cacheDetail(selId, data);
      setDetail(d => d?.id === selId ? { id: selId, data } : d);
    } catch (e) {
      setDetail(d => d?.id === selId ? { ...d, error: e.message } : d);
    }
    setRefreshing(false);
  };

  useEffect(() => {
    if (!ferry.mutationVersion) return;
    doScan();
    loadHistory();
    if (view === "library" && selId) refreshDetail();
  }, [ferry.mutationVersion]);

  // ----- 会话删除(先落一份备份,可撤销) -----
  const undoDelete = async snapshot => {
    setToast({ kind: "run", title: t("app:toast.restoring"), desc: t("app:toast.restoringDesc") });
    try {
      await rpc("session_undelete", { snapshot });
      doScan();
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
      detailCache.current.delete(s.id);
      if (selId === s.id) { setSelId(null); setDetail(null); }
      doScan();
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
        detailCache.current.delete(s.id);
        done++;
      } catch { fail++; }
    }
    if (targets.some(s => s.id === selId)) { setSelId(null); setDetail(null); }
    setMultiSel([]); doScan();
    setToast(fail
      ? { kind: "fail", title: t("app:toast.batchPartialFail"), desc: t("app:toast.batchPartialFailDesc", { done, fail }) }
      : { kind: "ok", title: t("app:toast.batchDone"),
          desc: t("app:toast.batchDoneDesc", { done }) });
  };

  const ctxSess = ctxMenu ? byId[ctxMenu.id] : null;
  const ctxMeta = ctxSess ? metaMap[ctxSess.id] || {} : {};
  const multiSess = multiSel.map(id => byId[id]).filter(Boolean);
  const addToAgent = session => {
    setAgentAttachments(list => addSessionAttachment(list, session));
    setView("askferry");
    setCtxMenu(null);
  };
  const copySessionReference = session => {
    const reference = serializeSessionAttachment(session);
    writeClipboardText(reference).then(() => {
      setToast({ kind: "ok", title: t("app:toast.sessionReferenceCopied"),
        desc: t("app:toast.sessionReferenceCopiedDesc") });
    }).catch(() => {});
  };
  const ctxItems = ctxMenu?.multi ? [
    { label: t("app:ctx.addTags"), onClick: () => setTagFor({ ids: [...multiSel], batch: true }) },
    { sep: true },
    { label: t("app:ctx.deleteN", { n: multiSess.length }), danger: true,
      onClick: () => setBatchDel(multiSess) },
    { sep: true },
    { label: t("app:ctx.cancelMulti"), onClick: () => setMultiSel([]) },
  ] : ctxSess ? [
    { label: t("app:ctx.addToAgent"), onClick: () => addToAgent(ctxSess) },
    { label: t("app:ctx.resumeTerminal"), hint: "↩", onClick: () => resumeDescriptor(
        ctxSess.tool, ctxSess.id, ctxSess.dir)
        .then(launch => openTerminal(launch, settings.terminalApp)).catch(() => {}) },
    ...(toolHasCapability(ctxSess.tool, "migrate-source") ? [{
      label: t("app:ctx.migrateTo"), onClick: () => {
        if (ctxSess.id !== selId) select(ctxSess.id);
        setMig({ scope: null }); },
    }] : []),
    { sep: true },
    { label: t("app:ctx.rename"), hint: "F2", onClick: () => setRenameFor(ctxSess) },
    { label: ctxMeta.pinned ? t("app:ctx.unpin") : t("app:ctx.pin"),
      onClick: () => setMetaFor(ctxSess.id, { pinned: !ctxMeta.pinned }) },
    { label: t("app:ctx.tags"), onClick: () => setTagFor({ ids: [ctxSess.id] }) },
    { sep: true },
    { label: t("app:ctx.copySessionReference"),
      onClick: () => copySessionReference(ctxSess) },
    { label: t("app:ctx.copyId"), onClick: () => writeClipboardText(ctxSess.id).catch(() => {}) },
    { label: t("app:ctx.copyResume"), onClick: () => resumeDescriptor(
        ctxSess.tool, ctxSess.id, ctxSess.dir)
        .then(d => writeClipboardText(d.display_command))
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
        // 桌面端 ⌘B 由原生菜单加速键接管,这里只兜底浏览器环境
        if (canReveal()) return;
        e.preventDefault(); setCollapsed(v => !v); return;
      }
      if ((e.metaKey || e.ctrlKey) && (e.key === "k" || e.key === "K") && paneCfg) {
        e.preventDefault(); setSearchOpen(true); return;
      }
      if (e.key === "Escape") {
        if (ctxMenu) setCtxMenu(null);
        else if (renameFor) setRenameFor(null);
        else if (tagFor) setTagFor(null);
        else if (batchDel) setBatchDel(null);
        else if (delConfirm) setDelConfirm(null);
        else if (histDel) setHistDel(null);
        else if (settingsOpen) setSettingsOpen(false);
        else if (popover) setPopover(null);
        else if (confirmApply) setConfirmApply(false);
        else if (diff) setDiff(null);
        else if (mig) setMig(null);
        else if (peekId) setPeekId(null);
        else if (multiSel.length) setMultiSel([]);
        else if (guideStep) finishGuide();
        return;
      }
      if (document.activeElement &&
          ["INPUT", "TEXTAREA"].includes(document.activeElement.tagName)) return;
      // 会话库快捷键:仅在没有弹层时生效
      const overlayOpen = ctxMenu || delConfirm || histDel || batchDel || renameFor || tagFor ||
        settingsOpen || popover || confirmApply || diff || mig || guideStep || searchOpen;
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
            .then(launch => openTerminal(launch, settings.terminalApp)).catch(() => {});
          return;
        }
      }
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const ids = visibleIds.current[view] || [];
        if (!ids.length) return;
        const curSel = view === "library" ? selId : selHist;
        let i = ids.indexOf(curSel);
        i = i < 0 ? 0 : Math.max(0, Math.min(ids.length - 1, i + (e.key === "ArrowDown" ? 1 : -1)));
        if (view === "library") select(ids[i]);
        else setSelHist(ids[i]);
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  });

  // ----- 拖拽分栏 -----
  const startDrag = e => {
    if (collapsed) return;
    const sx = e.clientX, sw = paneW;
    const move = ev => setPaneW(Math.max(190, Math.min(360, sw + (ev.clientX - sx))));
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

  const railDropAt = (x, y) => {
    const target = document.elementFromPoint(x, y)?.closest?.("[data-rail-key]");
    const key = target?.dataset.railKey;
    if (!DEFAULT_RAIL_ORDER.includes(key)) return null;
    const rect = target.getBoundingClientRect();
    return { key, position: y < rect.top + rect.height / 2 ? "before" : "after" };
  };
  const reorderRail = useCallback((source, target, position) => {
    if (!source || !target || source === target) return;
    setRailOrder(order => {
      const next = order.filter(key => key !== source);
      const index = next.indexOf(target) + (position === "after" ? 1 : 0);
      next.splice(index, 0, source);
      try { localStorage.setItem(RAIL_ORDER_KEY, JSON.stringify(next)); } catch { /* 私密模式等存储不可用时仅本次生效 */ }
      return next;
    });
  }, []);
  const endRailDrag = e => {
    const drag = railPointer.current;
    if (!drag || drag.pointerId !== e.pointerId) return;
    if (drag.dragging) {
      const drop = railDropAt(e.clientX, e.clientY);
      if (drop) reorderRail(drag.key, drop.key, drop.position);
      suppressRailClick.current = true;
      window.setTimeout(() => { suppressRailClick.current = false; }, 0);
    }
    e.currentTarget.releasePointerCapture?.(e.pointerId);
    railPointer.current = null;
    setRailDragging(null);
    setRailDrop(null);
  };
  const startRailDrag = e => {
    if (e.button !== 0 || e.isPrimary === false) return;
    railPointer.current = { key: e.currentTarget.dataset.railKey, pointerId: e.pointerId,
      x: e.clientX, y: e.clientY, dragging: false };
    e.currentTarget.setPointerCapture?.(e.pointerId);
  };
  const moveRailDrag = e => {
    const drag = railPointer.current;
    if (!drag || drag.pointerId !== e.pointerId) return;
    if (!drag.dragging) {
      if (Math.hypot(e.clientX - drag.x, e.clientY - drag.y) < 5) return;
      drag.dragging = true;
      setRailDragging(drag.key);
      railLeave();
    }
    e.preventDefault();
    setRailDrop(railDropAt(e.clientX, e.clientY));
  };
  const cancelRailDrag = e => {
    if (railPointer.current?.pointerId !== e.pointerId) return;
    railPointer.current = null;
    setRailDragging(null);
    setRailDrop(null);
  };
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

  // 行点击/右键:经 ref 转发保持回调身份稳定,memo 化的行组件才不会因新闭包全量重渲染
  const rowHandlers = useRef({});
  rowHandlers.current.click = (id, e) => {
    if (e.metaKey || e.ctrlKey) {           // ⌘点击:切换多选
      setMultiSel(sel => {
        const base = sel.length ? sel : (selId ? [selId] : []);
        return base.includes(id)
          ? base.filter(x => x !== id) : [...base, id];
      });
      return;
    }
    if (e.shiftKey && selId) {              // Shift 点击:按可见顺序范围选
      const ids = visibleIds.current.library || [];
      const a = ids.indexOf(selId), b = ids.indexOf(id);
      if (a >= 0 && b >= 0) {
        setMultiSel(ids.slice(Math.min(a, b), Math.max(a, b) + 1));
        return;
      }
    }
    setMultiSel([]); select(id);
  };
  // 更多按钮锚定在按钮下方;右键则锚定在指针位置
  rowHandlers.current.more = (id, e) => {
    const pos = e.type === "contextmenu"
      ? { x: e.clientX, y: e.clientY }
      : (() => {
          const rect = e.currentTarget.getBoundingClientRect();
          return { x: rect.right - 208, y: rect.bottom + 4 };
        })();
    if (multiSel.length > 1 && multiSel.includes(id)) {
      setCtxMenu({ ...pos, id, multi: true });
      return;
    }
    setMultiSel([]);
    if (id !== selId) select(id);
    setCtxMenu({ ...pos, id });
  };
  rowHandlers.current.pin = id =>
    setMetaFor(id, { pinned: !(metaMap[id] || {}).pinned });
  rowHandlers.current.delete = id => { const s = byId[id]; if (s) askDelete(s); };
  const onRowClick = useCallback((id, e) => rowHandlers.current.click(id, e), []);
  const onRowMore = useCallback((id, e) => rowHandlers.current.more(id, e), []);
  const onRowPin = useCallback(id => rowHandlers.current.pin(id), []);
  const onRowDelete = useCallback(id => rowHandlers.current.delete(id), []);

  // 详情区回调:同样经 ref 转发保持身份稳定,memo 化的 SessionDetail 才不会
  // 因侧边栏交互(展开分组/多选/悬停)产生的新闭包全量重渲染整条时间线
  const detailFns = useRef({});
  detailFns.current = {
    discardAll: () => setOps([]),
    setScope, addOp, removeOp, updateOp, startReplyEdit, authoringError,
    openDiff, apply: prepareApply,
    openMigrate: sc => setMig({ scope: sc ?? scope }),
    refresh: refreshDetail,
    resume: async meta => {
      setToast({ kind: "run", title: t("app:toast.openingTerminal"),
        desc: t("app:toast.openingTerminalDesc", { title: meta.title || meta.id }) });
      try {
        const launch = await resumeDescriptor(meta.tool, meta.id, meta.dir);
        await openTerminal(launch, settings.terminalApp);
        setToast({ kind: "ok", title: t("app:toast.terminalOpened"),
          desc: t("app:toast.terminalOpenedDesc") });
      } catch (error) {
        setToast({ kind: "fail", title: t("app:toast.openTerminalFail"), desc: error.message });
      }
    },
  };
  const detailActs = useMemo(() => ({
    onDiscardAll: () => detailFns.current.discardAll(),
    setScope: v => detailFns.current.setScope(v),
    addOp: (...a) => detailFns.current.addOp(...a),
    removeOp: (...a) => detailFns.current.removeOp(...a),
    updateOp: (...a) => detailFns.current.updateOp(...a),
    startReplyEdit: (...a) => detailFns.current.startReplyEdit(...a),
    authoringError: (...a) => detailFns.current.authoringError(...a),
    onOpenDiff: () => detailFns.current.openDiff(),
    onApply: () => detailFns.current.apply(),
    onOpenMigrate: sc => detailFns.current.openMigrate(sc),
    onRefresh: () => detailFns.current.refresh(),
    onResume: meta => detailFns.current.resume(meta),
  }), []);
  const detailMeta = useMemo(() => cur && metaMap[cur.id]?.name
    ? { ...cur, title: metaMap[cur.id].name } : cur, [cur, metaMap]);

  // 行展示数据与过滤用字段只依赖会话/元数据,预计算一次;
  // 之后筛选条件怎么变都只做轻量匹配,不再重建 3000+ 行的时间/文案字符串
  const libIndex = useMemo(() => sessions.map(s => {
    const m = metaMap[s.id] || {};
    const tags = m.tags || [];
    return {
      tool: s.tool, bucket: bucketOf(s.updated), repo: repoOf(s.dir), tags,
      pinned: !!m.pinned, sub: (s.tree_count || 1) > 1, mig: migratedIds.has(s.id),
      hay: `${s.title || ""}\n${m.name || ""}\n${tags.join("\n")}\n${s.dir || ""}\n${s.id}`.toLowerCase(),
      row: { id: s.id, title: m.name || s.title || t("app:library.untitled"), repo: repoOf(s.dir),
        dir: s.dir, active: fmtTime(s.updated, t), tool: s.tool, dot: "var(--ok)",
        pinned: !!m.pinned, tags: m.tags,
        hasSub: (s.tree_count || 1) > 1, subLabel: t("app:library.subLabel", { n: (s.tree_count || 1) - 1 }),
        hasMig: migratedIds.has(s.id) },
    };
  }), [sessions, metaMap, migratedIds, t]);

  const libGroups = useMemo(() => {
    const timeBuckets = { all: [...BUCKETS], today: ["today"],
      last7: ["today", "yesterday", "last7"],
      last30: ["today", "yesterday", "last7", "last30"] }[libF.time];
    const match = e => libF.src.includes(e.tool) &&
      (!libF.tag || e.tags.includes(libF.tag)) &&
      (!libF.dir || e.repo === libF.dir) &&
      (!libF.mig || e.mig) && (!libF.sub || e.sub) &&
      (!ql || e.hay.includes(ql));
    const byKey = { pinned: [] };
    BUCKETS.forEach(k => { byKey[k] = []; });
    for (const e of libIndex) {
      if (!match(e)) continue;
      (e.pinned ? byKey.pinned : byKey[e.bucket]).push(e.row);
    }
    const groups = [];
    if (byKey.pinned.length) {
      groups.push({ key: "pinned", label: t("app:library.pinned"),
        count: byKey.pinned.length, rows: byKey.pinned });
    }
    BUCKETS.filter(k => timeBuckets.includes(k)).forEach(key => {
      if (!byKey[key].length) return;
      groups.push({ key, label: t(`common:bucket.${key}`),
        count: byKey[key].length, rows: byKey[key] });
    });
    return groups;
    // 折叠状态刻意不进依赖:展开/收起只切换渲染,不重算数据
  }, [libIndex, libF, ql, t]);
  const onToggleGroup = useCallback(key =>
    setCollapsedGroups(g => ({ ...g, [key]: !(g[key] ?? false) })), []);
  visibleIds.current.library = libGroups.filter(g => !(collapsedGroups[g.key] ?? false))
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
  if (libF.tag) libTokens.push({ label: t("app:library.tokenTag", { tag: libF.tag }),
    onRemove: () => setLibF(v => ({ ...v, tag: null })) });

  const allTags = useMemo(
    () => [...new Set(Object.values(metaMap).flatMap(m => m.tags || []))].slice(0, 12),
    [metaMap]);

  // ----- 资源栏数据:迁移历史 -----
  // 优先用引擎给的稳定 id:删除后下标会整体前移,按下标编号会让选中项跳到别的记录上
  const histItems = useMemo(() => historyRows.map((h, i) => ({
    ...h, _id: h.id ? `h${h.id}` : `h${i}-${h.time}`, status: histStatus(h),
  })), [historyRows]);
  const hql = hq.trim().toLowerCase();
  // 迁移历史此前每次渲染都重算分组;memo 后仅在数据/筛选/选中变化时重建
  const { histFiltered, histGroups } = useMemo(() => {
    const matchHist = h => histF.src.includes(h.src) &&
      (histF.target === "all" || h.dst === histF.target) &&
      (histF.status === "all" || h.status === histF.status) &&
      (histF.time === "all" || bucketOf(h.time) === histF.time ||
        (histF.time === "earlier" && !["today", "yesterday"].includes(bucketOf(h.time)))) &&
      (!hql || (h.title || "").toLowerCase().includes(hql) ||
        (h.session_id || "").toLowerCase().includes(hql));
    const histFiltered = histItems.filter(matchHist);
    const histGroups = [["today", t("app:historyToken.today")], ["yesterday", t("app:historyToken.yesterday")], ["earlier", t("app:historyToken.earlier")]].map(([k, label]) => ({
      label,
      rows: histFiltered.filter(h => k === "earlier"
        ? !["today", "yesterday"].includes(bucketOf(h.time)) : bucketOf(h.time) === k)
        .map(h => ({ id: h._id, title: h.title || h.source_id, short: fmtTime(h.time, t),
          from: TOOL_NAME[h.src], to: TOOL_NAME[h.dst], status: h.status,
          statusLabel: t(`common:${h.status}`),
          stColor: { [STATUS_CODE.success]: "var(--ok)", [STATUS_CODE.failed]: "var(--err)",
            [STATUS_CODE.rolledBack]: "var(--tx3b)" }[h.status],
          tool: h.src, selected: h._id === (selHist ?? histFiltered[0]?._id),
          // 旧缓存里的记录没有引擎 id,删不了,索性不给删除按钮
          deletable: !!h.id,
          onClick: () => setSelHist(h._id) })),
    })).filter(g => g.rows.length);
    return { histFiltered, histGroups };
  }, [histItems, histF, hql, selHist, t]);
  visibleIds.current.history = histFiltered.map(h => h._id);
  const histSel = histItems.find(h => h._id === selHist) || histFiltered[0] || null;
  const histTokens = [];
  if (histF.target !== "all") histTokens.push({ label: t("app:historyToken.target", { tool: TOOL_NAME[histF.target] }),
    onRemove: () => setHistF(v => ({ ...v, target: "all" })) });
  if (histF.status !== "all") histTokens.push({ label: t(`common:${histF.status}`),
    onRemove: () => setHistF(v => ({ ...v, status: "all" })) });
  if (histF.time !== "all") histTokens.push({
    label: t(`app:historyToken.${histF.time}`),
    onRemove: () => setHistF(v => ({ ...v, time: "all" })) });

  // ----- 资源栏数据:Ask Ferry 对话 -----
  const aql = aq.trim().toLowerCase();
  const ferrySessions = useMemo(() => (aql
    ? ferry.sessions.filter(s => (s.title || "").toLowerCase().includes(aql))
    : ferry.sessions).slice().sort((left, right) =>
      Number(!!right.pinned) - Number(!!left.pinned)
      || String(right.updated_at || "").localeCompare(String(left.updated_at || ""))),
  [ferry.sessions, aql]);

  // ----- 资源栏骨架配置 -----
  const paneCfg = {
    askferry: { title: t("askferry:pane.title"), count: String(ferry.sessions.length),
      placeholder: t("askferry:pane.placeholder"),
      query: aq, onQuery: e => setAq(e.target.value),
      filterCount: 0, tokens: [],
      footer: t("askferry:pane.footer", { n: ferry.sessions.length }) },
    library: { title: t("app:pane.libraryTitle"), count: String(sessions.length), placeholder: t("app:pane.libraryPlaceholder"),
      query: q, onQuery: e => setQ(e.target.value),
      filterCount: (libF.src.length < TOOLS.length ? 1 : 0) + (libF.time !== "all" ? 1 : 0) +
        (libF.dir ? 1 : 0) + (libF.mig ? 1 : 0) + (libF.sub ? 1 : 0) +
        (libF.tag ? 1 : 0),
      tokens: libTokens,
      footer: scan?.error ? t("app:pane.libraryFooterError", { error: scan.error })
        : multiSel.length > 1 ? t("app:pane.libraryFooterMulti", { n: multiSel.length })
        : t("app:pane.libraryFooterBrowsing", { n: sessions.length, lastScan: lastScan ? t("app:pane.libraryFooterLastScan", { time: fmtTime(lastScan, t) }) : "" }) },
    history: { title: t("app:pane.historyTitle"), count: String(histItems.length), placeholder: t("app:pane.historyPlaceholder"),
      query: hq, onQuery: e => setHq(e.target.value),
      filterCount: (histF.src.length < TOOLS.length ? 1 : 0) + (histF.target !== "all" ? 1 : 0) +
        (histF.status !== "all" ? 1 : 0) + (histF.time !== "all" ? 1 : 0),
      tokens: histTokens, footer: t("app:pane.historyFooter", { n: histItems.length }) },
  }[view] || null;

  const clearLibF = () => {
    setLibF({ src: [...TOOLS], time: "all", dir: null, mig: false, sub: false,
      tag: null });
    setQ("");
  };
  // 侧栏只剩导航轨(无资源栏或已折叠)时,导航轨要容纳红绿灯
  const railOnly = !paneCfg || collapsed;
  const railLabels = {
    overview: t("app:rail.overview"),
    library: t("app:rail.library"),
    history: t("app:rail.history"),
    askferry: t("askferry:rail"),
  };
  const railItems = railOrder.map(k => ({ k, label: railLabels[k] })).filter(n => n.label);

  const firstDone = () => {
    localStorage.setItem("ferry-first-done", "1");
    setView("library"); doScan();
    if (!guideSeen) setTimeout(() => setGuideStep(1), 300);
  };

  return (
    <div data-ferry-win="1" style={{ height: "100vh", display: "flex",
      background: "var(--win-bg)", position: "relative", overflow: "hidden", fontSize: 13 }}>
        {/* 导航轨:通高到窗口顶,顶部 44px 留给红绿灯、可拖拽窗口。
            与资源栏同底色构成一整块侧栏(Finder 式);
            侧栏只剩导航轨时加宽到 80,让红绿灯完整落在侧栏内,分界线也避开顶部 44px */}
        <div style={{ width: railOnly ? 80 : 56, flex: "none", background: "var(--pane)",
          position: "relative", display: "flex", flexDirection: "column", alignItems: "center",
          padding: "0 0 12px", gap: 4, zIndex: 5,
          transition: dragging ? "none" : "width .2s ease-out" }}>
          {railOnly && (
            <div style={{ position: "absolute", right: 0, top: 44, bottom: 0, width: 1,
              background: "var(--line)", pointerEvents: "none" }} />
          )}
          <div data-tauri-drag-region style={{ height: 44, alignSelf: "stretch", flex: "none" }} />
          {railItems.map(n => {
            const on = view === n.k;
            const dropBefore = railDrop?.key === n.k && railDrop.position === "before" && railDragging !== n.k;
            const dropAfter = railDrop?.key === n.k && railDrop.position === "after" && railDragging !== n.k;
            return (
              <button key={n.k} className="hov-rail"
                data-rail-key={n.k}
                data-guide={n.k === "library" ? "rail" : undefined}
                onMouseEnter={e => railEnter(n.label, e)} onMouseLeave={railLeave}
                onPointerDown={startRailDrag} onPointerMove={moveRailDrag}
                onPointerUp={endRailDrag} onPointerCancel={cancelRailDrag}
                onClick={() => {
                  if (suppressRailClick.current) return;
                  setView(n.k); setSettingsOpen(false); setPopover(null); railLeave();
                }}
                style={{ width: 40, height: 40, border: "none", borderRadius: 8,
                  background: on ? "var(--acc-soft2)" : "transparent", display: "flex", alignItems: "center",
                  justifyContent: "center", cursor: "default",
                  touchAction: "none", opacity: railDragging === n.k ? .48 : 1,
                  transform: railDragging === n.k ? "scale(.9)" : "none",
                  boxShadow: dropBefore ? `0 -2px 0 ${ACCENT}` : dropAfter ? `0 2px 0 ${ACCENT}` : "none",
                  transition: "background .12s ease, transform .12s ease, opacity .12s ease" }}>
                <RailGlyph name={n.k} color={on ? ACCENT : "var(--tx4b)"} />
              </button>
            );
          })}
          <div style={{ flex: 1 }} />
          <button className="hov-rail"
            onMouseEnter={e => railEnter(scanning ? t("app:titlebar.scanning") : t("app:titlebar.rescan"), e)}
            onMouseLeave={railLeave}
            disabled={scanning}
            onClick={() => { doScan(); railLeave(); }}
            style={{ width: 40, height: 40, border: "none", borderRadius: 8,
              background: "transparent", display: "flex", alignItems: "center",
              justifyContent: "center", cursor: "default", transition: "background .12s ease",
              color: "var(--tx4b)" }}>
            {scanning ? <Spinner size={18} /> : <RescanIcon size={18} color="var(--tx4b)" />}
          </button>
          <button className="hov-rail"
            onMouseEnter={e => railEnter(t("app:rail.settings"), e)} onMouseLeave={railLeave}
            onClick={() => { setSettingsSection("prefs"); setSettingsOpen(v => !v); railLeave(); }}
            style={{ width: 40, height: 40, border: "none", borderRadius: 8,
              background: settingsOpen ? "var(--acc-soft2)" : "transparent", display: "flex",
              alignItems: "center", justifyContent: "center", cursor: "default",
              transition: "background .12s ease" }}>
            <RailGlyph name="settings" color={settingsOpen ? ACCENT : "var(--tx4b)"} />
          </button>
        </div>

        {/* 资源栏 */}
        {paneCfg && (
          <Pane collapsed={collapsed} width={paneW} dragging={dragging}
            title={paneCfg.title} count={paneCfg.count} placeholder={paneCfg.placeholder}
            query={paneCfg.query}
            onOpenSearch={() => setSearchOpen(true)}
            onClearSearch={() => paneCfg.onQuery({ target: { value: "" } })}
            filterCount={paneCfg.filterCount}
            filterOn={popover === { library: "lib", history: "hist" }[view] ||
              paneCfg.filterCount > 0}
            onFilter={e => {
              popAnchor.current = e.currentTarget.getBoundingClientRect();
              setPopover(p => {
                const key = { library: "lib", history: "hist" }[view];
                return p === key ? null : key;
              });
            }}
            footer={paneCfg.footer} tokens={paneCfg.tokens}
            listKey={view}>
            {view === "library" && (
              scanning && !sessions.length
                ? <div style={{ padding: "34px 12px", textAlign: "center", color: "var(--tx5)",
                    fontSize: 12, display: "flex", alignItems: "center", justifyContent: "center",
                    gap: 8 }}><Spinner /> {t("app:detail.scanningSessions")}</div>
                : <LibraryList groups={libGroups}
                    collapsed={collapsedGroups} onToggle={onToggleGroup}
                    empty={libGroups.length === 0} onClear={clearLibF}
                    selectedId={selId} multiSel={multiSel}
                    onRowClick={onRowClick} onRowPin={onRowPin}
                    onRowDelete={onRowDelete} onRowMore={onRowMore} />)}
            {view === "history" && (
              <HistoryList groups={histGroups} empty={histFiltered.length === 0}
                onDelete={id => setHistDel(histItems.find(x => x._id === id))}
                onClear={() => { setHistF({ src: [...TOOLS], target: "all", status: "all", time: "all" }); setHq(""); }} />)}
            {view === "askferry" && (
              <AgentSessionList sessions={ferrySessions}
                activeId={ferry.activeId} onOpen={ferry.openSession} onNew={ferry.newChat}
                onPin={s => ferry.pin(s.session_id, !s.pinned).catch(ferry.reportError)}
                onDelete={s => ferry.deleteSession(s.session_id).catch(ferry.reportError)}
                onRename={setAgentRenameFor} />)}
          </Pane>
        )}

        {/* 拖拽分隔条 */}
        {paneCfg && !collapsed && (
          <div onMouseDown={startDrag} onDoubleClick={() => setPaneW(232)}
            title={t("app:drag.hint")}
            style={{ width: 9, flex: "none", cursor: "col-resize", position: "relative",
              background: dragging ? "var(--acc-soft2)" : "var(--bg)", zIndex: 6 }}>
            <div style={{ position: "absolute", left: 0, top: 0, bottom: 0, width: 1,
              background: dragging ? ACCENT : "var(--line)" }} />
          </div>
        )}

        {/* 内容列:自带 44px 工具栏,白底从窗口顶通到底(窗口透明走 vibrancy 时必须自带不透明底) */}
        <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column", background: "var(--bg)" }}>
        <div data-tauri-drag-region style={{ height: 44, flex: "none", display: "flex", alignItems: "center",
          gap: 12, padding: "0 12px" }}>
          {/* 侧栏开关常驻工具栏(macOS 惯例):无资源栏的视图置灰禁用,避免切视图时按钮突然消失 */}
          <button className={paneCfg ? "hov" : undefined} disabled={!paneCfg}
            onClick={() => setCollapsed(v => !v)}
            title={collapsed ? t("app:titlebar.expand") : t("app:titlebar.collapse")}
            style={{ width: 28, height: 26, display: "inline-flex", alignItems: "center",
              justifyContent: "center", background: "transparent", border: "none", borderRadius: 6,
              cursor: "default", color: "var(--tx3b)", opacity: paneCfg ? 1 : .35 }}>
            <SidebarIcon />
          </button>
          {view === "library" && (
            <button className="fbtn" onClick={() => setOrganizerOpen(true)}
              disabled={!sessions.length}
              style={{ height: 27, fontSize: 11 }}>
              {t("organizing:open")}
            </button>
          )}
          <div data-tauri-drag-region style={{ flex: 1, alignSelf: "stretch" }} />
        </div>
        <div style={{ flex: 1, minHeight: 0, display: "flex" }}>
        {view === "overview" && (
          <Overview sessions={sessions} historyRows={historyRows}
            prices={pricing?.prices || {}} scanning={scanning}
            navigationTarget={navigationTarget} />)}
        {view === "library" && (cur ? (
          <SessionDetail key={selId}
            meta={detailMeta}
            data={detail?.data} error={detail?.error}
            onDiscardAll={detailActs.onDiscardAll}
            scope={scope} setScope={detailActs.setScope}
            ops={ops} dirtyOps={dirtyOps} addOp={detailActs.addOp} removeOp={detailActs.removeOp}
            updateOp={detailActs.updateOp}
            editCaps={editCaps}
            startReplyEdit={detailActs.startReplyEdit} authoringError={detailActs.authoringError}
            onOpenDiff={detailActs.onOpenDiff} onApply={detailActs.onApply} applying={applying}
            onOpenMigrate={detailActs.onOpenMigrate}
            navigationTarget={navigationTarget}
            onRefresh={detailActs.onRefresh} refreshing={refreshing}
            onResume={detailActs.onResume} />
        ) : (
          <div style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center",
            color: "var(--tx5)", fontSize: 13 }}>
            {scanning ? t("app:detail.scanningSessions") : t("app:detail.noSessionToDisplay")}</div>
        ))}
        {view === "history" && (
          <HistoryDetail h={histSel} onDelete={() => setHistDel(histSel)} />)}
        {view === "askferry" && (
          <AskFerry ferry={ferry} scanSessions={sessions}
            attachments={agentAttachments} onAttachmentsChange={setAgentAttachments}
            onNavigate={peekEntity}
            onOpenConfig={(section = "providers") => {
              setSettingsSection(section); setSettingsOpen(true); }} />)}
        {view === "firstrun" && <FirstRun env={env} scan={scan} onStart={firstDone} />}
        </div>
      </div>

      {/* 弹层 */}
      {organizerOpen && (
        <OrganizationPanel
          sessions={sessions.map(session => ({
            ...session, project: repoOf(session.dir),
          }))}
          onClose={() => setOrganizerOpen(false)}
          onApplied={() => {
            rpc("session_meta_list").then(value => setMetaMap(value || {}));
            doScan();
          }} />
      )}
      {peekId && cur && (
        <Sheet width="min(940px, 94vw)" maxHeight="90vh"
          onClose={() => setPeekId(null)}>
          <div style={{ display: "flex", alignItems: "center", gap: 8,
            padding: "9px 12px 9px 16px", borderBottom: "1px solid var(--line5)" }}>
            <span style={{ flex: 1, minWidth: 0, fontSize: 13, fontWeight: 600,
              overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
              {detailMeta?.title || detailMeta?.id}
            </span>
            <button type="button"
              style={{ fontSize: 12, padding: "4px 10px", borderRadius: 7,
                border: "1px solid var(--line3)", background: "var(--surface)",
                color: "var(--acc)", cursor: "pointer" }}
              onClick={() => { setPeekId(null); setView("library"); }}>
              {t("askferry:peek.openInLibrary")} ↗
            </button>
            <button type="button"
              style={{ fontSize: 12, padding: "4px 10px", borderRadius: 7,
                border: "1px solid var(--line3)", background: "var(--surface)",
                color: "var(--tx3)", cursor: "pointer" }}
              onClick={() => setPeekId(null)}>
              {t("askferry:peek.close")}
            </button>
          </div>
          <div style={{ height: "min(720px, 78vh)", display: "flex", minHeight: 0 }}>
            <SessionDetail key={selId}
              meta={detailMeta}
              data={detail?.data} error={detail?.error}
              onDiscardAll={detailActs.onDiscardAll}
              scope={scope} setScope={detailActs.setScope}
              ops={ops} dirtyOps={dirtyOps} addOp={detailActs.addOp} removeOp={detailActs.removeOp}
              updateOp={detailActs.updateOp}
              editCaps={editCaps}
              startReplyEdit={detailActs.startReplyEdit} authoringError={detailActs.authoringError}
              onOpenDiff={detailActs.onOpenDiff} onApply={detailActs.onApply} applying={applying}
              onOpenMigrate={detailActs.onOpenMigrate}
              navigationTarget={navigationTarget}
              onRefresh={detailActs.onRefresh} refreshing={refreshing}
              onResume={detailActs.onResume} />
          </div>
        </Sheet>
      )}
      {mig && cur && (
        <MigrateSheet meta={cur} scope={mig.scope} env={env}
          defaultProbe={!!settings.runtimeProbe} terminalApp={settings.terminalApp}
          onClose={() => setMig(null)}
          onDone={() => loadHistory()} />)}
      {diff && <DiffSheet ops={dirtyOps} preview={diff.preview} loading={diff.loading} error={diff.error}
        onClose={() => setDiff(null)} />}
      {confirmApply && <ApplyConfirm ops={dirtyOps}
        onCancel={() => setConfirmApply(false)} onConfirm={applyEdit} />}
      {searchOpen && paneCfg && (
        <SearchPalette
          placeholder={paneCfg.placeholder}
          query={paneCfg.query} onQuery={paneCfg.onQuery}
          recentLabel={paneCfg.query ? null : t("app:search.recent")}
          emptyLabel={t("app:search.empty")}
          results={(view === "askferry"
            ? ferrySessions.map(s => ({
                id: s.session_id, title: s.title || t("askferry:chat.untitled"),
                tool: null, meta: s.model_id,
                onClick: () => ferry.openSession(s.session_id) }))
            : view === "history"
            ? histGroups.flatMap(g => g.rows).map(h => ({
                id: h.id, title: h.title, tool: h.tool, meta: `${h.from} → ${h.to}`,
                onClick: () => setSelHist(h.id) }))
            : libGroups.flatMap(g => g.rows).map(r => ({
                id: r.id, title: r.title, tool: r.tool, meta: r.repo,
                onClick: () => { setMultiSel([]); select(r.id); } }))
          ).slice(0, 60)}
          onClose={() => setSearchOpen(false)} />)}
      {ctxMenu && ctxItems && (
        <ContextMenu x={ctxMenu.x} y={ctxMenu.y} items={ctxItems}
          onClose={() => setCtxMenu(null)} />)}
      {delConfirm && (
        <SessionDeleteConfirm sess={delConfirm}
          onCancel={() => setDelConfirm(null)}
          onConfirm={() => deleteSession(delConfirm)} />)}
      {histDel && (
        <HistoryDeleteConfirm h={histDel}
          onCancel={() => setHistDel(null)}
          onConfirm={() => {
            // 删的正好是选中项才清选中,列表回落到第一条;删别的不该打断当前查看
            if (histDel._id === selHist) setSelHist(null);
            setHistDel(null);
            deleteHistory(histDel.id).catch(() => {});
          }} />)}
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
      {agentRenameFor && (
        <PromptBox title={t("askferry:pane.renameTitle")}
          desc={t("askferry:pane.renameDesc", { title: agentRenameFor.title || t("askferry:chat.untitled") })}
          placeholder={t("askferry:pane.renamePlaceholder")} confirmLabel={t("askferry:pane.save")}
          initial={agentRenameFor.title || ""} onCancel={() => setAgentRenameFor(null)}
          onConfirm={title => {
            setAgentRenameFor(null);
            if (title) ferry.rename(agentRenameFor.session_id, title).catch(ferry.reportError);
          }} />)}
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
        <div style={{ position: "absolute", left: railOnly ? 86 : 62, top: railTip.top,
          transform: "translateY(-50%)", zIndex: 60, background: "var(--tooltip)", color: "#fff",
          fontSize: 11, padding: "5px 9px", borderRadius: 6,
          boxShadow: "var(--shadow-menu)", pointerEvents: "none",
          whiteSpace: "nowrap", animation: "ffade .1s ease" }}>{railTip.label}</div>)}
      {settingsOpen && (
        <SettingsPage settings={settings} setSettings={setSettings}
          updater={updater} ferry={ferry} initialSection={settingsSection}
          scan={scan} env={env} scanning={scanning} onRescan={doScan}
          guideSeen={guideSeen}
          onOpenGuide={() => { setSettingsOpen(false); openGuide(); }}
          onFirstRun={() => { setSettingsOpen(false); setView("firstrun"); }}
          onClose={() => setSettingsOpen(false)} />)}
      {popover === "lib" && (
        <LibraryFilter f={libF} setF={setLibF} counts={counts} dirs={dirs} tags={allTags}
          anchor={popAnchor.current}
          onClose={() => setPopover(null)} onClear={clearLibF} />)}
      {popover === "hist" && (
        <HistoryFilter f={histF} setF={setHistF} anchor={popAnchor.current}
          onClose={() => setPopover(null)}
          onClear={() => { setHistF({ src: [...TOOLS], target: "all", status: "all", time: "all" }); setHq(""); }} />)}
      {guideStep > 0 && (
        <Guide step={guideStep} onGo={setGuideStep} onFinish={finishGuide} />)}
    </div>
  );
}
