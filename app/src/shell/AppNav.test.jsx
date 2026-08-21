// 导航栏是这次改版的承重墙:范围(全部/置顶/Agent/项目/标签)从筛选浮层搬到了
// 这里,常驻、单选、带计数。这些用例守的是"点一行等于选一个范围"这条主干,
// 以及折叠态下范围区整体让位给图标轨。
import { beforeEach, test } from "vitest";
import assert from "node:assert/strict";
import { fireEvent, render as rtlRender, screen } from "@testing-library/react";

import { FerryRuntimeProvider } from "../shared/capabilities/ferryRuntime.jsx";
import { AppNav } from "./AppNav.jsx";

const ferry = { sessions: [] };
const render = ui => rtlRender(<FerryRuntimeProvider value={ferry}>{ui}</FerryRuntimeProvider>);
const noop = () => {};

const labels = {
  pinned: "置顶", agents: "AGENTS", favorites: "收藏", tags: "标签",
  favoritesEmpty: "在文件夹上点 ☆ 可收藏到这里",
  scanning: "扫描中", rescan: "重新扫描", settings: "设置",
  toolNames: { claude: "Claude Code", codex: "Codex" },
};

const project = (repo, count, index = 0) => ({
  dir: `/work/${repo}`, repo, parent: "/work", count,
  updated: 1000 - index, tools: ["claude"], ambiguous: false,
});

function baseProps(overrides = {}) {
  return {
    collapsed: false,
    railOnly: false,
    resizing: false,
    items: [{ key: "overview", label: "总览" }, { key: "library", label: "会话" }],
    activeView: "library",
    draggingKey: null,
    dropTarget: null,
    scanning: false,
    settingsOpen: false,
    labels,
    scope: { kind: "all" },
    scopeCounts: {
      total: 128,
      pinned: 3,
      agents: [{ tool: "claude", count: 84 }, { tool: "codex", count: 31 }],
      tags: [{ tag: "待整理", count: 5 }],
    },
    favoriteProjects: [project("ferry", 64), project("dotfiles", 11, 1)],
    onReorderFavorite: noop,
    onSelectScope: noop,
    onSelect: noop,
    onRescan: noop,
    onToggleSettings: noop,
    onEnter: noop,
    onLeave: noop,
    pointerHandlers: {},
    ...overrides,
  };
}

beforeEach(() => localStorage.clear());

test("展开态列出页面项、置顶、Agents、收藏与标签,各带计数", () => {
  render(<AppNav {...baseProps()} />);

  assert.ok(screen.getByText("总览"));
  assert.ok(screen.getByText("会话"));
  assert.ok(screen.getByText("128")); // 会话总数
  assert.ok(screen.getByText("置顶"));
  assert.ok(screen.getByText("Claude Code"));
  assert.ok(screen.getByText("84"));
  assert.ok(screen.getByText("ferry"));
  assert.ok(screen.getByText("待整理"));
});

test("Agent 按声明顺序渲染,不按计数重排——否则新会话进来就抖", () => {
  render(
    <AppNav
      {...baseProps({
        scopeCounts: {
          ...baseProps().scopeCounts,
          agents: [{ tool: "claude", count: 2 }, { tool: "codex", count: 40 }],
        },
      })}
    />,
  );

  const rows = screen.getAllByRole("button")
    .map(node => node.textContent.trim())
    .filter(text => text.includes("Claude Code") || text.includes("Codex"));
  assert.deepEqual(rows, ["Claude Code2", "Codex40"]);
});

test("点某个范围就把它回传上去", () => {
  const picked = [];
  render(<AppNav {...baseProps({ onSelectScope: value => picked.push(value) })} />);

  fireEvent.click(screen.getByText("Codex"));
  fireEvent.click(screen.getByText("ferry"));
  fireEvent.click(screen.getByText("置顶"));
  fireEvent.click(screen.getByText("待整理"));

  assert.deepEqual(picked, [
    { kind: "agent", value: "codex" },
    { kind: "project", value: "/work/ferry" },
    { kind: "pinned" },
    { kind: "tag", value: "待整理" },
  ]);
});

test("选中的范围行是高亮的那一条,「会话」此时不再高亮", () => {
  render(<AppNav {...baseProps({ scope: { kind: "agent", value: "claude" } })} />);

  const current = screen.getAllByRole("button")
    .filter(node => node.getAttribute("aria-current") === "true");
  assert.deepEqual(current.map(node => node.textContent.trim()), ["Claude Code84"]);
});

test("范围是置顶时高亮置顶行,不再高亮「会话」", () => {
  render(<AppNav {...baseProps({ scope: { kind: "pinned" } })} />);

  const current = screen.getAllByRole("button")
    .filter(node => node.getAttribute("aria-current") === "true");
  assert.deepEqual(current.map(node => node.textContent.trim()), ["置顶3"]);
});

test("没有置顶会话时不显示置顶项", () => {
  render(<AppNav {...baseProps({ scopeCounts: { ...baseProps().scopeCounts, pinned: 0 } })} />);

  assert.equal(screen.queryByText("置顶"), null);
});

// ---- 收藏区(Finder 侧栏式):只列收藏过的项目,顺序可拖 ----

test("收藏区只列收藏过的项目,不再是全部项目的第二份清单", () => {
  const favorites = Array.from({ length: 3 }, (_, i) => project(`p${i}`, i + 1, i));
  render(<AppNav {...baseProps({ favoriteProjects: favorites })} />);

  assert.ok(screen.getByText("收藏"));
  ["p0", "p1", "p2"].forEach(name => assert.ok(screen.getByText(name)));
  // 「显示全部 N 个项目」那条开关已经撤掉
  assert.equal(screen.queryByText(/显示全部/), null);
});

test("一个收藏都没有时给一行提示,而不是把分区整个藏起来", () => {
  render(<AppNav {...baseProps({ favoriteProjects: [] })} />);

  assert.ok(screen.getByText("收藏"));
  assert.ok(screen.getByText("在文件夹上点 ☆ 可收藏到这里"));
});

test("拖动收藏行把新顺序回传上去", () => {
  const moves = [];
  render(<AppNav {...baseProps({
    onReorderFavorite: (dir, target, position) => moves.push([dir, target, position]),
  })} />);

  const source = screen.getByText("dotfiles").closest("button");
  const target = screen.getByText("ferry").closest("button");
  const dataTransfer = { setData: () => {}, effectAllowed: "" };
  fireEvent.dragStart(source, { dataTransfer });
  // jsdom 不做布局,getBoundingClientRect 全是 0,clientY 0 落在上半 → before
  fireEvent.dragOver(target, { dataTransfer, clientY: 0 });
  fireEvent.drop(target, { dataTransfer });

  assert.deepEqual(moves, [["/work/dotfiles", "/work/ferry", "before"]]);
});

test("分区可折叠,折叠状态记进 localStorage", () => {
  const { unmount } = render(<AppNav {...baseProps()} />);

  fireEvent.click(screen.getByText("AGENTS"));
  assert.equal(screen.queryByText("Claude Code"), null);
  unmount();

  render(<AppNav {...baseProps()} />);
  assert.equal(screen.queryByText("Claude Code"), null);
  assert.ok(screen.getByText("ferry"), "只收起 AGENTS,收藏分区不受影响");
});

test("折叠态只剩图标:范围分区整体隐藏,置顶仍留一个入口", () => {
  const picked = [];
  const { container } = render(
    <AppNav {...baseProps({ collapsed: true, onSelectScope: value => picked.push(value) })} />,
  );

  assert.equal(container.firstChild.style.width, "56px");
  assert.equal(screen.queryByText("AGENTS"), null);
  assert.equal(screen.queryByText("收藏"), null);
  assert.equal(screen.queryByText("Claude Code"), null);
  assert.equal(screen.queryByText("ferry"), null);
  // 页面项 + 置顶 + 重扫 + 设置
  assert.equal(container.querySelectorAll("button").length, 5);
});

test("折叠且没有资源栏时导航栏加宽,给红绿灯让位", () => {
  const { container } = render(
    <AppNav {...baseProps({ collapsed: true, railOnly: true })} />,
  );
  assert.equal(container.firstChild.style.width, "80px");
});

test("重扫与设置在底部,扫描中时重扫不可点", () => {
  const calls = [];
  const { unmount } = render(
    <AppNav {...baseProps({ onRescan: () => calls.push("rescan"),
      onToggleSettings: () => calls.push("settings") })} />,
  );

  fireEvent.click(screen.getByText("重新扫描"));
  fireEvent.click(screen.getByText("设置"));
  assert.deepEqual(calls, ["rescan", "settings"]);
  unmount();

  render(<AppNav {...baseProps({ scanning: true, onRescan: () => calls.push("rescan") })} />);
  fireEvent.click(screen.getByText("扫描中"));
  assert.deepEqual(calls, ["rescan", "settings"], "扫描中不再重复触发");
});
