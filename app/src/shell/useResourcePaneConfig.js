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
}) {
  const needle = agentQuery.trim().toLowerCase();
  const ferrySessions = useMemo(
    () =>
      (needle
        ? ferry.sessions.filter((s) =>
            (s.title || "").toLowerCase().includes(needle),
          )
        : ferry.sessions
      )
        .slice()
        .sort(
          (left, right) =>
            Number(!!right.pinned) - Number(!!left.pinned) ||
            String(right.updated_at || "").localeCompare(
              String(left.updated_at || ""),
            ),
        ),
    [ferry.sessions, needle],
  );

  const paneConfig =
    {
      askferry: {
        title: t("askferry:pane.title"),
        count: String(ferry.sessions.length),
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
    }[view] || null;

  return { paneConfig, ferrySessions };
}
