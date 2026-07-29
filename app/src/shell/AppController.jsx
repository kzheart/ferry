// Ferry 主壳:标题栏 / 导航轨 / 资源栏 / 详情区 + 全部弹层(按原型复刻)
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { TOOLS, TOOL_NAME } from "../shared/contracts/tools.js";
import { FerryRuntimeProvider } from "../shared/capabilities/ferryRuntime.jsx";
import { SessionEditingProvider } from "../shared/capabilities/sessionEditing.jsx";
import { BrowserStateProvider } from "../shared/capabilities/browserState.jsx";
import { OperationsStateProvider } from "../shared/capabilities/operationsState.jsx";
import { AppChromeProvider } from "../shared/capabilities/appChrome.jsx";
import {
  sessionIdentity,
  useBrowserData,
  useLibraryResourcePane,
  useSessionDeletion,
  useSessionMetadata,
  useSessionOptimization,
  useSessionSelection,
} from "../modules/browser/public.js";
import { useAskFerry } from "../modules/askferry/public.js";
import { useAppUpdater, useSettings } from "../modules/settings/public.js";
import { useSessionEditing } from "../modules/editing/public.js";
import { useHistoryResourcePane } from "../modules/migration/public.js";
import { initialWorkspace, useOnboarding } from "../modules/onboarding/public.js";
import { useDesktopChrome } from "./useDesktopChrome.js";
import { AppRail } from "./AppRail.jsx";
import { WorkspaceToolbar } from "./WorkspaceToolbar.jsx";
import { AppShell } from "./AppShell.jsx";
import { AppOverlayController } from "./AppOverlayController.jsx";
import { WorkspaceRouter } from "./WorkspaceRouter.jsx";
import { ResourcePaneHost } from "./ResourcePaneHost.jsx";
import { useAppKeyboardShortcuts } from "./useAppKeyboardShortcuts.js";
import { useRailNavigation } from "./useRailNavigation.js";
import { useResourcePaneLayout } from "./useResourcePaneLayout.js";
import { useResourcePaneConfig } from "./useResourcePaneConfig.js";
import { useWorkspaceInteractions } from "./useWorkspaceInteractions.js";
import { useWorkspaceState } from "./useWorkspaceState.js";

// 启动阶段 scan 还是 null,每次渲染新建的空数组会把下游 useMemo 全部击穿。
const EMPTY_SESSIONS = [];

export default function App() {
  const { t } = useTranslation();
  const {
    env,
    scan,
    scanning,
    historyRows,
    pricing,
    scanReady,
    doScan,
    loadHistory,
    deleteHistory,
  } = useBrowserData();

  const [view, setView] = useState(initialWorkspace);
  const [navigationTarget, setNavigationTarget] = useState(null);
  const [peekId, setPeekId] = useState(null); // Ask Ferry 卡片就地预览的会话 id
  const [floatChatOpen, setFloatChatOpen] = useState(false); // 会话库右下角浮动 Agent 面板

  const [mig, setMig] = useState(null); // {scope}

  const paneLayout = useResourcePaneLayout();

  const ferry = useAskFerry();
  const [agentAttachments, setAgentAttachments] = useState([]);
  const [settingsSection, setSettingsSection] = useState("prefs");
  const [aq, setAq] = useState("");

  const [popover, setPopover] = useState(null); // 'lib'|'hist'
  const popAnchor = useRef(null); // 筛选按钮 rect,弹层锚定用
  const [searchOpen, setSearchOpen] = useState(false); // 搜索命令面板
  const [ctxMenu, setCtxMenu] = useState(null); // {x, y, key, multi?}
  const [histDel, setHistDel] = useState(null);
  const [renameFor, setRenameFor] = useState(null); // 行内重命名中的会话
  const [tagFor, setTagFor] = useState(null); // {sessions} 待编辑标签的会话
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settings, setSettings] = useSettings();
  const updater = useAppUpdater(settings.autoCheckUpdates);
  // 内置优化器角色属于「会话优化」测试中功能:功能关闭时从角色列表整体隐身
  // (角色设置、问答角色下拉都经由这份 Context 取角色)
  const ferryValue = useMemo(() => {
    if (settings.sessionOptimization) return ferry;
    return {
      ...ferry,
      roles: (ferry.roles || []).filter(
        role => !(role.builtin && role.optimizer === true)),
    };
  }, [ferry, settings.sessionOptimization]);
  // 被隐藏的角色若正被问答选中,回落到默认角色
  useEffect(() => {
    if (ferry.roles.length
      && !ferryValue.roles.some(role => role.id === ferry.selectedRoleId)) {
      ferry.setSelectedRoleId("default");
    }
  }, [ferryValue.roles, ferry.selectedRoleId]);
  const onboarding = useOnboarding({
    setView,
    closeSettings: () => setSettingsOpen(false),
    closeMigration: () => setMig(null),
    scan: doScan,
  });

  const sessions = scan?.sessions || EMPTY_SESSIONS;
  const selectionReset = useRef(() => {});
  const selection = useSessionSelection({
    sessions,
    ready: scanReady,
    onSelect: () => selectionReset.current(),
    onFallbackLoad: doScan,
  });
  const {
    selectedId: selId,
    detail,
    refreshing,
    loadingMore,
    sessionsByKey: byKey,
    select,
    loadEntitySession,
    refreshDetail,
    loadMore,
    clearSelection,
    discardCachedDetail,
  } = selection;
  const cur = selId ? byKey[selId] : null;
  const editing = useSessionEditing({
    current: cur,
    runtimeProbe: !!settings.runtimeProbe,
    doScan,
    onInplaceApplied: () => select(selId),
  });
  // 仅解构主壳渲染直接需要的部分;逐个动作回调由 useWorkspaceInteractions 消费
  const {
    ops,
    dirtyOps,
    diff,
    setDiff,
    confirmApply,
    setConfirmApply,
    toast,
    setToast,
    applying,
    scope,
    resetSelection,
    applyEdit,
  } = editing;
  selectionReset.current = resetSelection;
  const {
    metadata: metaMap,
    metaFor,
    reloadMetadata,
    updateMetadata: setMetaFor,
  } = useSessionMetadata({ setToast, t });
  const migratedSessionKeys = useMemo(
    () =>
      new Set(
        historyRows
          .map((history) =>
            sessionIdentity({
              tool: history.src,
              id: history.source_id,
            }),
          )
          .filter(Boolean),
      ),
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
    toolIds: historyToolIds,
    filterCount: histFilterCount,
    tokens: histTokens,
    clear: clearHistF,
  } = history;
  const deletion = useSessionDeletion({
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

  // 会话优化:魔法棒 → 会话绑定的优化器角色生成改写候选 → 内联 diff 取舍 →
  // 接受项作为一个批次写回;全程留在浏览界面,不跳转
  const optimization = useSessionOptimization({
    enabled: !!settings.sessionOptimization,
    current: cur,
    roles: ferry.roles,
    runtimeProbe: !!settings.runtimeProbe,
    doScan,
    onApplied: () => select(selId),
  });

  // 打开设置并定位到指定分区:桌面菜单 / 路由 / 浮动面板共用
  const openConfig = (section = "providers") => {
    setSettingsSection(section);
    setSettingsOpen(true);
  };

  useDesktopChrome({
    onOpenSettings: () => openConfig("prefs"),
    onToggleSidebar: paneLayout.toggleCollapsed,
    onRescan: doScan,
  });

  useEffect(() => {
    if (!ferry.mutationVersion) return;
    doScan();
    loadHistory();
    if (view === "library" && selId) refreshDetail();
  }, [ferry.mutationVersion]);

  const {
    askDelete,
    peekEntity,
    contextMenuItems: ctxItems,
    detailActions: detailActs,
    detailMeta,
    onRowClick,
    onRowMore,
    onRowPin,
    onRowDelete,
    onRowRename,
    onRowRenameSubmit,
    onRowRenameCancel,
  } = useWorkspaceInteractions({
    t,
    settings,
    current: cur,
    selectedId: selId,
    sessionsByKey: byKey,
    metadata: metaMap,
    metaFor,
    updateMetadata: setMetaFor,
    multiIds: multiSel,
    setMultiIds: setMultiSel,
    libraryVisibleIds,
    menu: ctxMenu,
    setMenu: setCtxMenu,
    select,
    loadEntitySession,
    deletion,
    editing,
    scope,
    refreshDetail,
    loadMore,
    setToast,
    setView,
    setSettingsOpen,
    setPopover,
    setNavigationTarget,
    setPeekId,
    setMigration: setMig,
    setRename: setRenameFor,
    setTagSelection: setTagFor,
    setAgentAttachments,
  });

  const editingSurface = useMemo(
    () => ({ scope, ops, dirtyOps, applying, ...detailActs }),
    [scope, ops, dirtyOps, applying, detailActs],
  );

  const { paneConfig: paneCfg, ferrySessions } = useResourcePaneConfig({
    t,
    view,
    ferry,
    agentQuery: aq,
    setAgentQuery: setAq,
    sessions,
    libraryQuery: q,
    setLibraryQuery: setQ,
    libraryFilterCount: libFilterCount,
    libraryTokens: libTokens,
    historyItems: histItems,
    historyQuery: hq,
    setHistoryQuery: setHq,
    historyFilterCount: histFilterCount,
    historyTokens: histTokens,
  });

  // 侧栏只剩导航轨(无资源栏或已折叠)时,导航轨要容纳红绿灯
  const railOnly = !paneCfg || paneLayout.collapsed;
  const rail = useRailNavigation({
    labels: {
      overview: t("app:rail.overview"),
      library: t("app:rail.library"),
      history: t("app:rail.history"),
      askferry: t("askferry:rail"),
    },
    storageKey: "ferry-rail-order",
  });

  useAppKeyboardShortcuts({
    paneAvailable: Boolean(paneCfg),
    onOpenSearch: () => setSearchOpen(true),
    dismissers: [
      { open: Boolean(ctxMenu), dismiss: () => setCtxMenu(null) },
      { open: Boolean(renameFor), dismiss: () => setRenameFor(null) },
      { open: Boolean(tagFor), dismiss: () => setTagFor(null) },
      {
        open: Boolean(deletion.batchConfirmation),
        dismiss: deletion.cancelBatchDeletion,
      },
      {
        open: Boolean(deletion.sessionConfirmation),
        dismiss: deletion.cancelSessionDeletion,
      },
      { open: Boolean(histDel), dismiss: () => setHistDel(null) },
      { open: settingsOpen, dismiss: () => setSettingsOpen(false) },
      { open: Boolean(popover), dismiss: () => setPopover(null) },
      { open: confirmApply, dismiss: () => setConfirmApply(false) },
      { open: Boolean(diff), dismiss: () => setDiff(null) },
      { open: Boolean(mig), dismiss: () => setMig(null) },
      { open: Boolean(peekId), dismiss: () => setPeekId(null) },
      { open: searchOpen, dismiss: () => setSearchOpen(false) },
      {
        open: floatChatOpen && view === "library",
        dismiss: () => setFloatChatOpen(false),
      },
      { open: multiSel.length > 0, dismiss: () => setMultiSel([]) },
      { open: onboarding.step > 0, dismiss: onboarding.finishGuide },
    ],
    view,
    currentSession: cur,
    multiIds: multiSel,
    sessionsByKey: byKey,
    onRename: setRenameFor,
    onBatchDelete: deletion.requestBatchDeletion,
    onDelete: askDelete,
    onResume: detailActs.onResume,
    libraryVisibleIds,
    historyVisibleIds,
    selectedSessionId: selId,
    selectedHistoryId: histSelectedId,
    selectSession: select,
    selectHistory,
  });




  const { browserState, operationsState, appChrome } = useWorkspaceState({
    allTags, applyEdit, clearHistF, clearLibF, confirmApply,
    counts, ctxItems, ctxMenu, cur, deleteHistory, deletion, detail, detailActs, dirs,
    detailMeta, diff, dirtyOps, doScan, env, ferrySessions, floatChatOpen,
    histDel, histF, histGroups, histSel, histSelectedId, historyToolIds, libF,
    libGroups, loadHistory, loadingMore, metaFor, mig, navigationTarget,
    onboarding, openConfig, paneCfg, peekEntity, peekId,
    popAnchor, popover, rail, railOnly, refreshing,
    scan, scanning, searchOpen, select, selectHistory, selId,
    sessions, setConfirmApply, setCtxMenu, setDiff,
    setFloatChatOpen, setHistDel, setHistF, setLibF, setMetaFor, setMig,
    setMultiSel, setPeekId, setPopover,
    setSearchOpen, setSettings, setSettingsOpen, setTagFor, setToast, setView,
    settings, settingsOpen, settingsSection, tagFor, toast, updater, view,
  });

  return (
    <FerryRuntimeProvider value={ferryValue}>
    <SessionEditingProvider value={editingSurface}>
    <BrowserStateProvider value={browserState}>
    <OperationsStateProvider value={operationsState}>
    <AppChromeProvider value={appChrome}>
    <div
      data-ferry-win="1"
      style={{
        height: "100vh",
        display: "flex",
        background: "var(--win-bg)",
        position: "relative",
        overflow: "hidden",
        fontSize: 13,
      }}
    >
      <AppShell
        rail={
          <AppRail
            railOnly={railOnly}
            resizing={paneLayout.resizing}
            items={rail.items}
            activeView={view}
            draggingKey={rail.draggingKey}
            dropTarget={rail.dropTarget}
            scanning={scanning}
            settingsOpen={settingsOpen}
            scanningLabel={t("app:titlebar.scanning")}
            rescanLabel={t("app:titlebar.rescan")}
            settingsLabel={t("app:rail.settings")}
            onSelect={(key) => {
              if (rail.shouldSuppressClick()) return;
              setView(key);
              setSettingsOpen(false);
              setPopover(null);
              rail.leave();
            }}
            onRescan={() => {
              doScan();
              rail.leave();
            }}
            onToggleSettings={() => {
              setSettingsSection("prefs");
              setSettingsOpen((value) => !value);
              rail.leave();
            }}
            onEnter={rail.enter}
            onLeave={rail.leave}
            pointerHandlers={rail.pointerHandlers}
          />
        }
        resourcePane={
          paneCfg && (
            <ResourcePaneHost
              view={view}
              pane={paneCfg}
              collapsed={paneLayout.collapsed}
              width={paneLayout.width}
              resizing={paneLayout.resizing}
              filterOpen={popover === { library: "lib", history: "hist" }[view]}
              onOpenSearch={() => setSearchOpen(true)}
              onFilter={(e) => {
                popAnchor.current = e.currentTarget.getBoundingClientRect();
                setPopover((value) => {
                  const key = { library: "lib", history: "hist" }[view];
                  return value === key ? null : key;
                });
              }}
              library={{
                scanning,
                sessions,
                scanError: scan?.error || null,
                scanningLabel: t("app:detail.scanningSessions"),
                groups: libGroups,
                collapsedGroups,
                onToggleGroup,
                onClear: clearLibF,
                onRescan: doScan,
                selectedId: selId,
                multiSel,
                renamingKey: renameFor ? sessionIdentity(renameFor) : null,
                onRowClick,
                onRowPin,
                onRowDelete,
                onRowMore,
                onRowRename,
                onRowRenameSubmit,
                onRowRenameCancel,
              }}
              history={{
                groups: histGroups,
                filtered: histFiltered,
                onDelete: (id) =>
                  setHistDel(histItems.find((item) => item._id === id)),
                onClear: clearHistF,
              }}
              agent={{ sessions: ferrySessions }}
            />
          )
        }
        showDivider={Boolean(paneCfg && !paneLayout.collapsed)}
        resizing={paneLayout.resizing}
        onResizeStart={paneLayout.startResize}
        onResizeReset={paneLayout.resetWidth}
        dividerTitle={t("app:drag.hint")}
        toolbar={
          <WorkspaceToolbar
            paneAvailable={Boolean(paneCfg)}
            collapsed={paneLayout.collapsed}
            onToggleCollapsed={paneLayout.toggleCollapsed}
          />
        }
      >
        <WorkspaceRouter
          historyRows={historyRows}
          pricing={pricing}
          historySelection={histSel}
          agentAttachments={agentAttachments}
          onAgentAttachmentsChange={setAgentAttachments}
          optimization={optimization}
          onFirstDone={onboarding.completeFirstRun}
          scanningLabel={t("app:detail.scanningSessions")}
          emptyLibraryLabel={t("app:detail.noSessionToDisplay")}
        />
      </AppShell>

      <AppOverlayController t={t} />
    </div>
    </AppChromeProvider>
    </OperationsStateProvider>
    </BrowserStateProvider>
    </SessionEditingProvider>
    </FerryRuntimeProvider>
  );
}
