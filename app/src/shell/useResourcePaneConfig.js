// 资源栏骨架配置:各工作区的标题、计数与搜索框文案。
// 新增工作区在此登记一条,不必改动主壳的渲染。
import { useMemo } from "react";

export function useResourcePaneConfig({
  t,
  view,
  ferry,
  agentQuery,
  setAgentQuery,
  libraryQuery,
  setLibraryQuery,
  libraryTitle,
  libraryCount,
  libraryDisplayDirty,
  historyItems,
  historyQuery,
  setHistoryQuery,
  historyFilterCount,
  historyTokens,
}) {
  const needle = agentQuery.trim().toLowerCase();
  // 优化用途的会话是浏览界面魔法棒的幕后执行体,跑完即删,不进对话列表
  const visibleSessions = useMemo(
    () => ferry.sessions.filter((s) => s.purpose !== "session-optimization"),
    [ferry.sessions],
  );
  const ferrySessions = useMemo(
    () =>
      (needle
        ? visibleSessions.filter((s) =>
            (s.title || "").toLowerCase().includes(needle),
          )
        : visibleSessions
      )
        .slice()
        .sort(
          (left, right) =>
            Number(!!right.pinned) - Number(!!left.pinned) ||
            String(right.updated_at || "").localeCompare(
              String(left.updated_at || ""),
            ),
        ),
    [visibleSessions, needle],
  );

  const paneConfig =
    {
      askferry: {
        title: t("askferry:pane.title"),
        count: String(visibleSessions.length),
        placeholder: t("askferry:pane.placeholder"),
        query: agentQuery,
        onQuery: (e) => setAgentQuery(e.target.value),
        filterCount: 0,
      },
      library: {
        title: libraryTitle,
        count: String(libraryCount),
        placeholder: t("app:pane.libraryPlaceholder"),
        query: libraryQuery,
        onQuery: (e) => setLibraryQuery(e.target.value),
        filterCount: libraryDisplayDirty,
        displayLabel: t("app:display.menu"),
      },
      history: {
        title: t("app:pane.historyTitle"),
        count: String(historyItems.length),
        placeholder: t("app:pane.historyPlaceholder"),
        query: historyQuery,
        onQuery: (e) => setHistoryQuery(e.target.value),
        filterCount: historyFilterCount,
        tokens: historyTokens,
        displayLabel: t("app:pane.filterButton"),
      },
    }[view] || null;

  return { paneConfig, ferrySessions };
}
