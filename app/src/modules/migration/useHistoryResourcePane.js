import { useCallback, useEffect, useMemo, useRef, useState } from "react";

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
  t,
  toolIds,
  toolNames,
}) {
  const items = useMemo(() => buildHistoryItems(historyRows), [historyRows]);
  const historicalToolIds = useMemo(
    () => [...new Set([
      ...toolIds,
      ...items.flatMap(item => [item.src, item.dst]).filter(Boolean),
    ])],
    [items, toolIds],
  );
  const previousToolIds = useRef(new Set(historicalToolIds));
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState(() => defaultFilter(historicalToolIds));
  const [selectedId, setSelectedId] = useState(null);
  useEffect(() => {
    const additions = historicalToolIds.filter(
      tool => !previousToolIds.current.has(tool),
    );
    previousToolIds.current = new Set(historicalToolIds);
    if (!additions.length) return;
    setFilter(value => ({
      ...value,
      src: [...new Set([...value.src, ...additions])],
    }));
  }, [historicalToolIds]);
  const filtered = useMemo(
    () => filterHistoryItems({ items, filter, query }),
    [items, filter, query],
  );
  const groups = useMemo(() => buildHistoryGroups({
    items: filtered, selectedId, t, toolNames,
  }).map(group => ({
    ...group,
    rows: group.rows.map(row => ({ ...row, onClick: () => setSelectedId(row.id) })),
  })), [filtered, selectedId, t, toolNames]);
  const selected = items.find(item => item._id === selectedId) || filtered[0] || null;
  const visibleIds = useMemo(() => filtered.map(item => item._id), [filtered]);
  const clear = useCallback(() => {
    setFilter(defaultFilter(historicalToolIds));
    setQuery("");
  }, [historicalToolIds]);
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
    selectedId,
    select: setSelectedId,
    visibleIds,
    toolIds: historicalToolIds,
    filterCount: historyFilterCount(filter, historicalToolIds),
    tokens,
    clear,
  };
}
