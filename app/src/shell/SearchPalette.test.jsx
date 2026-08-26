// 命令面板的键盘导航:结果超过一屏时,高亮不能停在视口外。
// jsdom 不做布局,scrollIntoView 只能靠调用本身来断言(setup 里是空实现)。
import { afterEach, test, vi } from "vitest";
import assert from "node:assert/strict";
import { fireEvent, render, screen } from "@testing-library/react";

import { SearchPalette } from "./SearchPalette.jsx";

afterEach(() => vi.restoreAllMocks());

const results = Array.from({ length: 40 }, (_, i) => ({
  id: `s${i}`,
  title: `会话 ${i}`,
  onClick: () => {},
}));

let container = null;

function renderPalette(overrides = {}) {
  const spy = vi.spyOn(Element.prototype, "scrollIntoView");
  const view = render(
    <SearchPalette
      placeholder="搜索"
      query=""
      onQuery={() => {}}
      results={results}
      emptyLabel="无结果"
      onClose={() => {}}
      {...overrides}
    />,
  );
  container = view.container;
  spy.mockClear(); // 忽略挂载时对首项的那次
  return { spy, container: view.container };
}

// 高亮行靠背景色认:选中项是 --acc-soft2,其余是 transparent
function highlighted() {
  const row = [...container.querySelectorAll(".fscroll div[style]")]
    .find(el => el.style.background === "var(--acc-soft2)");
  return row?.textContent ?? null;
}

const scroller = () => container.querySelector(".fscroll");

test("↓ 让新高亮的结果滚进视口", () => {
  const { spy } = renderPalette();

  fireEvent.keyDown(window, { key: "ArrowDown" });

  assert.equal(spy.mock.calls.length, 1);
  assert.deepEqual(spy.mock.calls[0][0], { block: "nearest" });
  assert.equal(spy.mock.instances[0].textContent, "会话 1");
});

test("连续 ↓ 一路跟随到当前高亮项,不会把高亮甩在视口外", () => {
  const { spy } = renderPalette();

  for (let i = 0; i < 12; i += 1) fireEvent.keyDown(window, { key: "ArrowDown" });

  assert.equal(spy.mock.instances.at(-1).textContent, "会话 12");
});

test("↑ 同样跟随", () => {
  const { spy } = renderPalette();

  fireEvent.keyDown(window, { key: "ArrowDown" });
  fireEvent.keyDown(window, { key: "ArrowDown" });
  fireEvent.keyDown(window, { key: "ArrowUp" });

  assert.equal(spy.mock.instances.at(-1).textContent, "会话 1");
});

test("Enter 打开当前高亮项并关闭面板", () => {
  const opened = [];
  let closed = false;
  const rows = results.slice(0, 5).map(r => ({ ...r, onClick: () => opened.push(r.id) }));
  render(
    <SearchPalette
      placeholder="搜索" query="" onQuery={() => {}}
      results={rows} emptyLabel="无结果"
      onClose={() => { closed = true; }}
    />,
  );

  fireEvent.keyDown(window, { key: "ArrowDown" });
  fireEvent.keyDown(window, { key: "Enter" });

  assert.deepEqual(opened, ["s1"]);
  assert.equal(closed, true);
  assert.ok(screen.getByText("会话 0"));
});

// 方向键会滚动列表,新行滑到静止的光标底下时浏览器照样派发 mouseenter。
// 不设闸的话,键盘选中会被"划过"的行抢走,高亮看着像在乱蹦。
test("方向键滚动后,未移动的鼠标划过某行不抢走键盘选中", () => {
  renderPalette();

  fireEvent.keyDown(window, { key: "ArrowDown" });
  fireEvent.keyDown(window, { key: "ArrowDown" });
  // 光标没动,只是列表滚上来了——mouseEnter 仍会派发
  fireEvent.mouseEnter(screen.getByText("会话 9").closest("div"));

  assert.equal(highlighted(), "会话 2");
});

test("鼠标真的移动过之后,hover 重新接管选中", () => {
  renderPalette();

  fireEvent.keyDown(window, { key: "ArrowDown" });
  fireEvent.mouseMove(scroller());
  fireEvent.mouseEnter(screen.getByText("会话 9").closest("div"));

  assert.equal(highlighted(), "会话 9");
});

test("鼠标移动过后再按方向键,重新交还键盘控制", () => {
  renderPalette();

  fireEvent.mouseMove(scroller());
  fireEvent.mouseEnter(screen.getByText("会话 5").closest("div"));
  assert.equal(highlighted(), "会话 5");

  fireEvent.keyDown(window, { key: "ArrowDown" });
  fireEvent.mouseEnter(screen.getByText("会话 20").closest("div"));

  assert.equal(highlighted(), "会话 6");
});

test("检索进行中画骨架和正在搜索,不闪无结果", () => {
  renderPalette({
    results: [],
    searching: true,
    searchingLabel: "正在搜索…",
  });

  assert.ok(screen.getByText("正在搜索…"));
  assert.ok(container.querySelector("[data-searching]"));
  assert.equal(screen.queryByText("无结果"), null);
});

test("检索结束后仍无命中才显示无结果", () => {
  renderPalette({ results: [], searching: false });

  assert.ok(screen.getByText("无结果"));
  assert.equal(container.querySelector("[data-searching]"), null);
});
