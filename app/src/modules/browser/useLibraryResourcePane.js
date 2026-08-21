import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import {
  DEFAULT_DISPLAY,
  DEFAULT_SCOPE,
  buildLibraryGroups,
  buildLibraryIndex,
  displayDirtyCount,
  effectiveGroupMode,
  favoriteProjectRows,
  FAVORITE_PROJECTS_KEY,
  libraryProjects,
  libraryScopeCounts,
  normalizeFavoriteProjects,
  reorderFavoriteProjects,
  seedFavoriteProjects,
  toggleFavoriteProject,
  migrateLegacyLibraryState,
  normalizeDisplay,
  normalizeScope,
  sameScope,
  scopeLabel,
  selectedProjectGroupKey,
  visibleLibraryIds,
} from "./libraryResourcePaneModel.js";

const SCOPE_KEY = "ferry-library-scope";
const DISPLAY_KEY = "ferry-library-display";
const LEGACY_VIEW_KEY = "ferry-library-view";

function readJson(key) {
  try {
    return JSON.parse(localStorage.getItem(key) || "null");
  } catch {
    return null;
  }
}

function writeJson(key, value) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // 隐私模式下写不进去,不影响本次会话
  }
}

/**
 * 首次读盘:没有新键就从旧的「时间 / 项目」视图分段迁移一次。
 *
 * 旧的六维筛选状态本来就只活在内存里(从没落过盘),所以迁移函数在这里只拿得到
 * view;它仍接受完整的 filter,是为了让映射规则本身可被直接断言。
 */
function readPersisted(toolIds) {
  const scope = readJson(SCOPE_KEY);
  const display = readJson(DISPLAY_KEY);
  if (scope || display) {
    return { scope: normalizeScope(scope), display: normalizeDisplay(display) };
  }
  let view = null;
  try {
    view = localStorage.getItem(LEGACY_VIEW_KEY);
  } catch {
    view = null;
  }
  const migrated = migrateLegacyLibraryState({ filter: null, view, toolIds });
  try {
    localStorage.removeItem(LEGACY_VIEW_KEY);
  } catch {
    // 清不掉也无所谓:新键一旦写出去,旧键就不会再被读到
  }
  return migrated;
}

/**
 * 收藏的项目。
 *
 * 存储键完全独立于会话元数据(理由见 libraryResourcePaneModel 里的注释)。
 * 「没有这个键」和「这个键是空数组」是两回事:前者是首次运行,要自动收藏最近
 * 活跃的 3 个;后者是用户把收藏全取消了,不能再自动塞回去。
 */
function useFavoriteProjects(projects) {
  const [favorites, setFavorites] = useState(
    () => normalizeFavoriteProjects(readJson(FAVORITE_PROJECTS_KEY)),
  );
  // 首启种子只撒一次:等到第一次真的扫出项目才动手,否则会写下一个空数组,
  // 把「没记录」变成「用户清空过」,种子就再也撒不下去了。
  const seeded = useRef(readJson(FAVORITE_PROJECTS_KEY) !== null);
  useEffect(() => {
    if (seeded.current || !projects.length) return;
    seeded.current = true;
    const seed = seedFavoriteProjects(projects);
    setFavorites(seed);
    writeJson(FAVORITE_PROJECTS_KEY, seed);
  }, [projects]);

  const persist = useCallback(next => {
    setFavorites(next);
    writeJson(FAVORITE_PROJECTS_KEY, next);
    // 用户手动动过收藏之后就不再需要种子了
    seeded.current = true;
  }, []);

  return {
    favorites,
    toggleFavorite: useCallback(
      dir => persist(toggleFavoriteProject(favorites, dir)), [favorites, persist]),
    reorderFavorite: useCallback(
      (dir, targetDir, position) =>
        persist(reorderFavoriteProjects(favorites, dir, targetDir, position)),
      [favorites, persist]),
  };
}

/**
 * 会话库资源栏的本地 UI 状态与纯展示投影。
 *
 * 写入、选择会话和上下文菜单仍归 App 协调；此 Hook 只维护搜索、范围、显示选项、
 * 分组折叠和多选状态,以及导航栏范围区要用的那份计数。
 */
export function useLibraryResourcePane({
  sessions,
  metadata,
  migratedSessionKeys,
  t,
  toolIds,
  toolNames,
  selectedKey = null,
}) {
  const [query, setQuery] = useState("");
  const [multiIds, setMultiIds] = useState([]);
  const [collapsedGroups, setCollapsedGroups] = useState({ earlier: true });
  const persisted = useRef(null);
  if (persisted.current === null) persisted.current = readPersisted(toolIds);
  const [scope, setScopeState] = useState(persisted.current.scope);
  const [display, setDisplayState] = useState(persisted.current.display);

  useEffect(() => { writeJson(SCOPE_KEY, scope); }, [scope]);
  useEffect(() => { writeJson(DISPLAY_KEY, display); }, [display]);

  const projects = useMemo(() => libraryProjects(sessions), [sessions]);
  const { favorites, toggleFavorite, reorderFavorite } = useFavoriteProjects(projects);
  const favoriteProjects = useMemo(
    () => favoriteProjectRows(projects, favorites), [projects, favorites]);
  const index = useMemo(() => buildLibraryIndex({
    sessions, metadata, migratedSessionKeys, t,
  }), [sessions, metadata, migratedSessionKeys, t]);
  // 计数取自全量索引:显示选项不该让导航栏的数字跳动
  const scopeCounts = useMemo(
    () => libraryScopeCounts(index, toolIds),
    [index, toolIds],
  );
  const groups = useMemo(() => buildLibraryGroups({
    index, scope, display, query, t,
  }), [index, scope, display, query, t]);
  const groupMode = effectiveGroupMode(scope, display);
  // 项目文件夹默认折叠;进入项目视图时把当前选中会话所在的文件夹展开一次,
  // 之后完全交给用户——重复自动展开会把他手动折叠的文件夹又顶开。
  const autoExpanded = useRef(false);
  const selectedKeyRef = useRef(selectedKey);
  selectedKeyRef.current = selectedKey;
  useEffect(() => {
    if (groupMode !== "project") { autoExpanded.current = false; return; }
    if (autoExpanded.current) return;
    if (!groups.some(group => group.kind === "project")) return; // 会话还没扫出来
    autoExpanded.current = true;
    const key = selectedProjectGroupKey(groups, selectedKeyRef.current);
    if (!key) return;
    setCollapsedGroups(value => (key in value ? value : { ...value, [key]: false }));
  }, [groupMode, groups]);

  const visibleIds = useMemo(
    () => visibleLibraryIds(groups, collapsedGroups, query),
    [groups, collapsedGroups, query],
  );

  // 折叠默认值按分组类型分开(项目文件夹默认折叠),第一次点击必须真的翻转当前状态
  const toggleGroup = useCallback((key, kind) => {
    setCollapsedGroups(value => ({ ...value, [key]: !(value[key] ?? kind === "project") }));
  }, []);
  // 范围单选互斥:再点一次当前范围就回到「全部」——对齐 Wake 侧栏的手感
  const selectScope = useCallback(next => {
    const target = normalizeScope(next);
    setScopeState(value => (sameScope(value, target) ? DEFAULT_SCOPE : target));
  }, []);
  const setDisplay = useCallback(patch => {
    setDisplayState(value => normalizeDisplay({ ...value, ...patch }));
  }, []);
  const resetDisplay = useCallback(() => setDisplayState(DEFAULT_DISPLAY), []);
  const clear = useCallback(() => {
    setScopeState(DEFAULT_SCOPE);
    setDisplayState(DEFAULT_DISPLAY);
    setQuery("");
  }, []);

  const currentScopeLabel = scopeLabel(scope, { t, toolNames });
  const scopeCount = useMemo(() => {
    switch (scope.kind) {
      case "pinned": return scopeCounts.pinned;
      case "agent":
        return scopeCounts.agents.find(item => item.tool === scope.value)?.count || 0;
      case "project":
        return projects.find(item => item.dir === scope.value)?.count || 0;
      case "tag":
        return scopeCounts.tags.find(item => item.tag === scope.value)?.count || 0;
      default: return scopeCounts.total;
    }
  }, [scope, scopeCounts, projects]);

  return {
    query,
    setQuery,
    scope,
    selectScope,
    scopeLabel: currentScopeLabel,
    scopeCount,
    scopeCounts,
    display,
    setDisplay,
    resetDisplay,
    displayDirty: displayDirtyCount(display),
    groupMode,
    projects,
    favorites,
    favoriteProjects,
    toggleFavorite,
    reorderFavorite,
    groups,
    collapsedGroups,
    toggleGroup,
    visibleIds,
    clear,
    multiIds,
    setMultiIds,
  };
}
