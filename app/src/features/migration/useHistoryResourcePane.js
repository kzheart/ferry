import { useCallback, useMemo, useState } from "react";

import {
  buildHistoryGroups,
  buildHistoryItems,
  filterHistoryItems,
  historyFilterCount,
  historyTokenDescriptors,
} from "./historyResourcePaneModel.js";

function defaultFilter(toolIds) {
  return { src: [...toolIds], target: "all", status: "all", time: "all" };
}

/** 迁移历史资源栏的本地状态与展示投影。 */
export function useHistoryResourcePane({
  historyRows,
  selectedId,
  onSelect,
  t,
  toolIds,
  toolNames,
}) {
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState(() => defaultFilter(toolIds));
  const items = useMemo(() => buildHistoryItems(historyRows), [historyRows]);
  const filtered = useMemo(
    () => filterHistoryItems({ items, filter, query }),
    [items, filter, query],
  );
  const groups = useMemo(() => buildHistoryGroups({
    items: filtered, selectedId, t, toolNames,
  }).map(group => ({
    ...group,
    rows: group.rows.map(row => ({ ...row, onClick: () => onSelect(row.id) })),
  })), [filtered, onSelect, selectedId, t, toolNames]);
  const selected = items.find(item => item._id === selectedId) || filtered[0] || null;
  const visibleIds = useMemo(() => filtered.map(item => item._id), [filtered]);
  const clear = useCallback(() => {
    setFilter(defaultFilter(toolIds));
    setQuery("");
  }, [toolIds]);
  const tokens = useMemo(() => historyTokenDescriptors(filter, toolNames, t).map(token => ({
    label: token.label,
    onRemove: () => setFilter(value => ({ ...value, [token.kind]: "all" })),
  })), [filter, t, toolNames]);

  return {
    query,
    setQuery,
    filter,
    setFilter,
    items,
    filtered,
    groups,
    selected,
    visibleIds,
    filterCount: historyFilterCount(filter, toolIds),
    tokens,
    clear,
  };
}
