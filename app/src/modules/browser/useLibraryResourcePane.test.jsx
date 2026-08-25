// 会话库资源栏的状态:范围与显示选项各自持久化,范围再点一次回到「全部」,
// 计数不受显示选项影响。这三条是导航栏能当"目录"用的前提。
import { beforeEach, test } from "vitest";
import assert from "node:assert/strict";
import { act, renderHook } from "@testing-library/react";

import { useLibraryResourcePane } from "./useLibraryResourcePane.js";

const t = key => key;
const toolIds = ["claude", "codex"];
const toolNames = { claude: "Claude Code", codex: "Codex" };
const now = Date.now();
const sessions = [
  { tool: "claude", id: "a", title: "Payment", dir: "/work/payments", updated: now },
  { tool: "codex", id: "b", title: "Search", dir: "/work/search", updated: now - 60e3 },
  { tool: "claude", id: "c", title: "旧的", dir: "/work/payments", updated: now - 40 * 86400e3 },
];
const metadata = { ["claude\u0000a"]: { pinned: true, tags: ["待整理"] } };

const setup = () => renderHook(() => useLibraryResourcePane({
  sessions, metadata, migratedSessionKeys: new Set(), t, toolIds, toolNames,
}));

beforeEach(() => localStorage.clear());

test("默认范围是全部、分组按时间,标题给出范围名与总数", () => {
  const { result } = setup();

  assert.deepEqual(result.current.scope, { kind: "all" });
  assert.equal(result.current.groupMode, "time");
  assert.equal(result.current.scopeLabel, "app:nav.allSessions");
  assert.equal(result.current.scopeCount, 3);
});

test("范围单选互斥:选 Agent 清项目,再点同一项回到全部", () => {
  const { result } = setup();

  act(() => result.current.selectScope({ kind: "project", value: "/work/payments" }));
  assert.deepEqual(result.current.scope, { kind: "project", value: "/work/payments" });
  assert.equal(result.current.scopeLabel, "payments");
  assert.equal(result.current.scopeCount, 2);
  // 选了项目后文件夹树退回时间平铺
  assert.equal(result.current.groupMode, "time");

  act(() => result.current.selectScope({ kind: "agent", value: "codex" }));
  assert.deepEqual(result.current.scope, { kind: "agent", value: "codex" });
  assert.equal(result.current.scopeLabel, "Codex");

  act(() => result.current.selectScope({ kind: "agent", value: "codex" }));
  assert.deepEqual(result.current.scope, { kind: "all" });
});

test("范围与显示选项分别落盘,重新挂载后还在", () => {
  const first = setup();
  act(() => {
    first.result.current.selectScope({ kind: "tag", value: "待整理" });
    first.result.current.setDisplay({ time: "last7", subOnly: true });
  });
  first.unmount();

  assert.deepEqual(
    JSON.parse(localStorage.getItem("ferry-library-scope")),
    { kind: "tag", value: "待整理" },
  );

  const { result } = setup();
  assert.deepEqual(result.current.scope, { kind: "tag", value: "待整理" });
  assert.equal(result.current.display.time, "last7");
  assert.equal(result.current.display.subOnly, true);
  assert.equal(result.current.displayDirty, 2);
});

test("首次启动从旧的「时间 / 项目」视图分段迁移一次,并清掉旧键", () => {
  localStorage.setItem("ferry-library-view", "time");
  const { result } = setup();

  assert.equal(result.current.display.group, "time");
  assert.equal(localStorage.getItem("ferry-library-view"), null);
});

test("导航栏计数取自全量索引,时间窗口收窄了也不跟着掉", () => {
  const { result } = setup();
  const before = result.current.scopeCounts;

  act(() => result.current.setDisplay({ time: "last7" }));

  assert.deepEqual(result.current.scopeCounts, before);
  assert.equal(result.current.scopeCounts.total, 3);
  assert.equal(result.current.scopeCounts.pinned, 1);
  assert.deepEqual(result.current.scopeCounts.agents,
    [{ tool: "claude", count: 2 }, { tool: "codex", count: 1 }]);
  assert.deepEqual(result.current.scopeCounts.tags, [{ tag: "待整理", count: 1 }]);
  // 但列表本身确实被窗口挡掉了 40 天前的那条
  assert.equal(
    result.current.groups.flatMap(group => group.rows).length, 2,
  );
});

test("清除同时归零范围、显示选项与搜索词", () => {
  const { result } = setup();
  act(() => {
    result.current.selectScope({ kind: "pinned" });
    result.current.setDisplay({ group: "none" });
    result.current.setQuery("pay");
  });

  act(() => result.current.clear());

  assert.deepEqual(result.current.scope, { kind: "all" });
  assert.equal(result.current.display.group, "time");
  assert.equal(result.current.query, "");
  assert.equal(result.current.displayDirty, 0);
});

// ---- 收藏的项目:首启自动收藏 3 个,之后完全交给用户 ----

test("首次运行(没有收藏记录键)自动收藏最近活跃的项目并落盘", () => {
  const { result } = setup();

  // sessions 里只有 payments / search 两个项目,种子最多给 3 个
  assert.deepEqual(result.current.favorites, ["/work/payments", "/work/search"]);
  assert.deepEqual(
    JSON.parse(localStorage.getItem("ferry-favorite-projects")),
    ["/work/payments", "/work/search"],
  );
  assert.deepEqual(
    result.current.favoriteProjects.map(p => p.repo), ["payments", "search"]);
});

test("用户把收藏清空后不再自动塞回去——空数组是选择,不是没记录", () => {
  localStorage.setItem("ferry-favorite-projects", "[]");
  const { result } = setup();

  assert.deepEqual(result.current.favorites, []);
  assert.deepEqual(result.current.favoriteProjects, []);
});

test("收藏是开关,改完立刻落盘", () => {
  localStorage.setItem("ferry-favorite-projects", JSON.stringify(["/work/payments"]));
  const { result } = setup();

  act(() => result.current.toggleFavorite("/work/search"));
  assert.deepEqual(result.current.favorites, ["/work/payments", "/work/search"]);

  act(() => result.current.toggleFavorite("/work/payments"));
  assert.deepEqual(
    JSON.parse(localStorage.getItem("ferry-favorite-projects")), ["/work/search"]);
});

test("拖拽排序改的是收藏顺序,收藏行跟着换位", () => {
  localStorage.setItem("ferry-favorite-projects",
    JSON.stringify(["/work/payments", "/work/search"]));
  const { result } = setup();

  act(() => result.current.reorderFavorite("/work/search", "/work/payments", "before"));
  assert.deepEqual(result.current.favoriteProjects.map(p => p.repo), ["search", "payments"]);
});
