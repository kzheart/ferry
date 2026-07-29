// 三个领域 Context 的 value 组装。放在组合根之外,是为了让 AppController 只
// 保留"接线"本身;这里不引入任何新状态,只是把已有状态按域分组并 memo 化。
//
// value 必须 memo 化:下游详情区与资源栏都是 memo 组件,靠引用相等跳过重渲染。
import { useMemo } from "react";

export function useWorkspaceState({
  allTags, applyEdit, clearHistF, clearLibF, confirmApply,
  counts, ctxItems, ctxMenu, cur, deleteHistory, deletion, detail, detailActs,
  dirs,
  detailMeta, diff, dirtyOps, doScan, env, ferrySessions, floatChatOpen,
  histDel, histF, histGroups, histSel, histSelectedId, historyToolIds, libF,
  libGroups, loadHistory, loadingMore, metaFor, mig, navigationTarget,
  onboarding, openConfig, paneCfg, peekEntity, peekId, popAnchor,
  popover, rail, railOnly, refreshing, scan,
  scanning, searchOpen, select, selectHistory, selId, sessions,
  setConfirmApply, setCtxMenu, setDiff, setFloatChatOpen,
  setHistDel, setHistF, setLibF, setMetaFor, setMig, setMultiSel,
  setPeekId, setPopover, setSearchOpen,
  setSettings, setSettingsOpen, setTagFor, setToast, setView, settings,
  settingsOpen, settingsSection, tagFor, toast, updater, view,
}) {
  const browserState = useMemo(
    () => ({
      peek: {
        id: peekId,
        current: cur,
        selectedId: selId,
        meta: detailMeta,
        detail,
        actions: detailActs,
        navigationTarget,
        refreshing,
        loadingMore,
        setId: setPeekId,
        setView,
      },
      search: {
        open: searchOpen,
        pane: paneCfg,
        view,
        ferrySessions,
        historyGroups: histGroups,
        libraryGroups: libGroups,
        // 全文命中回来的是 opaque ref,要靠扫描列表换回选中用的 identity key
        scanSessions: sessions,
        selectHistory,
        setMultiSelection: setMultiSel,
        selectSession: select,
        setOpen: setSearchOpen,
      },
      contextMenu: {
        value: ctxMenu,
        items: ctxItems,
        setValue: setCtxMenu,
      },
      deletion: {
        sessionConfirmation: deletion.sessionConfirmation,
        batchConfirmation: deletion.batchConfirmation,
        cancelSessionDeletion: deletion.cancelSessionDeletion,
        confirmSessionDeletion: deletion.confirmSessionDeletion,
        cancelBatchDeletion: deletion.cancelBatchDeletion,
        confirmBatchDeletion: deletion.confirmBatchDeletion,
        history: histDel,
        setHistory: setHistDel,
        deleteHistory,
        selectedHistoryId: histSelectedId,
        selectHistory,
        onDeleteHistory: () => setHistDel(histSel),
      },
      tags: {
        selection: tagFor,
        setSelection: setTagFor,
        metaFor,
        updateMetadata: setMetaFor,
      },
      filters: {
        popover,
        anchor: popAnchor.current,
        onClose: () => setPopover(null),
        library: {
          value: libF,
          onChange: setLibF,
          counts,
          dirs,
          tags: allTags,
          onClear: clearLibF,
        },
        history: {
          value: histF,
          tools: historyToolIds,
          onChange: setHistF,
          onClear: clearHistF,
        },
      },
    }),
    [
      peekId, cur, selId, detailMeta, detail, detailActs, navigationTarget,
      refreshing, loadingMore, searchOpen, paneCfg, view, ferrySessions,
      histGroups, libGroups, sessions, selectHistory, select, ctxMenu,
      ctxItems, deletion, histDel, deleteHistory, histSelectedId, histSel,
      metaFor, setMetaFor, tagFor, popover, libF, counts, dirs,
      allTags, clearLibF, histF, historyToolIds, setHistF, clearHistF,
      setMultiSel, setLibF,
    ],
  );
  const operationsState = useMemo(
    () => ({
      migration: {
        state: mig,
        current: cur,
        env,
        settings,
        setState: setMig,
        loadHistory,
      },
      editing: {
        diff,
        dirtyOps,
        confirmApply,
        setDiff,
        setConfirmApply,
        apply: applyEdit,
      },
      floatChat: {
        // 会话库浮动 Agent 面板:仅在浏览具体会话且设置页未打开时挂载;
        // open 只控制展开与否,悬浮球本身由 mounted 决定
        mounted: view === "library" && Boolean(cur) && !settingsOpen,
        open: floatChatOpen,
        session: cur,
        scanSessions: sessions,
        onToggle: () => setFloatChatOpen((value) => !value),
        onNavigate: peekEntity,
        onOpenConfig: openConfig,
        onOpenFull: () => {
          setFloatChatOpen(false);
          setView("askferry");
        },
      },
    }),
    [
      sessions, mig, cur, env, settings,
      loadHistory, diff, dirtyOps, confirmApply, applyEdit, view, settingsOpen,
      floatChatOpen, peekEntity, openConfig, setDiff,
      setConfirmApply,
    ],
  );
  const appChrome = useMemo(
    () => ({
      toast: { value: toast, setValue: setToast },
      railTip: { value: rail.railTip, railOnly },
      settings: {
        open: settingsOpen,
        value: settings,
        onChange: setSettings,
        updater,
        section: settingsSection,
        scanResult: scan,
        env,
        scanning,
        scan: doScan,
        guideSeen: onboarding.seen,
        setOpen: setSettingsOpen,
        openGuide: onboarding.openGuide,
        setView,
      },
      guide: {
        step: onboarding.step,
        onGo: onboarding.goStep,
        onFinish: onboarding.finishGuide,
      },
    }),
    [
      toast, setToast, rail.railTip, railOnly, settingsOpen, settings,
      setSettings, updater, settingsSection, scan, env, scanning,
      doScan, onboarding,
    ],
  );

  return { browserState, operationsState, appChrome };
}
