import { useCallback, useMemo, useState } from "react";

import {
  buildLibraryGroups,
  buildLibraryIndex,
  libraryDirectories,
  libraryFilterCount,
  libraryTags,
  libraryTokenDescriptors,
  libraryToolCounts,
  visibleLibraryIds,
} from "./libraryResourcePaneModel.js";

function defaultFilter(toolIds) {
  return { src: [...toolIds], time: "all", dir: null, mig: false, sub: false, tag: null };
}

/**
 * 会话库资源栏的本地 UI 状态与纯展示投影。
 *
 * 写入、选择会话和上下文菜单仍归 App 协调；此 Hook 只维护资源栏本身的
 * 搜索、筛选、分组折叠和多选状态，避免它们继续散落在应用壳中。
 */
export function useLibraryResourcePane({
  sessions,
  metadata,
  migratedSessionKeys,
  t,
  toolIds,
  toolNames,
}) {
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState(() => defaultFilter(toolIds));
  const [multiIds, setMultiIds] = useState([]);
  const [collapsedGroups, setCollapsedGroups] = useState({ earlier: true });

  const counts = useMemo(() => libraryToolCounts(sessions), [sessions]);
  const dirs = useMemo(() => libraryDirectories(sessions), [sessions]);
  const tags = useMemo(() => libraryTags(metadata), [metadata]);
  const index = useMemo(() => buildLibraryIndex({
    sessions, metadata, migratedSessionKeys, t,
  }), [sessions, metadata, migratedSessionKeys, t]);
  const groups = useMemo(() => buildLibraryGroups({
    index, filter, query, t,
  }), [index, filter, query, t]);
  const visibleIds = useMemo(
    () => visibleLibraryIds(groups, collapsedGroups),
    [groups, collapsedGroups],
  );
  const filterCount = libraryFilterCount(filter, toolIds);

  const toggleGroup = useCallback(key => {
    setCollapsedGroups(value => ({ ...value, [key]: !(value[key] ?? false) }));
  }, []);
  const clear = useCallback(() => {
    setFilter(defaultFilter(toolIds));
    setQuery("");
  }, [toolIds]);
  const tokens = useMemo(() => libraryTokenDescriptors(filter, toolIds, toolNames, t).map(token => ({
    label: token.label,
    onRemove: () => {
      if (token.kind === "source") {
        setFilter(value => {
          const src = value.src.filter(tool => tool !== token.tool);
          return { ...value, src: src.length ? src : [...toolIds] };
        });
      } else if (token.kind === "time") setFilter(value => ({ ...value, time: "all" }));
      else if (token.kind === "dir") setFilter(value => ({ ...value, dir: null }));
      else if (token.kind === "mig") setFilter(value => ({ ...value, mig: false }));
      else if (token.kind === "sub") setFilter(value => ({ ...value, sub: false }));
      else if (token.kind === "tag") setFilter(value => ({ ...value, tag: null }));
    },
  })), [filter, t, toolIds, toolNames]);

  return {
    query,
    setQuery,
    filter,
    setFilter,
    counts,
    dirs,
    tags,
    groups,
    collapsedGroups,
    toggleGroup,
    visibleIds,
    filterCount,
    tokens,
    clear,
    multiIds,
    setMultiIds,
  };
}
