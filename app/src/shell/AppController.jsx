// Ferry 主壳:标题栏 / 导航轨 / 资源栏 / 详情区 + 全部弹层(按原型复刻)
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { TOOLS, TOOL_NAME } from "../shared/contracts/tools.js";
import { FerryRuntimeProvider } from "../shared/capabilities/ferryRuntime.jsx";
import { SessionEditingProvider } from "../shared/capabilities/sessionEditing.jsx";
import { BrowserStateProvider } from "../shared/capabilities/browserState.jsx";
import { OperationsStateProvider } from "../shared/capabilities/operationsState.jsx";
import { AppChromeProvider } from "../shared/capabilities/appChrome.jsx";
import { useIsFeatureEnabled } from "../shared/capabilities/features.jsx";
import {
  sessionIdentity,
  useBrowserData,
  useLibraryResourcePane,
  useSessionMetadata,
  useSessionSelection,
} from "../modules/browser/public.js";
import { useAskFerry } from "../modules/askferry/public.js";
import { UpdateBadge, useAppUpdater, useSettings } from "../modules/settings/public.js";
import { useSessionEditing } from "../modules/editing/public.js";
import { initialWorkspace, useOnboarding } from "../modules/onboarding/public.js";
import { useDesktopChrome } from "./useDesktopChrome.js";
import { AppNav } from "./AppNav.jsx";
import { WorkspaceToolbar } from "./WorkspaceToolbar.jsx";
import { AppShell } from "./AppShell.jsx";
import { AppOverlayController } from "./AppOverlayController.jsx";
import { WorkspaceRouter } from "./WorkspaceRouter.jsx";
import { ResourcePaneHost } from "./ResourcePaneHost.jsx";
import { useAppKeyboardShortcuts } from "./useAppKeyboardShortcuts.js";
import { useRailNavigation } from "./useRailNavigation.js";
import { useResourcePaneLayout } from "./useResourcePaneLayout.js";
import { useSidebarCollapse } from "./useSidebarCollapse.js";
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
  } = useBrowserData();

  const [view, setView] = useState(initialWorkspace);
  // 只锁存进程启动时是否真的进入过首启向导。完成向导会立即写入持久化标记，
  // 不能等切进主界面后再读；否则用户提前进入时也看不到首次索引的剩余进度。
  const firstRunSession = useRef(view === "firstrun").current;
  const [navigationTarget, setNavigationTarget] = useState(null);
  const [peekId, setPeekId] = useState(null); // Ask Ferry 卡片就地预览的会话 id
  const [floatChatOpen, setFloatChatOpen] = useState(false); // 会话库右下角浮动 Agent 面板

  const [mig, setMig] = useState(null); // {scope}

  const paneLayout = useResourcePaneLayout();
  const sidebar = useSidebarCollapse();

  const ferry = useAskFerry();
  const [agentAttachments, setAgentAttachments] = useState([]);
  const [settingsSection, setSettingsSection] = useState("prefs");
  const [aq, setAq] = useState("");

  const [searchOpen, setSearchOpen] = useState(false); // 搜索命令面板
  const [searchQuery, setSearchQuery] = useState("");
  const [ctxMenu, setCtxMenu] = useState(null); // {x, y, key, multi?}
  const [renameFor, setRenameFor] = useState(null); // 行内重命名中的会话
  const [tagFor, setTagFor] = useState(null); // {sessions} 待编辑标签的会话
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settings, setSettings] = useSettings();
  // 特性开关的事实源在宿主的配置文件里,界面据此决定入口显不显示。列表型入口
  // (导航轨、引导步骤)按表项上标的 feature 过滤,非列表点直接读这个判定。
  const isFeatureEnabled = useIsFeatureEnabled();
  const builtinAgent = isFeatureEnabled("builtin-agent");
  const updater = useAppUpdater(settings.autoCheckUpdates);
  // 助手关掉时正停在对话工作区:状态也跟着回落,否则导航轨没有高亮项、资源栏还
  // 挂着对话列表(路由层另有一层同样的回落,负责这一帧就不渲染它)
  useEffect(() => {
    if (!builtinAgent && view === "askferry") setView("overview");
  }, [builtinAgent, view]);
  const onboarding = useOnboarding({
    setView,
    closeSettings: () => setSettingsOpen(false),
    closeMigration: () => setMig(null),
    isFeatureEnabled,
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
    loadingMore,
    sessionsByKey: byKey,
    select,
    loadEntitySession,
    refreshDetail,
    loadMore,
  } = selection;
  const cur = selId ? byKey[selId] : null;
  const editing = useSessionEditing({
    current: cur,
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
    selectedKey: selId,
  });
  const {
    query: q,
    setQuery: setQ,
    scope: libScope,
    selectScope: selectLibScope,
    scopeLabel: libScopeLabel,
    scopeCount: libScopeCount,
    scopeCounts: libScopeCounts,
    display: libDisplay,
    setDisplay: setLibDisplay,
    displayDirty: libDisplayDirty,
    groupMode: libGroupMode,
    projects: libProjects,
    favorites: libFavorites,
    favoriteProjects: libFavoriteProjects,
    toggleFavorite: toggleLibFavorite,
    reorderFavorite: reorderLibFavorite,
    groups: libGroups,
    collapsedGroups,
    toggleGroup: onToggleGroup,
    visibleIds: libraryVisibleIds,
    index: libIndex,
    clear: clearLibF,
    multiIds: multiSel,
    setMultiIds: setMultiSel,
  } = library;
  // 范围单选:选中即切到会话页——它本来就是"我要看这一部分会话"的意思
  const pickLibraryScope = useCallback((next) => {
    selectLibScope(next);
    setView("library");
    setSettingsOpen(false);
  }, [selectLibScope]);
  // 首次扫描完成后默认选中第一个会话
  useEffect(() => {
    if (!selId && sessions.length) select(sessionIdentity(sessions[0]));
  }, [sessions]);

  // 打开设置并定位到指定分区:桌面菜单 / 路由 / 浮动面板共用
  const openConfig = (section = "providers") => {
    setSettingsSection(section);
    setSettingsOpen(true);
  };

  useDesktopChrome({
    onOpenSettings: () => openConfig("prefs"),
    onToggleSidebar: sidebar.toggle,
    onRescan: doScan,
  });

  useEffect(() => {
    if (!ferry.mutationVersion) return;
    doScan();
    loadHistory();
    if (view === "library" && selId) refreshDetail();
  }, [ferry.mutationVersion]);

  const {
    peekEntity,
    contextMenuItems: ctxItems,
    detailActions: detailActs,
    detailMeta,
    onRowClick,
    onRowMore,
    onRowPin,
    onRowRename,
    onRowRenameSubmit,
    onRowRenameCancel,
  } = useWorkspaceInteractions({
    t,
    settings, env, openConfig,
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
    editing,
    scope,
    loadMore,
    setToast,
    setView,
    setSettingsOpen,
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
    libraryTitle: libScopeLabel,
    libraryCount: libScopeCount,
    libraryDisplayDirty: libDisplayDirty,
  });
  const searchPane = useMemo(() => ({
    query: searchQuery,
    onQuery: event => setSearchQuery(event.target.value),
    placeholder: view === "askferry"
      ? t("askferry:pane.placeholder")
      : t("app:search.placeholder"),
  }), [searchQuery, view, t]);

  const rail = useRailNavigation({
    labels: {
      overview: t("app:rail.overview"),
      library: t("app:rail.library"),
      askferry: t("askferry:rail"),
    },
    storageKey: "ferry-rail-order",
    isFeatureEnabled,
  });

  useAppKeyboardShortcuts({
    paneAvailable: Boolean(paneCfg),
    onOpenSearch: () => setSearchOpen(true),
    onToggleSidebar: sidebar.toggle,
    onFocusPaneSearch: () => {
      if (view !== "library") return;
      document.dispatchEvent(new Event("ferry-focus-pane-search"));
    },
    dismissers: [
      { open: Boolean(ctxMenu), dismiss: () => setCtxMenu(null) },
      { open: Boolean(renameFor), dismiss: () => setRenameFor(null) },
      { open: Boolean(tagFor), dismiss: () => setTagFor(null) },
      { open: settingsOpen, dismiss: () => setSettingsOpen(false) },
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
    onRename: setRenameFor,
    onResume: detailActs.onResume,
    libraryVisibleIds,
    selectedSessionId: selId,
    selectSession: select,
  });




  const { browserState, operationsState, appChrome } = useWorkspaceState({
    applyEdit, confirmApply,
    ctxItems, ctxMenu, cur, detail, detailActs,
    detailMeta, diff, dirtyOps, doScan, env, ferrySessions: ferry.sessions, floatChatOpen,
    loadHistory, loadingMore, metaFor, mig, navigationTarget,
    onboarding, openConfig, peekEntity, peekId, rail,
    scan, scanning, searchOpen, searchPane, libIndex, select, selId,
    sessions, setConfirmApply, setCtxMenu, setDiff,
    setFloatChatOpen, setMetaFor, setMig,
    setMultiSel, setPeekId,
    setSearchOpen, setSettings, setSettingsOpen, setSettingsSection, setTagFor,
    setToast, setView,
    settings, settingsOpen, settingsSection, tagFor, toast, updater, view,
  });

  return (
    <FerryRuntimeProvider value={ferry}>
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
      {/* 首启向导独占整个窗口:导航栏、资源栏、工具条全部让位,只留标题栏拖拽区 */}
      <AppShell
        rail={view === "firstrun" ? null
          : <AppNav
            collapsed={sidebar.collapsed}
            items={rail.items}
            activeView={view}
            draggingKey={rail.draggingKey}
            dropTarget={rail.dropTarget}
            scanning={scanning}
            settingsOpen={settingsOpen}
            scope={libScope}
            scopeCounts={libScopeCounts}
            favoriteProjects={libFavoriteProjects}
            onReorderFavorite={reorderLibFavorite}
            onSelectScope={pickLibraryScope}
            labels={{
              pinned: t("app:library.pinned"),
              agents: t("app:nav.agents"),
              favorites: t("app:nav.favorites"),
              favoritesEmpty: t("app:nav.favoritesEmpty"),
              tags: t("app:nav.tags"),
              scanning: t("app:titlebar.scanning"),
              rescan: t("app:titlebar.rescan"),
              settings: t("app:rail.settings"),
              toolNames: TOOL_NAME,
            }}
            onSelect={(key) => {
              if (rail.shouldSuppressClick()) return;
              // 点「会话」= 回到全部:导航项本身也是一个范围
              if (key === "library") selectLibScope({ kind: "all" });
              setView(key);
              setSettingsOpen(false);
            }}
            onRescan={() => {
              doScan();
            }}
            onToggleSettings={() => {
              setSettingsSection("prefs");
              setSettingsOpen((value) => !value);
            }}
            settingsBadge={
              <UpdateBadge
                phase={updater.phase}
                version={updater.update?.version}
                progress={updater.progress}
                onStart={updater.startUpdate}
              />
            }
            pointerHandlers={rail.pointerHandlers}
          />
        }
        resourcePane={
          view !== "firstrun" && paneCfg && (
            <ResourcePaneHost
              view={view}
              pane={paneCfg}
              collapsed={sidebar.collapsed}
              width={paneLayout.width}
              resizing={paneLayout.resizing}
              onOpenSearch={() => setSearchOpen(true)}
              library={{
                scanning,
                scope: libScope,
                scopeCounts: libScopeCounts,
                projects: libProjects,
                favorites: libFavorites,
                onFavoriteProject: toggleLibFavorite,
                toolNames: TOOL_NAME,
                onSelectScope: pickLibraryScope,
                display: libDisplay,
                onDisplayChange: setLibDisplay,
                groupMode: libGroupMode,
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
                onRowMore,
                onRowRename,
                onRowRenameSubmit,
                onRowRenameCancel,
              }}
              agent={{ sessions: ferrySessions }}
            />
          )
        }
        showDivider={view !== "firstrun" && Boolean(paneCfg)}
        dividerCollapsed={sidebar.collapsed}
        resizing={paneLayout.resizing}
        onResizeStart={paneLayout.startResize}
        onResizeReset={paneLayout.resetWidth}
        dividerTitle={t("app:drag.hint")}
        sidebarCollapsed={view === "firstrun" ? true : sidebar.collapsed}
        toolbar={view === "firstrun" ? null : (
          <WorkspaceToolbar collapsed={sidebar.collapsed} onToggle={sidebar.toggle}
            showIndexProgress={firstRunSession} />
        )}
      >
        <WorkspaceRouter
          historyRows={historyRows}
          pricing={pricing}
          agentAttachments={agentAttachments}
          onAgentAttachmentsChange={setAgentAttachments}
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
