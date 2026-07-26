// 资源栏骨架配置:各工作区的标题、计数、搜索框与页脚文案。
// 新增工作区在此登记一条,不必改动主壳的渲染。
import { useMemo } from "react";
import { fmtTime } from "../modules/browser/public.js";

export function useResourcePaneConfig({
  t,
  view,
  ferry,
  agentQuery,
  setAgentQuery,
  sessions,
  scan,
  lastScan,
  libraryQuery,
  setLibraryQuery,
  libraryFilterCount,
  libraryTokens,
  multiIds,
  historyItems,
  historyQuery,
  setHistoryQuery,
  historyFilterCount,
  historyTokens,
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
        tokens: [],
        footer: t("askferry:pane.footer", { n: ferry.sessions.length }),
      },
      library: {
        title: t("app:pane.libraryTitle"),
        count: String(sessions.length),
        placeholder: t("app:pane.libraryPlaceholder"),
        query: libraryQuery,
        onQuery: (e) => setLibraryQuery(e.target.value),
        filterCount: libraryFilterCount,
        tokens: libraryTokens,
        footer: scan?.error
          ? t("app:pane.libraryFooterError", { error: scan.error })
          : multiIds.length > 1
            ? t("app:pane.libraryFooterMulti", { n: multiIds.length })
            : t("app:pane.libraryFooterBrowsing", {
                n: sessions.length,
                lastScan: lastScan
                  ? t("app:pane.libraryFooterLastScan", {
                      time: fmtTime(lastScan, t),
                    })
                  : "",
              }),
      },
      history: {
        title: t("app:pane.historyTitle"),
        count: String(historyItems.length),
        placeholder: t("app:pane.historyPlaceholder"),
        query: historyQuery,
        onQuery: (e) => setHistoryQuery(e.target.value),
        filterCount: historyFilterCount,
        tokens: historyTokens,
        footer: t("app:pane.historyFooter", { n: historyItems.length }),
      },
    }[view] || null;

  return { paneConfig, ferrySessions };
}
