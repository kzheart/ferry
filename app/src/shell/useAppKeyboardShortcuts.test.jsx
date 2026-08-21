// 全局快捷键里这次新增的两条:⌘⇧S 折叠导航栏、⌘F 聚焦资源栏那条常驻搜索框。
// ⌘K(全文命令面板)必须原样活着——三者共用同一个 keydown,很容易互相抢。
import { test } from "vitest";
import assert from "node:assert/strict";
import { fireEvent, renderHook } from "@testing-library/react";

import { useAppKeyboardShortcuts } from "./useAppKeyboardShortcuts.js";

const noop = () => {};

function setup(overrides = {}) {
  const calls = [];
  renderHook(() => useAppKeyboardShortcuts({
    paneAvailable: true,
    onOpenSearch: () => calls.push("search"),
    onToggleNav: () => calls.push("nav"),
    onFocusPaneSearch: () => calls.push("focus"),
    dismissers: [],
    view: "library",
    currentSession: null,
    multiIds: [],
    sessionsByKey: {},
    onRename: noop,
    onBatchDelete: noop,
    onDelete: noop,
    onResume: noop,
    libraryVisibleIds: [],
    historyVisibleIds: [],
    selectedSessionId: null,
    selectedHistoryId: null,
    selectSession: noop,
    selectHistory: noop,
    ...overrides,
  }));
  return calls;
}

const press = (key, init = {}) =>
  fireEvent.keyDown(document, { key, metaKey: true, ...init });

test("⌘⇧S 折叠导航栏,⌘F 聚焦常驻搜索框,⌘K 仍是全文面板", () => {
  const calls = setup();

  press("s", { shiftKey: true });
  press("f");
  press("k");

  assert.deepEqual(calls, ["nav", "focus", "search"]);
});

test("不带 Shift 的 ⌘S 不折叠导航栏", () => {
  const calls = setup();
  press("s");
  assert.deepEqual(calls, []);
});

test("没有资源栏的视图里 ⌘F / ⌘K 不动作,⌘⇧S 仍然可用", () => {
  const calls = setup({ paneAvailable: false });

  press("f");
  press("k");
  press("s", { shiftKey: true });

  assert.deepEqual(calls, ["nav"]);
});
