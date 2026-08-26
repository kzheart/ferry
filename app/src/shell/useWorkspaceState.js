// 三个领域 Context 的 value 组装。放在组合根之外,是为了让 AppController 只
// 保留"接线"本身;这里不引入任何新状态,只是把已有状态按域分组并 memo 化。
//
// value 必须 memo 化:下游详情区与资源栏都是 memo 组件,靠引用相等跳过重渲染。
import { useMemo } from "react";

import { useFeature } from "../shared/capabilities/features.jsx";

export function useWorkspaceState({
  applyEdit, confirmApply,
  ctxItems, ctxMenu, cur, detail, detailActs,
  detailMeta, diff, dirtyOps, doScan, env, ferrySessions, floatChatOpen,
  loadHistory, loadingMore, metaFor, mig, navigationTarget,
  onboarding, openConfig, peekEntity, peekId, rail, scan,
  scanning, searchOpen, searchPane, libIndex, select, selId, sessions,
  setConfirmApply, setCtxMenu, setDiff, setFloatChatOpen,
  setMetaFor, setMig, setMultiSel,
  setPeekId, setSearchOpen,
  setSettings, setSettingsOpen, setSettingsSection, setTagFor, setToast, setView,
  settings, settingsOpen, settingsSection, tagFor, toast, updater, view,
}) {
  // 悬浮球是内置 AI 助手的一个入口:不是列表项,直接读开关。
  const builtinAgent = useFeature("builtin-agent");
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
        loadingMore,
        setId: setPeekId,
        setView,
      },
      search: {
        open: searchOpen,
        pane: searchPane,
        view,
        ferrySessions,
        libraryIndex: libIndex,
        // 全文命中回来的是 opaque ref,要靠扫描列表换回选中用的 identity key
        scanSessions: sessions,
        setMultiSelection: setMultiSel,
        selectSession: select,
        setOpen: setSearchOpen,
      },
      contextMenu: {
        value: ctxMenu,
        items: ctxItems,
        setValue: setCtxMenu,
      },
      tags: {
        selection: tagFor,
        setSelection: setTagFor,
        metaFor,
        updateMetadata: setMetaFor,
      },
    }),
    [
      peekId, cur, selId, detailMeta, detail, detailActs, navigationTarget,
      loadingMore, searchOpen, searchPane, view, ferrySessions,
      libIndex, sessions, select, ctxMenu,
      ctxItems,
      metaFor, setMetaFor, tagFor,
      setMultiSel,
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
        openConfig,
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
        // open 只控制展开与否,悬浮球本身由 mounted 决定。它也是内置 AI 助手的
        // 一个入口,特性关着时连悬浮球都不出现
        mounted:
          builtinAgent && view === "library" && Boolean(cur) && !settingsOpen,
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
      sessions, mig, cur, env, settings, builtinAgent,
      loadHistory, diff, dirtyOps, confirmApply, applyEdit, view, settingsOpen,
      floatChatOpen, peekEntity, openConfig, setDiff,
      setConfirmApply,
    ],
  );
  const appChrome = useMemo(
    () => ({
      toast: { value: toast, setValue: setToast },
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
        setSection: setSettingsSection,
        openGuide: onboarding.openGuide,
        setView,
      },
      guide: {
        step: onboarding.step,
        steps: onboarding.steps,
        onGo: onboarding.goStep,
        onFinish: onboarding.finishGuide,
      },
    }),
    [
      toast, setToast, settingsOpen, settings,
      setSettings, updater, settingsSection, setSettingsSection, scan, env, scanning,
      doScan, onboarding,
    ],
  );

  return { browserState, operationsState, appChrome };
}
