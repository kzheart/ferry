// Ferry 主壳:标题栏 / 导航轨 / 资源栏 / 详情区 + 全部弹层(按原型复刻)
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { openTerminal, revealPath, rpc,
  operationApply, operationPlan,
  writeClipboardText } from "../api/transport/rpc.js";
import { TOOLS, TOOL_NAME, resumeDescriptor,
  toolHasCapability } from "../api/contract/tools.js";
import { BUCKETS, bucketOf, fmtTime, operationRef, repoOf,
  sessionRef } from "../domain/sessions/sessionModel.js";
import { addSessionAttachment, serializeSessionAttachment, sessionIdentity }
  from "../domain/sessions/sessionAttachment.js";
import { histStatus, STATUS_CODE } from "../features/migration/migrationModel.js";
import { SidebarIcon } from "../components/ui/icons.jsx";
import { Sheet } from "../components/ui/primitives.jsx";
import SessionDetail from "../features/browser/SessionDetail.jsx";
import MigrateSheet from "../features/migration/MigrateSheet.jsx";
import SettingsPage from "../features/settings/Settings.jsx";
import { BatchDeleteConfirm, ContextMenu, DiffSheet, Guide, HistoryDeleteConfirm,
  HistoryFilter, ApplyConfirm, LibraryFilter, PromptBox, SearchPalette,
  SessionDeleteConfirm, Toast } from "../components/ui/Overlays.jsx";
import { useAskFerry } from "../features/askferry/useAskFerry.js";
import { useSettings } from "../features/settings/useSettings.js";
import { useAppUpdater } from "../features/settings/useAppUpdater.js";
import { useBrowserData } from "../features/browser/useBrowserData.js";
import { useSessionEditing } from "../features/editing/useSessionEditing.js";
import { useLibraryResourcePane } from "../features/browser/useLibraryResourcePane.js";
import OrganizationPanel from "../features/organizing/OrganizationPanel.jsx";
import { useDesktopChrome } from "../features/shell/useDesktopChrome.js";
import { AppRail } from "../features/shell/AppRail.jsx";
import { AppShell } from "../features/shell/AppShell.jsx";
import { WorkspaceRouter } from "../features/shell/WorkspaceRouter.jsx";
import { ResourcePaneHost } from "../features/shell/ResourcePaneHost.jsx";

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
  const [selId, setSelId] = useState(null); // UI 内部会话身份: tool + native id
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
  const [hq, setHq] = useState("");
  const [histF, setHistF] = useState({ src: [...TOOLS], target: "all", status: "all", time: "all" });
  const [popover, setPopover] = useState(null); // 'lib'|'hist'
  const popAnchor = useRef(null); // 筛选按钮 rect,弹层锚定用
  const [searchOpen, setSearchOpen] = useState(false); // 搜索命令面板
  const [ctxMenu, setCtxMenu] = useState(null); // {x, y, key, multi?}
  const [delConfirm, setDelConfirm] = useState(null);
  const [histDel, setHistDel] = useState(null);
  const [batchDel, setBatchDel] = useState(null);   // 待批量删除的会话数组
  const [renameFor, setRenameFor] = useState(null); // 待重命名的会话
  const [tagFor, setTagFor] = useState(null);       // {sessions} 待编辑标签的会话
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
  const visibleIds = useRef({});

  const sessions = scan?.sessions || [];
  const byKey = useMemo(
    () => Object.fromEntries(sessions.map(s => [sessionIdentity(s), s])),
    [sessions],
  );
  const migratedSessionKeys = useMemo(
    () => new Set(historyRows.map(history => sessionIdentity({
      tool: history.src,
      id: history.source_id,
    })).filter(Boolean)),
    [historyRows],
  );
  const library = useLibraryResourcePane({
    sessions,
    metadata: metaMap,
    migratedSessionKeys,
    t,
    toolIds: TOOLS,
    toolNames: TOOL_NAME,
  });
  const {
    query: q,
    setQuery: setQ,
    filter: libF,
    setFilter: setLibF,
    counts,
    dirs,
    tags: allTags,
    groups: libGroups,
    collapsedGroups,
    toggleGroup: onToggleGroup,
    visibleIds: libraryVisibleIds,
    filterCount: libFilterCount,
    tokens: libTokens,
    clear: clearLibF,
    multiIds: multiSel,
    setMultiIds: setMultiSel,
  } = library;
  const cur = selId ? byKey[selId] : null;
  const editing = useSessionEditing({ current: cur,
    runtimeProbe: !!settings.runtimeProbe, doScan,
    onInplaceApplied: () => select(selId) });

  const { ops, dirtyOps, setOps, diff, setDiff,
    confirmApply, setConfirmApply, toast, setToast, applying, scope, setScope,
    editCaps, resetSelection, loadCapabilities, addOp, startReplyEdit,
    removeOp, updateOp, replyEditError, openDiff, prepareApply, applyEdit } = editing;

  // 首次扫描完成后默认选中第一个会话
  useEffect(() => {
    if (!selId && sessions.length) select(sessionIdentity(sessions[0]));
  }, [sessions]);

  useDesktopChrome({
    onOpenSettings: () => { setSettingsSection("prefs"); setSettingsOpen(true); },
    onToggleSidebar: () => setCollapsed(value => !value),
    onRescan: doScan,
  });

  // ----- 会话元数据(重命名/置顶/归档/标签,sidecar 存储) -----
  useEffect(() => {
    rpc("session_meta_list").then(m => setMetaMap(m || {})).catch(() => {});
  }, []);
  const metaFor = session => metaMap[sessionIdentity(session)] || {};
  const setMetaFor = async (session, patch) => {
    try {
      const plan = await operationPlan({
        kind: "metadata",
        tool: session.tool,
        ref: operationRef(session),
        patch,
      });
      const applied = await operationApply(plan.plan_id);
      const entry = applied.result.metadata;
      setMetaMap(m => {
        const next = { ...m };
        const key = sessionIdentity(session);
        if (entry && Object.keys(entry).length) next[key] = entry;
        else delete next[key];
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

  const select = key => {
    setSelId(key); resetSelection();
    const s = byKey[key] || sessions.find(x => sessionIdentity(x) === key);
    if (!s) return;
    setDetail({ id: key, data: detailCache.current.get(key) || null });
    rpc("show", { tool: s.tool, ref: sessionRef(s) })
      .then(data => { cacheDetail(key, data);
        setDetail(d => d?.id === key ? { ...d, data } : d); })
      .catch(e => setDetail(d => d?.id === key ? { ...d, error: e.message } : d));
    loadCapabilities(s.tool);
  };

  // 把实体对应的会话装入选中态与详情缓存,不切换主视图。返回装入的会话 id。
  const loadEntitySession = (action, entity) => {
    const candidate = sessions.find(session =>
      (action.sessionId && session.tool === action.tool && session.id === action.sessionId) ||
      (action.ref && sessionRef(session) === action.ref) ||
      (entity?.title && session.tool === action.tool &&
        session.title === entity.title &&
        (!entity.project || repoOf(session.dir) === entity.project)));
    if (candidate) {
      const key = sessionIdentity(candidate);
      select(key);
      return key;
    }
    if (action.tool && (action.ref || action.sessionId)) {
      const key = sessionIdentity({ tool: action.tool, id: action.sessionId || action.ref });
      setSelId(key); resetSelection();
      setDetail({ id: key, data: null });
      rpc("show", { tool: action.tool, ref: action.ref || action.sessionId })
        .then(data => setDetail(current =>
          current?.id === key ? { ...current, data } : current))
        .catch(error => setDetail(current =>
          current?.id === key ? { ...current, error: error.message } : current));
      loadCapabilities(action.tool);
      doScan();
      return key;
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
    const s = selId && (byKey[selId] || sessions.find(x => sessionIdentity(x) === selId));
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
  const undoDelete = async recoveryId => {
    setToast({ kind: "run", title: t("app:toast.restoring"), desc: t("app:toast.restoringDesc") });
    try {
      const plan = await operationPlan({
        kind: "restore-delete",
        recovery_id: recoveryId,
      });
      await operationApply(plan.plan_id);
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
      const plan = await operationPlan({
        kind: "delete",
        tool: s.tool,
        ref: operationRef(s),
      });
      const r = (await operationApply(plan.plan_id)).result;
      const key = sessionIdentity(s);
      detailCache.current.delete(key);
      if (selId === key) { setSelId(null); setDetail(null); }
      doScan();
      setToast({ kind: "ok", title: t("app:toast.deleteDone"),
        desc: t("app:toast.deleteDoneDesc", { title: s.title || s.id }),
        action: r.undoable
          ? { label: t("app:toast.undo"), onClick: () => undoDelete(r.recovery_id) } : undefined });
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
        const plan = await operationPlan({
          kind: "delete",
          tool: s.tool,
          ref: operationRef(s),
        });
        await operationApply(plan.plan_id);
        detailCache.current.delete(sessionIdentity(s));
        done++;
      } catch { fail++; }
    }
    if (targets.some(s => sessionIdentity(s) === selId)) { setSelId(null); setDetail(null); }
    setMultiSel([]); doScan();
    setToast(fail
      ? { kind: "fail", title: t("app:toast.batchPartialFail"), desc: t("app:toast.batchPartialFailDesc", { done, fail }) }
      : { kind: "ok", title: t("app:toast.batchDone"),
          desc: t("app:toast.batchDoneDesc", { done }) });
  };

  const ctxSess = ctxMenu ? byKey[ctxMenu.key] : null;
  const ctxMeta = ctxSess ? metaFor(ctxSess) : {};
  const multiSess = multiSel.map(key => byKey[key]).filter(Boolean);
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
    { label: t("app:ctx.addTags"), onClick: () => setTagFor({ sessions: multiSess, batch: true }) },
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
        if (sessionIdentity(ctxSess) !== selId) select(sessionIdentity(ctxSess));
        setMig({ scope: null }); },
    }] : []),
    { sep: true },
    { label: t("app:ctx.rename"), hint: "F2", onClick: () => setRenameFor(ctxSess) },
    { label: ctxMeta.pinned ? t("app:ctx.unpin") : t("app:ctx.pin"),
      onClick: () => setMetaFor(ctxSess, { pinned: !ctxMeta.pinned }) },
    { label: t("app:ctx.tags"), onClick: () => setTagFor({ sessions: [ctxSess] }) },
    { sep: true },
    { label: t("app:ctx.copySessionReference"),
      onClick: () => copySessionReference(ctxSess) },
    { label: t("app:ctx.copyId"), onClick: () => writeClipboardText(ctxSess.id).catch(() => {}) },
    { label: t("app:ctx.copyResume"), onClick: () => resumeDescriptor(
        ctxSess.tool, ctxSess.id, ctxSess.dir)
        .then(d => writeClipboardText(d.display_command))
        .catch(() => {}) },
    { label: t("app:ctx.revealInFinder"), disabled: !ctxSess.path,
      disabledHint: t("app:ctx.noSessionFile"),
      onClick: () => revealPath(ctxSess.path).catch(() => {}) },
    { sep: true },
    { label: t("app:ctx.deleteSession"), hint: "⌫", danger: true, onClick: () => askDelete(ctxSess) },
  ] : null;

  // ----- 键盘 -----
  useEffect(() => {
    const onKey = e => {
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
          if (multiSel.length > 1) setBatchDel(multiSel.map(key => byKey[key]).filter(Boolean));
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
        const ids = view === "library" ? libraryVisibleIds : (visibleIds.current[view] || []);
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

  // 行点击/右键:经 ref 转发保持回调身份稳定,memo 化的行组件才不会因新闭包全量重渲染
  const rowHandlers = useRef({});
  rowHandlers.current.click = (key, e) => {
    if (e.metaKey || e.ctrlKey) {           // ⌘点击:切换多选
      setMultiSel(sel => {
        const base = sel.length ? sel : (selId ? [selId] : []);
        return base.includes(key)
          ? base.filter(x => x !== key) : [...base, key];
      });
      return;
    }
    if (e.shiftKey && selId) {              // Shift 点击:按可见顺序范围选
      const ids = libraryVisibleIds;
      const a = ids.indexOf(selId), b = ids.indexOf(key);
      if (a >= 0 && b >= 0) {
        setMultiSel(ids.slice(Math.min(a, b), Math.max(a, b) + 1));
        return;
      }
    }
    setMultiSel([]); select(key);
  };
  // 更多按钮锚定在按钮下方;右键则锚定在指针位置
  rowHandlers.current.more = (key, e) => {
    const pos = e.type === "contextmenu"
      ? { x: e.clientX, y: e.clientY }
      : (() => {
          const rect = e.currentTarget.getBoundingClientRect();
          return { x: rect.right - 208, y: rect.bottom + 4 };
        })();
    if (multiSel.length > 1 && multiSel.includes(key)) {
      setCtxMenu({ ...pos, key, multi: true });
      return;
    }
    setMultiSel([]);
    if (key !== selId) select(key);
    setCtxMenu({ ...pos, key });
  };
  rowHandlers.current.pin = key => {
    const session = byKey[key];
    if (session) setMetaFor(session, { pinned: !metaFor(session).pinned });
  };
  rowHandlers.current.delete = key => { const s = byKey[key]; if (s) askDelete(s); };
  const onRowClick = useCallback((key, e) => rowHandlers.current.click(key, e), []);
  const onRowMore = useCallback((key, e) => rowHandlers.current.more(key, e), []);
  const onRowPin = useCallback(key => rowHandlers.current.pin(key), []);
  const onRowDelete = useCallback(key => rowHandlers.current.delete(key), []);

  // 详情区回调:同样经 ref 转发保持身份稳定,memo 化的 SessionDetail 才不会
  // 因侧边栏交互(展开分组/多选/悬停)产生的新闭包全量重渲染整条时间线
  const detailFns = useRef({});
  detailFns.current = {
    discardAll: () => setOps([]),
    setScope, addOp, removeOp, updateOp, startReplyEdit, replyEditError,
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
    replyEditError: (...a) => detailFns.current.replyEditError(...a),
    onOpenDiff: () => detailFns.current.openDiff(),
    onApply: () => detailFns.current.apply(),
    onOpenMigrate: sc => detailFns.current.openMigrate(sc),
    onRefresh: () => detailFns.current.refresh(),
    onResume: meta => detailFns.current.resume(meta),
  }), []);
  const detailMeta = useMemo(() => cur && metaFor(cur).name
    ? { ...cur, title: metaFor(cur).name } : cur, [cur, metaMap]);

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
      filterCount: libFilterCount,
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

  // 侧栏只剩导航轨(无资源栏或已折叠)时,导航轨要容纳红绿灯
  const railOnly = !paneCfg || collapsed;
  const railLabels = {
    overview: t("app:rail.overview"),
    library: t("app:rail.library"),
    history: t("app:rail.history"),
    askferry: t("askferry:rail"),
  };
  const railItems = railOrder.map(key => ({ key, label: railLabels[key] })).filter(item => item.label);

  const firstDone = () => {
    localStorage.setItem("ferry-first-done", "1");
    setView("library"); doScan();
    if (!guideSeen) setTimeout(() => setGuideStep(1), 300);
  };

  return (
    <div data-ferry-win="1" style={{ height: "100vh", display: "flex",
      background: "var(--win-bg)", position: "relative", overflow: "hidden", fontSize: 13 }}>
      <AppShell
        rail={<AppRail
          railOnly={railOnly}
          resizing={dragging}
          items={railItems}
          activeView={view}
          draggingKey={railDragging}
          dropTarget={railDrop}
          scanning={scanning}
          settingsOpen={settingsOpen}
          scanningLabel={t("app:titlebar.scanning")}
          rescanLabel={t("app:titlebar.rescan")}
          settingsLabel={t("app:rail.settings")}
          onSelect={key => {
            if (suppressRailClick.current) return;
            setView(key); setSettingsOpen(false); setPopover(null); railLeave();
          }}
          onRescan={() => { doScan(); railLeave(); }}
          onToggleSettings={() => {
            setSettingsSection("prefs"); setSettingsOpen(value => !value); railLeave();
          }}
          onEnter={railEnter}
          onLeave={railLeave}
          pointerHandlers={{
            down: startRailDrag,
            move: moveRailDrag,
            up: endRailDrag,
            cancel: cancelRailDrag,
          }}
        />}
        resourcePane={paneCfg && (
          <ResourcePaneHost
            view={view}
            pane={paneCfg}
            collapsed={collapsed}
            width={paneW}
            resizing={dragging}
            filterOpen={popover === { library: "lib", history: "hist" }[view]}
            onOpenSearch={() => setSearchOpen(true)}
            onFilter={e => {
              popAnchor.current = e.currentTarget.getBoundingClientRect();
              setPopover(value => {
                const key = { library: "lib", history: "hist" }[view];
                return value === key ? null : key;
              });
            }}
            library={{
              scanning, sessions, scanningLabel: t("app:detail.scanningSessions"),
              groups: libGroups, collapsedGroups, onToggleGroup, onClear: clearLibF,
              selectedId: selId, multiSel, onRowClick, onRowPin, onRowDelete, onRowMore,
            }}
            history={{
              groups: histGroups, filtered: histFiltered,
              onDelete: id => setHistDel(histItems.find(item => item._id === id)),
              onClear: () => {
                setHistF({ src: [...TOOLS], target: "all", status: "all", time: "all" });
                setHq("");
              },
            }}
            agent={{
              sessions: ferrySessions, activeId: ferry.activeId,
              onOpen: ferry.openSession, onNew: ferry.newChat,
              onPin: session => ferry.pin(session.session_id, !session.pinned).catch(ferry.reportError),
              onDelete: session => ferry.deleteSession(session.session_id).catch(ferry.reportError),
              onRename: setAgentRenameFor,
            }}
          />
        )}
        showDivider={Boolean(paneCfg && !collapsed)}
        resizing={dragging}
        onResizeStart={startDrag}
        onResizeReset={() => setPaneW(232)}
        dividerTitle={t("app:drag.hint")}
        toolbar={<>
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
        </>}
      >
        <WorkspaceRouter
          view={view}
          sessions={sessions}
          historyRows={historyRows}
          pricing={pricing}
          scanning={scanning}
          navigationTarget={navigationTarget}
          currentSession={cur}
          selectedSessionId={selId}
          detailMeta={detailMeta}
          detail={detail}
          detailActions={{ ...detailActs, refreshing, onDeleteHistory: () => setHistDel(histSel) }}
          scope={scope}
          ops={ops}
          dirtyOps={dirtyOps}
          editCaps={editCaps}
          applying={applying}
          historySelection={histSel}
          ferry={ferry}
          agentAttachments={agentAttachments}
          onAgentAttachmentsChange={setAgentAttachments}
          onNavigate={peekEntity}
          onOpenConfig={(section = "providers") => {
            setSettingsSection(section); setSettingsOpen(true); }}
          environment={env}
          scan={scan}
          onFirstDone={firstDone}
          scanningLabel={t("app:detail.scanningSessions")}
          emptyLibraryLabel={t("app:detail.noSessionToDisplay")}
        />
      </AppShell>

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
              startReplyEdit={detailActs.startReplyEdit} replyEditError={detailActs.replyEditError}
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
                id: r.key, title: r.title, tool: r.tool, meta: r.repo,
                onClick: () => { setMultiSel([]); select(r.key); } }))
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
          initial={metaFor(renameFor).name || renameFor.title || ""}
          onCancel={() => setRenameFor(null)}
          onConfirm={v => { setRenameFor(null); setMetaFor(renameFor, { name: v }); }} />)}
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
          title={tagFor.batch ? t("app:prompt.tagsBatchTitle", { n: tagFor.sessions.length }) : t("app:prompt.tagsTitle")}
          desc={tagFor.batch ? t("app:prompt.tagsBatchDesc")
            : t("app:prompt.tagsDesc")}
          placeholder={t("app:prompt.tagsPlaceholder")} confirmLabel={t("app:prompt.save")}
          initial={tagFor.batch ? "" : (metaFor(tagFor.sessions[0]).tags || []).join(", ")}
          onCancel={() => setTagFor(null)}
          onConfirm={async v => {
            setTagFor(null);
            const tags = v.split(/[,，]/).map(t => t.trim()).filter(Boolean);
            for (const session of tagFor.sessions) {
              const merged = tagFor.batch
                ? [...new Set([...(metaFor(session).tags || []), ...tags])] : tags;
              await setMetaFor(session, { tags: merged });
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
