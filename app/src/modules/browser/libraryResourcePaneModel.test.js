import assert from "node:assert/strict";
import { test } from "vitest";

import {
  DEFAULT_DISPLAY,
  DEFAULT_SCOPE,
  buildLibraryGroups,
  buildLibraryIndex,
  displayDirtyCount,
  effectiveGroupMode,
  favoriteProjectRows,
  libraryGroupExpanded,
  libraryProjects,
  libraryScopeCounts,
  parentOf,
  migrateLegacyLibraryState,
  normalizeDisplay,
  normalizeFavoriteProjects,
  normalizeScope,
  reorderFavoriteProjects,
  seedFavoriteProjects,
  toggleFavoriteProject,
  sameScope,
  scopeLabel,
  visibleLibraryIds,
  selectedProjectGroupKey,
} from "./libraryResourcePaneModel.js";

const t = (key, params = {}) => `${key}:${JSON.stringify(params)}`;
const tools = ["claude", "codex", "opencode"];
const now = Date.now();
const sessions = [
  { tool: "claude", id: "a", title: "Payment", dir: "/work/payments", updated: now, tree_count: 2 },
  { tool: "codex", id: "b", title: "Search", dir: "/work/search", updated: now - 2 * 86400e3 },
];
const scope = DEFAULT_SCOPE;
const display = DEFAULT_DISPLAY;
// 大多数用例只关心"按时间平铺"这条基线;项目文件夹树单独用 byProject 覆盖
const byTime = DEFAULT_DISPLAY;
const byProject = { ...DEFAULT_DISPLAY, group: "project" };

test("会话库投影优先置顶，并按当前筛选分组", () => {
  const index = buildLibraryIndex({
    sessions,
    metadata: { "claude\u0000a": { pinned: true, tags: ["finance"] } },
    migratedSessionKeys: new Set(["codex\u0000b"]),
    t,
  });
  const groups = buildLibraryGroups({ index, scope, display: byTime, query: "pay", t });

  assert.deepEqual(groups.map(group => group.key), ["pinned"]);
  assert.deepEqual(groups[0].rows.map(row => row.key), ["claude\u0000a"]);
  assert.equal(index[1].mig, true);
});

test("折叠分组只影响可见导航顺序，不重建分组", () => {
  const index = buildLibraryIndex({ sessions, metadata: {}, migratedSessionKeys: new Set(), t });
  const groups = buildLibraryGroups({ index, scope, display: byTime, query: "", t });

  assert.deepEqual(groups.map(group => group.key), ["today", "last7"]);
  assert.deepEqual(visibleLibraryIds(groups, { today: true }), ["codex\u0000b"]);
  assert.deepEqual(groups[0].rows.map(row => row.key), ["claude\u0000a"]);
});

// ---- 项目视图:分组键是完整 dir,仓库名只是显示用的标签 ----

const projectSessions = [
  { tool: "claude", id: "a", title: "Payment", dir: "/work/payments", updated: now, count: 12 },
  { tool: "codex", id: "b", title: "Payment 修复", dir: "/work/payments", updated: now - 60e3 },
  { tool: "claude", id: "c", title: "Payment 备份", dir: "/side/payments", updated: now - 120e3 },
];
const projectIndex = () => buildLibraryIndex({
  sessions: projectSessions, metadata: {}, migratedSessionKeys: new Set(), t,
});
const KEYS = ["claude\u0000a", "codex\u0000b", "claude\u0000c"];

test("项目视图按完整 dir 分组，同名仓库各成一组且组内按最近活跃倒序", () => {
  const groups = buildLibraryGroups({
    index: projectIndex(), scope, display: byProject, query: "", t,
  });

  assert.deepEqual(groups.map(group => group.key), ["dir:/work/payments", "dir:/side/payments"]);
  assert.deepEqual(groups.map(group => group.label), ["payments (work)", "payments (side)"]);
  assert.deepEqual(groups.map(group => group.parent), ["/work", "/side"]);
  assert.deepEqual(groups[0].rows.map(row => row.key), [KEYS[0], KEYS[1]]);
  assert.deepEqual(groups[0].tools, ["claude", "codex"]);
  assert.equal(groups[0].count, 2);
});

test("同名仓库在项目清单里被标记为需要父路径消歧，并按最近活跃排序", () => {
  const projects = libraryProjects(projectSessions);

  assert.deepEqual(projects.map(project => project.dir), ["/work/payments", "/side/payments"]);
  assert.deepEqual(projects.map(project => project.ambiguous), [true, true]);
  assert.deepEqual(projects[0].tools, ["claude", "codex"]);
  assert.equal(projects[0].count, 2);
});

test("项目范围按完整 dir 精确匹配，旧的仓库名取值仍然可用", () => {
  // 选中某个项目后分组方式退回时间平铺:文件夹树只在「全部会话」下成立
  const byDir = buildLibraryGroups({
    index: projectIndex(), scope: { kind: "project", value: "/side/payments" },
    display: byProject, query: "", t,
  });
  assert.deepEqual(byDir.flatMap(group => group.rows.map(row => row.key)), [KEYS[2]]);

  // 存量状态里存的是仓库名(不含 "/"),两个同名目录都该留下
  const legacy = buildLibraryGroups({
    index: projectIndex(), scope: { kind: "project", value: "payments" },
    display: byProject, query: "", t,
  });
  assert.deepEqual(
    legacy.flatMap(group => group.rows.map(row => row.key)).sort(),
    [...KEYS].sort(),
  );
});

test("Windows 路径在项目清单和分组里只显示文件夹名", () => {
  const winSessions = [
    { tool: "codex", id: "w1", title: "Win", dir: "D:\\code\\ferry", updated: now },
    { tool: "claude", id: "w2", title: "Desk", dir: "C:\\Users\\12467\\Desktop\\rweixin", updated: now - 1 },
    { tool: "opencode", id: "w3", title: "Desk 2", dir: "c:/Users/12467/Desktop/rweixin/", updated: now - 2 },
  ];
  const projects = libraryProjects(winSessions);
  assert.deepEqual(projects.map(project => project.repo), ["ferry", "rweixin"]);
  assert.equal(projects[1].count, 2, "不同 Agent 的斜杠和盘符大小写必须合成同一项目");
  assert.equal(parentOf("D:\\code\\ferry"), "D:\\code");
  assert.equal(parentOf("C:\\Users\\12467\\Desktop\\rweixin"), "C:\\Users\\12467\\Desktop");

  const groups = buildLibraryGroups({
    index: buildLibraryIndex({
      sessions: winSessions, metadata: {}, migratedSessionKeys: new Set(), t,
    }),
    scope, display: byProject, query: "", t,
  });
  assert.deepEqual(groups.map(group => group.label), ["ferry", "rweixin"]);
  assert.equal(groups[1].count, 2);

  const filtered = buildLibraryGroups({
    index: buildLibraryIndex({
      sessions: winSessions, metadata: {}, migratedSessionKeys: new Set(), t,
    }),
    scope: { kind: "project", value: "D:\\code\\ferry" },
    display: byTime, query: "", t,
  });
  assert.deepEqual(
    filtered.flatMap(group => group.rows.map(row => row.id)),
    ["w1"],
  );
});

test("同名但不同路径的项目显示最短父目录，不再看起来像重复文件夹", () => {
  const dated = [
    { tool: "codex", id: "n1", title: "One", dir: "C:\\Chats\\2026-06-21\\new-chat", updated: now },
    { tool: "codex", id: "n2", title: "Two", dir: "C:\\Chats\\2026-08-07\\new-chat", updated: now - 1 },
  ];
  const groups = buildLibraryGroups({
    index: buildLibraryIndex({ sessions: dated, metadata: {}, migratedSessionKeys: new Set(), t }),
    scope, display: byProject, query: "", t,
  });
  assert.deepEqual(groups.map(group => group.label),
    ["new-chat (2026-06-21)", "new-chat (2026-08-07)"]);
});

test("搜索时命中的项目文件夹自动展开，折叠状态只在无搜索时生效", () => {
  const groups = buildLibraryGroups({
    index: projectIndex(), scope, display: byProject, query: "", t,
  });
  // 项目文件夹默认折叠,只有 /side/payments 被显式展开过
  const collapsed = { "dir:/side/payments": false };

  assert.deepEqual(visibleLibraryIds(groups, collapsed), [KEYS[2]]);
  assert.deepEqual(visibleLibraryIds(groups, collapsed, "payment"), [KEYS[0], KEYS[1], KEYS[2]]);
  assert.equal(libraryGroupExpanded(groups[0], collapsed, "payment"), true);
  assert.equal(libraryGroupExpanded(groups[0], collapsed, ""), false);
});

test("项目文件夹默认折叠，时间分组与置顶组默认展开", () => {
  const projectGroups = buildLibraryGroups({
    index: projectIndex(), scope, display: byProject, query: "", t,
  });
  assert.deepEqual(projectGroups.map(group => libraryGroupExpanded(group, {}, "")), [false, false]);
  assert.deepEqual(visibleLibraryIds(projectGroups, {}), []);
  // 展开过的记忆沿用同一映射
  assert.deepEqual(
    visibleLibraryIds(projectGroups, { "dir:/work/payments": false }),
    [KEYS[0], KEYS[1]],
  );

  const index = buildLibraryIndex({
    sessions, metadata: { "claude\u0000a": { pinned: true } }, migratedSessionKeys: new Set(), t,
  });
  const timeGroups = buildLibraryGroups({ index, scope, display: byTime, query: "", t });
  assert.deepEqual(timeGroups.map(group => group.key), ["pinned", "last7"]);
  assert.deepEqual(timeGroups.map(group => libraryGroupExpanded(group, {}, "")), [true, true]);
});

test("首次进入项目视图时定位选中会话所在的文件夹", () => {
  const groups = buildLibraryGroups({
    index: projectIndex(), scope, display: byProject, query: "", t,
  });

  assert.equal(selectedProjectGroupKey(groups, KEYS[2]), "dir:/side/payments");
  assert.equal(selectedProjectGroupKey(groups, "claude\u0000missing"), null);
  assert.equal(selectedProjectGroupKey(groups, null), null);
});

test("消息数带进投影，缺字段时不臆造条数", () => {
  const index = projectIndex();

  assert.equal(index[0].row.count, 12);
  assert.equal(index[1].row.count, null);
});

// ---- 范围(Scope):单选、互斥,计数取自全量索引 ----

test("范围只留下属于它的会话,Agent 与项目互不叠加", () => {
  const index = projectIndex();

  const agent = buildLibraryGroups({
    index, scope: { kind: "agent", value: "codex" }, display, query: "", t,
  });
  assert.deepEqual(agent.flatMap(group => group.rows.map(row => row.key)), [KEYS[1]]);

  const pinnedIndex = buildLibraryIndex({
    sessions: projectSessions, metadata: { [KEYS[0]]: { pinned: true } },
    migratedSessionKeys: new Set(), t,
  });
  const pinned = buildLibraryGroups({
    index: pinnedIndex, scope: { kind: "pinned" }, display, query: "", t,
  });
  // 范围本身就是「置顶」时不再单开一个置顶组
  assert.deepEqual(pinned.map(group => group.key), ["today"]);
  assert.deepEqual(pinned.flatMap(group => group.rows.map(row => row.key)), [KEYS[0]]);
});

test("标签范围按标签过滤", () => {
  const index = buildLibraryIndex({
    sessions: projectSessions,
    metadata: { [KEYS[1]]: { tags: ["待整理"] } },
    migratedSessionKeys: new Set(), t,
  });
  const groups = buildLibraryGroups({
    index, scope: { kind: "tag", value: "待整理" }, display, query: "", t,
  });
  assert.deepEqual(groups.flatMap(group => group.rows.map(row => row.key)), [KEYS[1]]);
});

test("导航栏计数基于全量索引:Agent 按声明顺序,标签按数量", () => {
  const index = buildLibraryIndex({
    sessions: projectSessions,
    metadata: {
      [KEYS[0]]: { pinned: true, tags: ["a", "b"] },
      [KEYS[1]]: { tags: ["b"] },
    },
    migratedSessionKeys: new Set(), t,
  });
  const counts = libraryScopeCounts(index, tools);

  assert.equal(counts.total, 3);
  assert.equal(counts.pinned, 1);
  // codex 只有一条却排在 claude 之后:顺序取自声明表,不随计数抖动
  assert.deepEqual(counts.agents, [{ tool: "claude", count: 2 }, { tool: "codex", count: 1 }]);
  assert.deepEqual(counts.tags, [{ tag: "b", count: 2 }, { tag: "a", count: 1 }]);
});

test("范围名给资源栏标题用,标签带 # 前缀", () => {
  const label = value => scopeLabel(value, { t, toolNames: { claude: "Claude Code" } });
  assert.equal(label({ kind: "all" }), 'app:nav.allSessions:{}');
  assert.equal(label({ kind: "agent", value: "claude" }), "Claude Code");
  assert.equal(label({ kind: "project", value: "/work/payments" }), "payments");
  assert.equal(label({ kind: "tag", value: "待整理" }), "#待整理");
});

// ---- 显示选项(Display) ----

test("分组方式:文件夹树只在「全部会话」下成立", () => {
  assert.equal(effectiveGroupMode({ kind: "all" }, byProject), "project");
  assert.equal(effectiveGroupMode({ kind: "agent", value: "claude" }, byProject), "time");
  assert.equal(effectiveGroupMode({ kind: "all" }, { ...byProject, group: "none" }), "none");
});

test("不分组时整条列表按更新时间排,没有任何分组头", () => {
  const groups = buildLibraryGroups({
    index: projectIndex(), scope, display: { ...byProject, group: "none" }, query: "", t,
  });

  assert.deepEqual(groups.map(group => group.kind), ["flat"]);
  assert.deepEqual(groups[0].rows.map(row => row.key), KEYS);
  assert.equal(libraryGroupExpanded(groups[0], {}, ""), true);
});

test("时间窗口与两个开关只影响列表,不改变范围", () => {
  const index = buildLibraryIndex({
    sessions, metadata: {}, migratedSessionKeys: new Set(["codex\u0000b"]), t,
  });

  const today = buildLibraryGroups({
    index, scope, display: { ...byTime, time: "last7" }, query: "", t,
  });
  assert.deepEqual(today.map(group => group.key), ["today", "last7"]);

  const migOnly = buildLibraryGroups({
    index, scope, display: { ...byTime, migOnly: true }, query: "", t,
  });
  assert.deepEqual(migOnly.flatMap(group => group.rows.map(row => row.key)), ["codex\u0000b"]);

  const subOnly = buildLibraryGroups({
    index, scope, display: { ...byTime, subOnly: true }, query: "", t,
  });
  assert.deepEqual(subOnly.flatMap(group => group.rows.map(row => row.key)), ["claude\u0000a"]);
});

test("显示选项被改过几项决定按钮上的圆点", () => {
  assert.equal(displayDirtyCount(DEFAULT_DISPLAY), 0);
  assert.equal(displayDirtyCount({ ...DEFAULT_DISPLAY, group: "project" }), 1);
  assert.equal(displayDirtyCount({ ...DEFAULT_DISPLAY, time: "last7", subOnly: true }), 2);
});

// ---- 存量状态的读回与迁移 ----

test("存量值对不上就整体回落到默认,不让列表进入说不清的状态", () => {
  assert.deepEqual(normalizeScope(null), DEFAULT_SCOPE);
  assert.deepEqual(normalizeScope({ kind: "agent" }), DEFAULT_SCOPE); // 缺 value
  assert.deepEqual(normalizeScope({ kind: "nope", value: "x" }), DEFAULT_SCOPE);
  assert.deepEqual(normalizeScope({ kind: "pinned" }), { kind: "pinned" });
  assert.deepEqual(normalizeScope({ kind: "tag", value: "x" }), { kind: "tag", value: "x" });

  assert.deepEqual(normalizeDisplay(null), DEFAULT_DISPLAY);
  assert.deepEqual(
    normalizeDisplay({ group: "weird", time: "today", subOnly: 1, sort: "size" }),
    { ...DEFAULT_DISPLAY, subOnly: true },
  );
});

test("同一个范围再点一次的判定", () => {
  assert.equal(sameScope({ kind: "all" }, { kind: "all" }), true);
  assert.equal(sameScope({ kind: "agent", value: "claude" }, { kind: "agent", value: "claude" }), true);
  assert.equal(sameScope({ kind: "agent", value: "claude" }, { kind: "agent", value: "codex" }), false);
  assert.equal(sameScope({ kind: "all" }, { kind: "pinned" }), false);
});

test("旧筛选状态按优先级映射成范围,其余进显示选项", () => {
  // 目录优先于单选来源与标签
  assert.deepEqual(
    migrateLegacyLibraryState({
      filter: { dir: "/work/payments", src: ["claude"], tag: "x", time: "last7", sub: true },
      view: "time", toolIds: tools,
    }),
    {
      scope: { kind: "project", value: "/work/payments" },
      display: { ...DEFAULT_DISPLAY, group: "time", time: "last7", subOnly: true },
    },
  );

  // 只勾了一个来源 → Agent 范围
  assert.deepEqual(
    migrateLegacyLibraryState({ filter: { src: ["codex"] }, view: "project", toolIds: tools }).scope,
    { kind: "agent", value: "codex" },
  );
  // 多选来源没有对应的范围,直接丢弃
  assert.deepEqual(
    migrateLegacyLibraryState({ filter: { src: tools }, toolIds: tools }).scope,
    DEFAULT_SCOPE,
  );
  assert.deepEqual(
    migrateLegacyLibraryState({ filter: { tag: "待整理" }, toolIds: tools }).scope,
    { kind: "tag", value: "待整理" },
  );
  // 「今天」这一档新的时间窗口里没有,同样丢弃;没存过视图分段时默认按时间
  assert.deepEqual(
    migrateLegacyLibraryState({ filter: { time: "today" } }).display,
    DEFAULT_DISPLAY,
  );
  assert.deepEqual(migrateLegacyLibraryState().scope, DEFAULT_SCOPE);
});

// ---- 收藏的项目 ----

const proj = (repo, count, updated) => ({ dir: `/work/${repo}`, repo, count, updated });
const favProjects = [
  proj("ferry", 64, 300), proj("dotfiles", 11, 200), proj("klib", 5, 100), proj("old", 1, 50),
];

test("收藏值归一化:丢掉非字符串与重复项,顺序保持不变", () => {
  assert.deepEqual(
    normalizeFavoriteProjects(["/a", "/b", "/a", 3, null, "", "/c"]),
    ["/a", "/b", "/c"],
  );
  assert.deepEqual(
    normalizeFavoriteProjects(["C:\\Work\\Ferry", "c:/work/ferry/"]),
    ["C:\\Work\\Ferry"],
  );
  assert.deepEqual(normalizeFavoriteProjects("nope"), []);
});

test("首启种子取最近活跃的前 3 个项目的完整 dir", () => {
  assert.deepEqual(seedFavoriteProjects(favProjects),
    ["/work/ferry", "/work/dotfiles", "/work/klib"]);
});

test("收藏行按收藏顺序渲染,而不是按最近活跃", () => {
  const rows = favoriteProjectRows(favProjects, ["/work/klib", "/work/ferry"]);
  assert.deepEqual(rows.map(r => r.repo), ["klib", "ferry"]);
});

test("已经扫不到会话的收藏项不渲染,但不从存储里抹掉", () => {
  const favorites = ["/work/gone", "/work/ferry"];
  assert.deepEqual(favoriteProjectRows(favProjects, favorites).map(r => r.repo), ["ferry"]);
  // toggle 仍以完整列表为准,不会因为渲染时被过滤掉就丢了
  assert.deepEqual(toggleFavoriteProject(favorites, "/work/klib"),
    ["/work/gone", "/work/ferry", "/work/klib"]);
});

test("收藏是开关:再点一次就取消", () => {
  assert.deepEqual(toggleFavoriteProject(["/a"], "/a"), []);
  assert.deepEqual(toggleFavoriteProject([], "/a"), ["/a"]);
});

test("拖拽排序把项挪到目标之前或之后", () => {
  const list = ["/a", "/b", "/c"];
  assert.deepEqual(reorderFavoriteProjects(list, "/c", "/a", "before"), ["/c", "/a", "/b"]);
  assert.deepEqual(reorderFavoriteProjects(list, "/a", "/c", "after"), ["/b", "/c", "/a"]);
});

test("拖到自己身上或目标不在列表里时顺序原样不动", () => {
  const list = ["/a", "/b"];
  assert.equal(reorderFavoriteProjects(list, "/a", "/a"), list);
  assert.equal(reorderFavoriteProjects(list, "/a", "/zzz"), list);
  assert.equal(reorderFavoriteProjects(list, "/zzz", "/a"), list);
});

