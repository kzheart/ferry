// 工作区交互回调:上下文菜单、资源栏行为、详情区动作、卡片预览导航。
// 与渲染分离,新增交互不必改动主壳的 JSX。
import { useMemo, useRef } from "react";
import { openTerminal } from "../platform/desktop/client.js";
import {
  resumeDescriptor,
  supportsAgentCapability,
  supportsSessionResumeCli,
} from "../shared/contracts/tools.js";
import {
  createSessionContextMenu,
  sessionRef,
  useLibraryResourcePaneActions,
} from "../modules/browser/public.js";
import { copyResumeCommand, copyResumeInstruction } from "../modules/migration/public.js";
import { useIsFeatureEnabled } from "../shared/capabilities/features.jsx";

export function useWorkspaceInteractions({
  t,
  settings,
  env,
  current,
  selectedId,
  sessionsByKey,
  metadata,
  metaFor,
  updateMetadata,
  multiIds,
  setMultiIds,
  libraryVisibleIds,
  menu,
  setMenu,
  select,
  loadEntitySession,
  deletion,
  editing,
  scope,
  loadMore,
  setToast,
  openConfig,
  setView,
  setSettingsOpen,
  setPopover,
  setNavigationTarget,
  setPeekId,
  setMigration,
  setRename,
  setTagSelection,
  setAgentAttachments,
}) {
  // 右键菜单是列表型入口:开关状态直接在这里取,不必由主壳层层转交。
  const isFeatureEnabled = useIsFeatureEnabled();
  const askDelete = session => {
    if (!supportsAgentCapability(session?.tool, "delete")) return;
    deletion.requestSessionDeletion(session);
  };
  const askBatchDelete = sessions => {
    if (!sessions.every(session =>
      supportsAgentCapability(session?.tool, "delete"))) return;
    deletion.requestBatchDeletion(sessions);
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

  const contextMenuItems = createSessionContextMenu({
    menu,
    sessionsByKey,
    selectedId,
    multiIds,
    metaFor,
    updateMetadata,
    setTagSelection,
    setRename,
    setBatchDelete: askBatchDelete,
    setMultiIds,
    setAgentAttachments,
    setView,
    setMenu,
    setToast,
    select,
    setMigration,
    // 只复制一条指令:接手由目标 agent 的 ferry-resume skill 自己完成。
    onResumeElsewhere: session => copyResumeCommand({
      tool: session.tool,
      sessionId: session.id,
      t,
      setToast,
      openConfig,
    }),
    settings,
    isFeatureEnabled,
    t,
    askDelete,
  });

  const rowActions = useLibraryResourcePaneActions({
    sessionsByKey,
    selectedId,
    visibleIds: libraryVisibleIds,
    multiIds,
    setMultiIds,
    onSelect: select,
    onTogglePin: (session) =>
      updateMetadata(session, { pinned: !metaFor(session).pinned }),
    onDelete: askDelete,
    onOpenMenu: setMenu,
    onStartRename: setRename,
    // 留空恢复原始标题(与旧弹窗语义一致);未改动则不写元数据
    onSubmitRename: (session, value) => {
      setRename(null);
      const name = value.trim();
      if (name === (metaFor(session).name || session.title || "")) return;
      updateMetadata(session, { name });
    },
    onCancelRename: () => setRename(null),
  });

  // 详情区回调:经 ref 转发保持身份稳定,memo 化的 SessionDetail 才不会
  // 因侧边栏交互(展开分组/多选/悬停)产生的新闭包全量重渲染整条时间线
  const detailFns = useRef({});
  detailFns.current = {
    discardAll: () => editing.setOps([]),
    setScope: editing.setScope,
    addOp: editing.addOp,
    removeOp: editing.removeOp,
    updateOp: editing.updateOp,
    startReplyEdit: editing.startReplyEdit,
    replyEditError: editing.replyEditError,
    openDiff: editing.openDiff,
    apply: () => {
      if (supportsAgentCapability(current?.tool, "edit")) {
        editing.prepareApply();
      }
    },
    openMigrate: (value) => {
      if (supportsAgentCapability(current?.tool, "migration-source")) {
        setMigration({ scope: value ?? scope });
      }
    },
    loadMore,
    // 与右键菜单同一条指令,但不走 toast:按钮自己有反馈位,结果画在按钮上
    resumeElsewhere: (meta) => copyResumeInstruction({
      tool: meta.tool,
      sessionId: meta.id,
    }),
    resume: async (meta) => {
      if (!supportsSessionResumeCli(meta?.tool)) return;
      setToast({
        kind: "run",
        title: t("app:toast.openingTerminal"),
        desc: t("app:toast.openingTerminalDesc", {
          title: meta.title || meta.id,
        }),
      });
      try {
        const launch = await resumeDescriptor(meta.tool, sessionRef(meta));
        await openTerminal(launch, settings.terminalApp);
        setToast({
          kind: "ok",
          title: t("app:toast.terminalOpened"),
          desc: t("app:toast.terminalOpenedDesc"),
        });
      } catch (error) {
        setToast({
          kind: "fail",
          title: t("app:toast.openTerminalFail"),
          desc: error.message,
        });
      }
    },
  };

  const detailActions = useMemo(
    () => ({
      onDiscardAll: () => detailFns.current.discardAll(),
      setScope: (v) => detailFns.current.setScope(v),
      addOp: (...a) => detailFns.current.addOp(...a),
      removeOp: (...a) => detailFns.current.removeOp(...a),
      updateOp: (...a) => detailFns.current.updateOp(...a),
      startReplyEdit: (...a) => detailFns.current.startReplyEdit(...a),
      replyEditError: (...a) => detailFns.current.replyEditError(...a),
      onOpenDiff: () => detailFns.current.openDiff(),
      onApply: () => detailFns.current.apply(),
      onOpenMigrate: (sc) => detailFns.current.openMigrate(sc),
      onLoadMore: () => detailFns.current.loadMore(),
      onResume: (meta) => detailFns.current.resume(meta),
      onResumeElsewhere: (meta) => detailFns.current.resumeElsewhere(meta),
    }),
    [],
  );

  const detailMeta = useMemo(
    () =>
      current && metaFor(current).name
        ? { ...current, title: metaFor(current).name }
        : current,
    [current, metadata],
  );

  return {
    askDelete,
    peekEntity,
    contextMenuItems,
    detailActions,
    detailMeta,
    ...rowActions,
  };
}
