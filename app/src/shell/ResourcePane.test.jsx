// 资源栏的两处交互细节:选中行的滚动跟随,以及行内重命名的失焦语义。
// 列表是虚拟化的,视口外的行不挂载——滚动跟随一旦缺失,键盘 ↑/↓ 翻过视口后
// 高亮就"消失"了(既不在屏幕上,也没有 DOM 可循)。
import { test } from "vitest";
import assert from "node:assert/strict";
import { fireEvent, render, screen } from "@testing-library/react";

import { LibraryList, nextFocusScrollTop } from "./ResourcePane.jsx";

// ---- 滚动跟随的位置计算(jsdom 不做布局,只能在纯函数上断言) ----

const viewport = { scrollTop: 100, viewHeight: 200 }; // 视口覆盖 100..300

test("目标行已在视口内时不滚动,不打断用户自己的滚动位置", () => {
  const next = nextFocusScrollTop({ itemTop: 150, itemHeight: 30, ...viewport });
  assert.equal(next, null);
});

test("目标行在视口上方时贴上沿,留出一点余白", () => {
  const next = nextFocusScrollTop({ itemTop: 40, itemHeight: 30, ...viewport });
  assert.equal(next, 32); // 40 - 8
});

test("目标行在视口下方时贴下沿,行底完整可见", () => {
  const next = nextFocusScrollTop({ itemTop: 400, itemHeight: 30, ...viewport });
  assert.equal(next, 238); // 400 + 30 - 200 + 8
});

test("列表顶部的行不会算出负的 scrollTop", () => {
  const next = nextFocusScrollTop({
    itemTop: 0, itemHeight: 30, scrollTop: 50, viewHeight: 200,
  });
  assert.equal(next, 0);
});

// ---- 行内重命名:失焦即提交,而不是丢弃已输入的内容 ----

const row = { key: "claude:a", tool: "claude", title: "原标题", dir: "/tmp", active: "刚刚" };

function renderRenaming(handlers) {
  const groups = [{ key: "today", label: "今天", count: 1, rows: [row] }];
  return render(
    <LibraryList
      groups={groups}
      collapsed={{}}
      onToggle={() => {}}
      empty={false}
      onClear={() => {}}
      selectedId={row.key}
      multiSel={[]}
      renamingKey={row.key}
      onRowClick={() => {}}
      onRowPin={() => {}}
      onRowDelete={() => {}}
      onRowMore={() => {}}
      onRowRename={() => {}}
      onRowRenameSubmit={() => {}}
      onRowRenameCancel={() => {}}
      {...handlers}
    />,
  );
}

test("重命名输入框失焦时提交已输入的内容,而不是丢弃", () => {
  const submitted = [];
  let cancelled = false;
  renderRenaming({
    onRowRenameSubmit: (key, value) => submitted.push([key, value]),
    onRowRenameCancel: () => { cancelled = true; },
  });

  const input = screen.getByPlaceholderText("app:prompt.renamePlaceholder");
  fireEvent.change(input, { target: { value: "新标题" } });
  fireEvent.blur(input);

  assert.deepEqual(submitted, [["claude:a", "新标题"]]);
  assert.equal(cancelled, false);
});

test("Esc 取消后紧随的失焦不再提交一次", () => {
  const submitted = [];
  let cancelled = false;
  renderRenaming({
    onRowRenameSubmit: (key, value) => submitted.push([key, value]),
    onRowRenameCancel: () => { cancelled = true; },
  });

  const input = screen.getByPlaceholderText("app:prompt.renamePlaceholder");
  fireEvent.change(input, { target: { value: "半途改的名字" } });
  fireEvent.keyDown(input, { key: "Escape" });
  fireEvent.blur(input);

  assert.equal(cancelled, true);
  assert.deepEqual(submitted, []);
});

test("Enter 提交后紧随的失焦不会重复提交", () => {
  const submitted = [];
  renderRenaming({ onRowRenameSubmit: (key, value) => submitted.push([key, value]) });

  const input = screen.getByPlaceholderText("app:prompt.renamePlaceholder");
  fireEvent.change(input, { target: { value: "新标题" } });
  fireEvent.keyDown(input, { key: "Enter" });
  fireEvent.blur(input);

  assert.deepEqual(submitted, [["claude:a", "新标题"]]);
});
