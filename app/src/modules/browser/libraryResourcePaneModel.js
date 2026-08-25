import {
  BUCKETS,
  bucketOf,
  fmtTime,
  repoOf,
} from "./sessionModel.js";
import { sessionIdentity } from "./sessionAttachment.js";

// 项目路径的父目录:同名仓库(多处 clone / monorepo 子包)只能靠它区分
export function parentOf(dir) {
  if (!dir) return "";
  const parts = String(dir).split("/").filter(Boolean);
  parts.pop();
  if (!parts.length) return "";
  return `${String(dir).startsWith("/") ? "/" : ""}${parts.join("/")}`;
}

/**
 * 导航栏「项目」分区与项目文件夹树共用的项目清单。
 *
 * 以完整 dir 为主键(仓库名会撞车),按最近活跃倒序;仓库名重复时标记
 * ambiguous——导航栏不再做视觉消歧,完整路径挂在行的 title 上。
 */
export function libraryProjects(sessions) {
  const byDir = new Map();
  for (const session of sessions) {
    const dir = session.dir;
    if (!dir) continue;
    const current = byDir.get(dir);
    if (current) {
      current.count += 1;
      current.updated = Math.max(current.updated, session.updated || 0);
      if (!current.tools.includes(session.tool)) current.tools.push(session.tool);
    } else {
      byDir.set(dir, {
        dir,
        repo: repoOf(dir),
        parent: parentOf(dir),
        count: 1,
        updated: session.updated || 0,
        tools: [session.tool],
        ambiguous: false,
      });
    }
  }
  const projects = [...byDir.values()].sort((left, right) => right.updated - left.updated);
  const repoCounts = {};
  projects.forEach(project => {
    repoCounts[project.repo] = (repoCounts[project.repo] || 0) + 1;
  });
  projects.forEach(project => { project.ambiguous = repoCounts[project.repo] > 1; });
  return projects;
}

// ---------------------------------------------------------------------------
// 收藏的项目(导航栏「收藏」分区)
//
// 存的是完整 dir 数组,顺序即展示顺序(可拖拽)。不进会话元数据:引擎的
// session_meta_* 一整套都按「会话」为键,项目收藏没有对应的会话可挂,
// 塞进去要么污染某条会话的元数据,要么得给引擎加一张新表——两者都不值当。
// 所以单开一个 localStorage 键 ferry-favorite-projects。
// ---------------------------------------------------------------------------

export const FAVORITE_PROJECTS_KEY = "ferry-favorite-projects";
// 首次运行自动收藏最近活跃的几个,免得导航栏一上来就是空的
export const FAVORITE_SEED_COUNT = 3;

export function normalizeFavoriteProjects(value) {
  if (!Array.isArray(value)) return [];
  const seen = new Set();
  const out = [];
  for (const item of value) {
    const dir = typeof item === "string" ? item : "";
    if (!dir || seen.has(dir)) continue;
    seen.add(dir);
    out.push(dir);
  }
  return out;
}

/** 首启种子:按最近活跃取前 N 个项目的 dir。 */
export function seedFavoriteProjects(projects, count = FAVORITE_SEED_COUNT) {
  return projects.slice(0, count).map(project => project.dir);
}

export function toggleFavoriteProject(favorites, dir) {
  if (!dir) return favorites;
  return favorites.includes(dir)
    ? favorites.filter(item => item !== dir)
    : [...favorites, dir];
}

/**
 * 拖拽排序:把 dir 挪到 targetDir 之前/之后。
 * 目标不在列表里(或就是自己)时原样返回,不让一次误拖把顺序打乱。
 */
export function reorderFavoriteProjects(favorites, dir, targetDir, position = "before") {
  if (!dir || dir === targetDir) return favorites;
  if (!favorites.includes(dir) || !favorites.includes(targetDir)) return favorites;
  const rest = favorites.filter(item => item !== dir);
  const at = rest.indexOf(targetDir);
  if (at < 0) return favorites;
  rest.splice(position === "after" ? at + 1 : at, 0, dir);
  return rest;
}

/**
 * 导航栏「收藏」分区要渲染的行。
 *
 * 顺序完全按收藏列表走(用户拖出来的),计数/仓库名从当前项目清单里现取;
 * 已经没有任何会话的收藏项不渲染,但仍留在存储里——重新扫到就会自己回来。
 */
export function favoriteProjectRows(projects, favorites) {
  const byDir = new Map(projects.map(project => [project.dir, project]));
  return favorites.map(dir => byDir.get(dir)).filter(Boolean);
}

export function buildLibraryIndex({ sessions, metadata, migratedSessionKeys, t }) {
  return sessions.map(session => {
    const meta = metadata[sessionIdentity(session)] || {};
    const tags = meta.tags || [];
    const treeCount = session.tree_count || 1;
    // 引擎索引行带的是整棵树的消息数;缺字段(旧缓存/其它来源)时元信息行不显示条数
    const count = Number.isFinite(session.count) ? session.count : null;
    return {
      tool: session.tool,
      bucket: bucketOf(session.updated),
      repo: repoOf(session.dir),
      dir: session.dir || "",
      updated: session.updated || 0,
      tags,
      pinned: !!meta.pinned,
      sub: treeCount > 1,
      mig: migratedSessionKeys.has(sessionIdentity(session)),
      hay: `${session.title || ""}\n${meta.name || ""}\n${tags.join("\n")}\n${session.dir || ""}\n${session.id}`.toLowerCase(),
      row: {
        key: sessionIdentity(session),
        id: session.id,
        title: meta.name || session.title || t("app:library.untitled"),
        repo: repoOf(session.dir),
        dir: session.dir,
        branch: session.branch || "",
        active: fmtTime(session.updated, t),
        tool: session.tool,
        dot: "var(--ok)",
        pinned: !!meta.pinned,
        tags: meta.tags,
        count,
        hasSub: treeCount > 1,
        subCount: treeCount - 1,
        subLabel: t("app:library.subLabel", { n: treeCount - 1 }),
        hasMig: migratedSessionKeys.has(sessionIdentity(session)),
      },
    };
  });
}

const TIME_BUCKETS = {
  all: BUCKETS,
  today: ["today"],
  last7: ["today", "yesterday", "last7"],
  last30: ["today", "yesterday", "last7", "last30"],
};

// 项目视图的文件夹 key:必须带完整路径,仓库名会撞车
export const projectGroupKey = dir => `dir:${dir}`;

/**
 * 项目范围按完整 dir 比对。
 *
 * 旧版本把仓库名存进了 filter.dir(localStorage / 内存状态都可能残留),
 * 读到不含 "/" 的值就退回按仓库名匹配,不让存量用户看到空列表。
 */
export function matchesProjectFilter(entry, dir) {
  if (!dir) return true;
  return String(dir).includes("/") ? entry.dir === dir : entry.repo === dir;
}

// ---------------------------------------------------------------------------
// 范围(Scope)与显示选项(Display)
//
// 范围回答「我在看哪一部分会话」,是单选、互斥的:全部 / 置顶 / 某 Agent /
// 某项目 / 某标签。它常驻在导航栏上,选中即切到会话页。
// 显示选项回答「这部分会话怎么排怎么显示」:分组方式、时间窗口、两个低频开关
// 和排序,躲在资源栏的「显示」菜单里,不占常驻位置。
// 两者一起取代了原来的六维筛选浮层。
// ---------------------------------------------------------------------------

export const SCOPE_KINDS = ["all", "pinned", "agent", "project", "tag"];
export const DEFAULT_SCOPE = { kind: "all" };
export const DEFAULT_DISPLAY = {
  group: "time",
  time: "all",
  subOnly: false,
  migOnly: false,
  sort: "updated",
};
const GROUP_MODES = ["project", "time", "none"];
const DISPLAY_TIMES = ["all", "last7", "last30"];

// 存量值可能来自旧版本或被手改过:任何一处对不上就整体回落到默认,
// 宁可丢一次用户偏好,也不要让列表进入一个说不清的状态。
export function normalizeScope(value) {
  if (!value || typeof value !== "object") return DEFAULT_SCOPE;
  const { kind } = value;
  if (kind === "all" || kind === "pinned") return { kind };
  if (!SCOPE_KINDS.includes(kind)) return DEFAULT_SCOPE;
  return value.value ? { kind, value: String(value.value) } : DEFAULT_SCOPE;
}

export function normalizeDisplay(value) {
  if (!value || typeof value !== "object") return DEFAULT_DISPLAY;
  return {
    group: GROUP_MODES.includes(value.group) ? value.group : DEFAULT_DISPLAY.group,
    time: DISPLAY_TIMES.includes(value.time) ? value.time : DEFAULT_DISPLAY.time,
    subOnly: !!value.subOnly,
    migOnly: !!value.migOnly,
    sort: "updated",
  };
}

/**
 * 旧筛选状态 → 新的 scope / display。
 *
 * 能映射的映射:目录 → 项目范围、只勾了一个来源 → Agent 范围、标签 → 标签范围
 * (三者互斥,按这个优先级取第一个命中的);时间 / 仅迁移 / 仅子会话 → 显示选项;
 * 旧的「时间 / 项目」视图分段 → 显示选项里的分组方式。其余(多选来源、"今天"
 * 这个新时间窗口里没有的档位)直接丢弃。
 */
export function migrateLegacyLibraryState({ filter, view, toolIds = [] } = {}) {
  const legacy = filter || {};
  let scope = DEFAULT_SCOPE;
  if (legacy.dir) scope = { kind: "project", value: String(legacy.dir) };
  else if (Array.isArray(legacy.src) && legacy.src.length === 1
    && toolIds.length > 1) scope = { kind: "agent", value: String(legacy.src[0]) };
  else if (legacy.tag) scope = { kind: "tag", value: String(legacy.tag) };
  const display = normalizeDisplay({
    group: GROUP_MODES.includes(view) ? view : DEFAULT_DISPLAY.group,
    time: legacy.time,
    subOnly: legacy.sub,
    migOnly: legacy.mig,
  });
  return { scope, display };
}

export function scopeMatches(entry, scope) {
  switch (scope.kind) {
    case "pinned": return entry.pinned;
    case "agent": return entry.tool === scope.value;
    case "project": return matchesProjectFilter(entry, scope.value);
    case "tag": return entry.tags.includes(scope.value);
    default: return true;
  }
}

export function sameScope(left, right) {
  return left.kind === right.kind && (left.value ?? null) === (right.value ?? null);
}

/**
 * 分组方式的实际取值。
 *
 * 项目文件夹树只在「全部会话」下成立:已经选了某个项目或 Agent 时,再按项目
 * 分组要么只剩一个文件夹,要么把范围本身重复说了一遍——此时一律回落到时间分组。
 */
export function effectiveGroupMode(scope, display) {
  if (display.group === "none") return "none";
  if (display.group === "project" && scope.kind === "all") return "project";
  return "time";
}

// 有几项显示选项被改过:决定「显示」按钮上要不要亮那个圆点
export function displayDirtyCount(display) {
  return (display.group !== DEFAULT_DISPLAY.group ? 1 : 0)
    + (display.time !== DEFAULT_DISPLAY.time ? 1 : 0)
    + (display.subOnly ? 1 : 0)
    + (display.migOnly ? 1 : 0);
}

/**
 * 导航栏范围区的全部计数。
 *
 * 一律基于全量索引:显示选项(时间窗口、仅子会话…)不该让导航栏上的数字跳动,
 * 否则用户没法拿它当"我这里一共有多少"来读。Agent 按声明顺序排(按计数排会
 * 随着新会话进来抖动),项目按最近活跃排,标签按计数排。
 */
export function libraryScopeCounts(index, toolIds = []) {
  const byTool = {};
  const byTag = {};
  let pinned = 0;
  for (const entry of index) {
    byTool[entry.tool] = (byTool[entry.tool] || 0) + 1;
    if (entry.pinned) pinned += 1;
    for (const tag of entry.tags) byTag[tag] = (byTag[tag] || 0) + 1;
  }
  return {
    total: index.length,
    pinned,
    agents: toolIds
      .filter(tool => byTool[tool])
      .map(tool => ({ tool, count: byTool[tool] })),
    tags: Object.entries(byTag)
      .map(([tag, count]) => ({ tag, count }))
      .sort((left, right) => right.count - left.count
        || left.tag.localeCompare(right.tag)),
  };
}

// 范围名:资源栏标题与导航栏选中项共用一份文案
export function scopeLabel(scope, { t, toolNames = {} }) {
  switch (scope.kind) {
    case "pinned": return t("app:library.pinned");
    case "agent": return toolNames[scope.value] || scope.value;
    case "project": return repoOf(scope.value) || scope.value;
    case "tag": return `#${scope.value}`;
    default: return t("app:nav.allSessions");
  }
}

export function buildLibraryGroups({ index, scope, display, query, t }) {
  const timeBuckets = TIME_BUCKETS[display.time] || TIME_BUCKETS.all;
  const needle = query.trim().toLowerCase();
  const matched = index.filter(entry => scopeMatches(entry, scope)
    && (!display.migOnly || entry.mig)
    && (!display.subOnly || entry.sub)
    && (!needle || entry.hay.includes(needle)));

  const groups = [];
  // 范围本身就是「置顶」时不再单开一个置顶组:那等于把整个列表包了一层
  let rest = matched;
  if (scope.kind !== "pinned") {
    const pinned = matched.filter(entry => entry.pinned);
    if (pinned.length) {
      groups.push({ key: "pinned", kind: "time", label: t("app:library.pinned"),
        count: pinned.length, rows: pinned.map(entry => entry.row) });
    }
    rest = matched.filter(entry => !entry.pinned);
  }
  const inWindow = rest.filter(entry => timeBuckets.includes(entry.bucket));
  const mode = effectiveGroupMode(scope, display);

  if (mode === "none") {
    if (inWindow.length) {
      const rows = inWindow.slice()
        .sort((left, right) => right.updated - left.updated)
        .map(entry => entry.row);
      groups.push({ key: "flat", kind: "flat", label: "", count: rows.length, rows });
    }
    return groups;
  }

  if (mode === "project") {
    const byDir = new Map();
    for (const entry of inWindow) {
      const key = projectGroupKey(entry.dir);
      const group = byDir.get(key) || {
        key,
        kind: "project",
        dir: entry.dir,
        label: repoOf(entry.dir) || entry.dir,
        parent: parentOf(entry.dir),
        tools: [],
        updated: 0,
        entries: [],
      };
      if (!group.tools.includes(entry.tool)) group.tools.push(entry.tool);
      group.updated = Math.max(group.updated, entry.updated);
      group.entries.push(entry);
      byDir.set(key, group);
    }
    [...byDir.values()]
      .sort((left, right) => right.updated - left.updated)
      .forEach(group => {
        const entries = group.entries.slice().sort((left, right) => right.updated - left.updated);
        groups.push({
          key: group.key,
          kind: "project",
          dir: group.dir,
          label: group.label,
          parent: group.parent,
          tools: group.tools,
          count: entries.length,
          rows: entries.map(entry => entry.row),
        });
      });
    return groups;
  }

  const byBucket = {};
  BUCKETS.forEach(key => { byBucket[key] = []; });
  inWindow.forEach(entry => { byBucket[entry.bucket].push(entry.row); });
  for (const key of BUCKETS) {
    if (!byBucket[key].length) continue;
    groups.push({ key, kind: "time", label: t(`common:bucket.${key}`),
      count: byBucket[key].length, rows: byBucket[key] });
  }
  return groups;
}

/**
 * 分组当前是否展开。
 *
 * 默认值按分组类型分开:时间分组(含置顶组)默认展开,项目文件夹默认折叠——
 * 项目视图一进去往往几十个文件夹,全展开等于把列表变成一条长滚动。
 * 用户手动展开/折叠过的记录仍存在同一个 collapsedGroups 映射里。
 *
 * 搜的时候把命中的项目文件夹自动展开:留在折叠里的命中行等于没搜到
 * (没命中的文件夹本来就已经被过滤掉,不会出现在 groups 里)。
 */
export function libraryGroupExpanded(group, collapsedGroups, query = "") {
  if (group.kind === "flat") return true;
  if (group.kind === "project" && query.trim()) return true;
  return !(collapsedGroups[group.key] ?? group.kind === "project");
}

// 首次进入项目视图时,选中会话所在的文件夹自动展开一次,避免上下文凭空消失
export function selectedProjectGroupKey(groups, selectedKey) {
  if (!selectedKey) return null;
  const group = groups.find(item => item.kind === "project"
    && item.rows.some(row => row.key === selectedKey));
  return group ? group.key : null;
}

export function visibleLibraryIds(groups, collapsedGroups, query = "") {
  return groups.filter(group => libraryGroupExpanded(group, collapsedGroups, query))
    .flatMap(group => group.rows.map(row => row.key));
}
