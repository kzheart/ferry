import { bucketOf, fmtTime } from "../browser/sessionModel.js";
import { histStatus, STATUS_CODE } from "./historyStatus.js";

const HISTORY_GROUPS = [
  ["today", "app:historyToken.today"],
  ["yesterday", "app:historyToken.yesterday"],
  ["earlier", "app:historyToken.earlier"],
];

export function buildHistoryItems(historyRows) {
  return historyRows.map((history, index) => ({
    ...history,
    _id: history.id ? `h${history.id}` : `h${index}-${history.time}`,
    status: histStatus(history),
  }));
}

export function filterHistoryItems({ items, filter, query }) {
  const needle = query.trim().toLowerCase();
  return items.filter(history => filter.src.includes(history.src)
    && (filter.target === "all" || history.dst === filter.target)
    && (filter.status === "all" || history.status === filter.status)
    && (filter.time === "all" || bucketOf(history.time) === filter.time
      || (filter.time === "earlier"
        && !["today", "yesterday"].includes(bucketOf(history.time))))
    && (!needle || (history.title || "").toLowerCase().includes(needle)
      || (history.session_id || "").toLowerCase().includes(needle)));
}

export function buildHistoryGroups({ items, selectedId, t, toolNames }) {
  const defaultId = items[0]?._id;
  return HISTORY_GROUPS.map(([key, label]) => ({
    label: t(label),
    rows: items.filter(history => key === "earlier"
      ? !["today", "yesterday"].includes(bucketOf(history.time))
      : bucketOf(history.time) === key)
      .map(history => ({
        id: history._id,
        title: history.title || history.source_id,
        short: fmtTime(history.time, t),
        from: toolNames[history.src],
        to: toolNames[history.dst],
        status: history.status,
        statusLabel: t(`common:${history.status}`),
        stColor: {
          [STATUS_CODE.success]: "var(--ok)",
          [STATUS_CODE.failed]: "var(--err)",
          [STATUS_CODE.rolledBack]: "var(--tx3b)",
        }[history.status],
        tool: history.src,
        selected: history._id === (selectedId ?? defaultId),
        deletable: !!history.id,
      })),
  })).filter(group => group.rows.length);
}

export function historyFilterCount(filter, toolIds) {
  return (filter.src.length < toolIds.length ? 1 : 0)
    + (filter.target !== "all" ? 1 : 0)
    + (filter.status !== "all" ? 1 : 0)
    + (filter.time !== "all" ? 1 : 0);
}

export function historyTokenDescriptors(filter, toolNames, t) {
  const tokens = [];
  if (filter.target !== "all") {
    tokens.push({ kind: "target", label: t("app:historyToken.target", { tool: toolNames[filter.target] }) });
  }
  if (filter.status !== "all") {
    tokens.push({ kind: "status", label: t(`common:${filter.status}`) });
  }
  if (filter.time !== "all") {
    tokens.push({ kind: "time", label: t(`app:historyToken.${filter.time}`) });
  }
  return tokens;
}
