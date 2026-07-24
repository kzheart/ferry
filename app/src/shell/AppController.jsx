// Ferry 主壳:标题栏 / 导航轨 / 资源栏 / 详情区 + 全部弹层(按原型复刻)
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { openTerminal, revealPath, writeClipboardText }
  from "../platform/desktop/client.js";
import { TOOLS, TOOL_NAME, resumeDescriptor } from "../shared/contracts/tools.js";
import { fmtTime, sessionRef } from "../modules/browser/sessionModel.js";
import { addSessionAttachment, serializeSessionAttachment, sessionIdentity }
  from "../modules/browser/sessionAttachment.js";
import { SidebarIcon } from "../shared/ui/icons.jsx";
import { useAskFerry } from "../modules/askferry/useAskFerry.js";
import { useSettings } from "../modules/settings/useSettings.js";
import { useAppUpdater } from "../modules/settings/useAppUpdater.js";
import { useBrowserData } from "../modules/browser/useBrowserData.js";
import { useSessionEditing } from "../modules/editing/useSessionEditing.js";
import { useLibraryResourcePane } from "../modules/browser/useLibraryResourcePane.js";
import { useLibraryResourcePaneActions } from "../modules/browser/useLibraryResourcePaneActions.js";
import { useSessionDeletion } from "../modules/browser/useSessionDeletion.js";
import { useSessionMetadata } from "../modules/browser/useSessionMetadata.js";
import { useSessionSelection } from "../modules/browser/useSessionSelection.js";
import { useHistoryResourcePane } from "../modules/migration/useHistoryResourcePane.js";
import { useDesktopChrome } from "./useDesktopChrome.js";
import { AppRail } from "./AppRail.jsx";
import { AppShell } from "./AppShell.jsx";
import { AppOverlays } from "./AppOverlays.jsx";
import { WorkspaceRouter } from "./WorkspaceRouter.jsx";
import { ResourcePaneHost } from "./ResourcePaneHost.jsx";
import { useRailNavigation } from "./useRailNavigation.js";

export default function App() {
  const { t, i18n } = useTranslation();
  // ----- 数据 -----
  const { env, scan, scanning, lastScan, historyRows, pricing,
    doScan, loadHistory, deleteHistory } = useBrowserData();

  // ----- 导航与选中 -----
  const [view, setView] = useState(
    () => localStorage.getItem("ferry-first-done") ? "overview" : "firstrun");
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
  const [popover, setPopover] = useState(null); // 'lib'|'hist'
  const popAnchor = useRef(null); // 筛选按钮 rect,弹层锚定用
  const [searchOpen, setSearchOpen] = useState(false); // 搜索命令面板
  const [ctxMenu, setCtxMenu] = useState(null); // {x, y, key, multi?}
  const [delConfirm, setDelConfirm] = useState(null);
  const [histDel, setHistDel] = useState(null);
  const [batchDel, setBatchDel] = useState(null);   // 待批量删除的会话数组
  const [renameFor, setRenameFor] = useState(null); // 待重命名的会话
  const [tagFor, setTagFor] = useState(null);       // {sessions} 待编辑标签的会话
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settings, setSettings] = useSettings();
  const updater = useAppUpdater(settings.autoCheckUpdates);
  const [guideStep, setGuideStep] = useState(0);
  const [guideSeen, setGuideSeen] = useState(() => localStorage.getItem("ferry-guide-seen") === "1");

  const sessions = scan?.sessions || [];
  const selectionReset = useRef(() => {});
  const selection = useSessionSelection({
    sessions,
    onSelect: () => selectionReset.current(),
    onFallbackLoad: doScan,
  });
  const {
    selectedId: selId,
    detail,
    refreshing,
    sessionsByKey: byKey,
    select,
    loadEntitySession,
    refreshDetail,
    clearSelection,
    discardCachedDetail,
  } = selection;
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
  const history = useHistoryResourcePane({
    historyRows,
    t,
    toolIds: TOOLS,
    toolNames: TOOL_NAME,
  });
  const {
    query: hq,
    setQuery: setHq,
    filter: histF,
    setFilter: setHistF,
    items: histItems,
    filtered: histFiltered,
    groups: histGroups,
    selected: histSel,
    selectedId: histSelectedId,
    select: selectHistory,
    visibleIds: historyVisibleIds,
    filterCount: histFilterCount,
    tokens: histTokens,
    clear: clearHistF,
  } = history;
  const cur = selId ? byKey[selId] : null;
  const editing = useSessionEditing({ current: cur,
    runtimeProbe: !!settings.runtimeProbe, doScan,
    onInplaceApplied: () => select(selId) });

  const { ops, dirtyOps, setOps, diff, setDiff,
    confirmApply, setConfirmApply, toast, setToast, applying, scope, setScope,
    resetSelection, addOp, startReplyEdit,
    removeOp, updateOp, replyEditError, openDiff, prepareApply, applyEdit } = editing;
  selectionReset.current = resetSelection;
  const {
    metadata: metaMap,
    metaFor,
    reloadMetadata,
    updateMetadata: setMetaFor,
  } = useSessionMetadata({ setToast, t });
  const { deleteSession, deleteSessions } = useSessionDeletion({
    clearSelection,
    discardCachedDetail,
    doScan,
    selectedId: selId,
    setMultiIds: setMultiSel,
    setToast,
    t,
  });

  // 首次扫描完成后默认选中第一个会话
  useEffect(() => {
    if (!selId && sessions.length) select(sessionIdentity(sessions[0]));
  }, [sessions]);

  useDesktopChrome({
    onOpenSettings: () => { setSettingsSection("prefs"); setSettingsOpen(true); },
    onToggleSidebar: () => setCollapsed(value => !value),
    onRescan: doScan,
  });

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
      if (candidate) selectHistory(candidate._id);
      return;
    }
    setView(action.view);
  };

  useEffect(() => {
    if (!ferry.mutationVersion) return;
    doScan();
    loadHistory();
    if (view === "library" && selId) refreshDetail();
  }, [ferry.mutationVersion]);

  const askDelete = s => {
    if (s.tool === "opencode" || (s.tree_count || 1) > 1) setDelConfirm(s);
    else deleteSession(s);
  };
  const doBatchDelete = async () => {
    const targets = batchDel;
    setBatchDel(null);
    await deleteSessions(targets);
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
        ctxSess.tool, sessionRef(ctxSess))
        .then(launch => openTerminal(launch, settings.terminalApp)).catch(() => {}) },
    ...(TOOLS.includes(ctxSess.tool) ? [{
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
        ctxSess.tool, sessionRef(ctxSess))
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
          resumeDescriptor(cur.tool, sessionRef(cur))
            .then(launch => openTerminal(launch, settings.terminalApp)).catch(() => {});
          return;
        }
      }
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const ids = view === "library" ? libraryVisibleIds
          : view === "history" ? historyVisibleIds : [];
        if (!ids.length) return;
        const curSel = view === "library" ? selId : histSelectedId;
        let i = ids.indexOf(curSel);
        i = i < 0 ? 0 : Math.max(0, Math.min(ids.length - 1, i + (e.key === "ArrowDown" ? 1 : -1)));
        if (view === "library") select(ids[i]);
        else selectHistory(ids[i]);
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

  const finishGuide = () => {
    setGuideStep(0); setGuideSeen(true);
    localStorage.setItem("ferry-guide-seen", "1");
  };

  const { onRowClick, onRowMore, onRowPin, onRowDelete } = useLibraryResourcePaneActions({
    sessionsByKey: byKey,
    selectedId: selId,
    visibleIds: libraryVisibleIds,
    multiIds: multiSel,
    setMultiIds: setMultiSel,
    onSelect: select,
    onTogglePin: session => setMetaFor(
      session, { pinned: !metaFor(session).pinned },
    ),
    onDelete: askDelete,
    onOpenMenu: setCtxMenu,
  });

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
        const launch = await resumeDescriptor(meta.tool, sessionRef(meta));
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
      filterCount: histFilterCount,
      tokens: histTokens, footer: t("app:pane.historyFooter", { n: histItems.length }) },
  }[view] || null;

  // 侧栏只剩导航轨(无资源栏或已折叠)时,导航轨要容纳红绿灯
  const railOnly = !paneCfg || collapsed;
  const rail = useRailNavigation({
    labels: {
      overview: t("app:rail.overview"),
      library: t("app:rail.library"),
      history: t("app:rail.history"),
      askferry: t("askferry:rail"),
    },
    storageKey: "ferry-rail-order",
  });

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
          items={rail.items}
          activeView={view}
          draggingKey={rail.draggingKey}
          dropTarget={rail.dropTarget}
          scanning={scanning}
          settingsOpen={settingsOpen}
          scanningLabel={t("app:titlebar.scanning")}
          rescanLabel={t("app:titlebar.rescan")}
          settingsLabel={t("app:rail.settings")}
          onSelect={key => {
            if (rail.shouldSuppressClick()) return;
            setView(key); setSettingsOpen(false); setPopover(null); rail.leave();
          }}
          onRescan={() => { doScan(); rail.leave(); }}
          onToggleSettings={() => {
            setSettingsSection("prefs"); setSettingsOpen(value => !value); rail.leave();
          }}
          onEnter={rail.enter}
          onLeave={rail.leave}
          pointerHandlers={rail.pointerHandlers}
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
              onClear: clearHistF,
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

      <AppOverlays
        t={t}
        organization={{
          open: organizerOpen,
          sessions,
          onClose: () => setOrganizerOpen(false),
          onApplied: () => {
            reloadMetadata();
            doScan();
          },
        }}
        peek={{
          open: Boolean(peekId && cur),
          selectedId: selId,
          meta: detailMeta,
          detail,
          actions: detailActs,
          scope,
          ops,
          dirtyOps,
          applying,
          navigationTarget,
          refreshing,
          onClose: () => setPeekId(null),
          onOpenLibrary: () => {
            setPeekId(null);
            setView("library");
          },
        }}
        migration={{
          open: Boolean(mig && cur),
          meta: cur,
          scope: mig?.scope,
          env,
          defaultProbe: Boolean(settings.runtimeProbe),
          terminalApp: settings.terminalApp,
          onClose: () => setMig(null),
          onDone: loadHistory,
        }}
        editing={{
          diff,
          dirtyOps,
          confirmApply,
          onCloseDiff: () => setDiff(null),
          onCancelApply: () => setConfirmApply(false),
          onConfirmApply: applyEdit,
        }}
        search={{
          open: searchOpen,
          pane: paneCfg,
          results: (view === "askferry"
            ? ferrySessions.map(session => ({
                id: session.session_id,
                title: session.title || t("askferry:chat.untitled"),
                tool: null,
                meta: session.model_id,
                onClick: () => ferry.openSession(session.session_id),
              }))
            : view === "history"
              ? histGroups.flatMap(group => group.rows).map(item => ({
                  id: item.id,
                  title: item.title,
                  tool: item.tool,
                  meta: `${item.from} → ${item.to}`,
                  onClick: () => selectHistory(item.id),
                }))
              : libGroups.flatMap(group => group.rows).map(row => ({
                  id: row.key,
                  title: row.title,
                  tool: row.tool,
                  meta: row.repo,
                  onClick: () => {
                    setMultiSel([]);
                    select(row.key);
                  },
                }))
          ).slice(0, 60),
          onClose: () => setSearchOpen(false),
        }}
        contextMenu={{
          open: Boolean(ctxMenu),
          x: ctxMenu?.x,
          y: ctxMenu?.y,
          items: ctxItems,
          onClose: () => setCtxMenu(null),
        }}
        sessionDelete={{
          session: delConfirm,
          onCancel: () => setDelConfirm(null),
          onConfirm: () => {
            const session = delConfirm;
            setDelConfirm(null);
            deleteSession(session);
          },
        }}
        historyDelete={{
          history: histDel,
          onCancel: () => setHistDel(null),
          onConfirm: () => {
            if (histDel._id === histSelectedId) selectHistory(null);
            setHistDel(null);
            deleteHistory(histDel.id).catch(() => {});
          },
        }}
        batchDelete={{
          sessions: batchDel,
          onCancel: () => setBatchDel(null),
          onConfirm: doBatchDelete,
        }}
        rename={{
          session: renameFor,
          initial: renameFor
            ? metaFor(renameFor).name || renameFor.title || ""
            : "",
          onCancel: () => setRenameFor(null),
          onConfirm: value => {
            setRenameFor(null);
            setMetaFor(renameFor, { name: value });
          },
        }}
        agentRename={{
          session: agentRenameFor,
          onCancel: () => setAgentRenameFor(null),
          onConfirm: title => {
            setAgentRenameFor(null);
            if (title) {
              ferry.rename(agentRenameFor.session_id, title).catch(ferry.reportError);
            }
          },
        }}
        tags={{
          selection: tagFor,
          initial: tagFor && !tagFor.batch
            ? (metaFor(tagFor.sessions[0]).tags || []).join(", ")
            : "",
          onCancel: () => setTagFor(null),
          onConfirm: async value => {
            setTagFor(null);
            const nextTags = value.split(/[,，]/).map(tag => tag.trim()).filter(Boolean);
            for (const session of tagFor.sessions) {
              const merged = tagFor.batch
                ? [...new Set([...(metaFor(session).tags || []), ...nextTags])]
                : nextTags;
              await setMetaFor(session, { tags: merged });
            }
          },
        }}
        toast={{ value: toast, onDismiss: () => setToast(null) }}
        railTip={{ value: rail.railTip, railOnly }}
        settings={{
          open: settingsOpen,
          value: settings,
          onChange: setSettings,
          updater,
          ferry,
          initialSection: settingsSection,
          scan,
          env,
          scanning,
          onRescan: doScan,
          guideSeen,
          onOpenGuide: () => {
            setSettingsOpen(false);
            openGuide();
          },
          onFirstRun: () => {
            setSettingsOpen(false);
            setView("firstrun");
          },
          onClose: () => setSettingsOpen(false),
        }}
        libraryFilter={{
          open: popover === "lib",
          value: libF,
          onChange: setLibF,
          counts,
          dirs,
          tags: allTags,
          anchor: popAnchor.current,
          onClose: () => setPopover(null),
          onClear: clearLibF,
        }}
        historyFilter={{
          open: popover === "hist",
          value: histF,
          onChange: setHistF,
          anchor: popAnchor.current,
          onClose: () => setPopover(null),
          onClear: clearHistF,
        }}
        guide={{ step: guideStep, onGo: setGuideStep, onFinish: finishGuide }}
      />
    </div>
  );
}
