// 导航栏折叠态:记住用户的选择,没有记录时按窗口宽度决定第一次的样子。
import { afterEach, beforeEach, test } from "vitest";
import assert from "node:assert/strict";
import { act, renderHook } from "@testing-library/react";

import { NARROW_WIDTH, useNavCollapse } from "./useNavCollapse.js";

const originalWidth = window.innerWidth;
const setWidth = value => Object.defineProperty(window, "innerWidth", {
  value, configurable: true, writable: true,
});

beforeEach(() => localStorage.clear());
afterEach(() => setWidth(originalWidth));

test("宽窗口首次打开时导航栏是展开的", () => {
  setWidth(NARROW_WIDTH + 200);
  const { result } = renderHook(() => useNavCollapse());
  assert.equal(result.current.collapsed, false);
});

test("窄窗口首次打开时先收起来,把宽度让给主区", () => {
  setWidth(NARROW_WIDTH - 1);
  const { result } = renderHook(() => useNavCollapse());
  assert.equal(result.current.collapsed, true);
});

test("用户手动切换过之后就按记录来,不再看窗口宽度", () => {
  setWidth(NARROW_WIDTH + 200);
  const first = renderHook(() => useNavCollapse());
  act(() => first.result.current.toggle());
  assert.equal(first.result.current.collapsed, true);
  first.unmount();

  setWidth(NARROW_WIDTH - 1);
  const second = renderHook(() => useNavCollapse());
  assert.equal(second.result.current.collapsed, true);
  act(() => second.result.current.toggle());
  assert.equal(second.result.current.collapsed, false);
  second.unmount();

  // 窄窗口也不该把用户展开过的导航栏又收回去
  const third = renderHook(() => useNavCollapse());
  assert.equal(third.result.current.collapsed, false);
});

test("阈值跟着「标准」密度的栏宽走:240 + 300 + 主区 580 才不算窄", () => {
  // 数字本身是承重的:阈值低于导航 + 资源栏 + 主区最低宽,窄窗口就会挤成三条缝
  assert.equal(NARROW_WIDTH, 1200);
  assert.ok(NARROW_WIDTH >= 240 + 300 + 580);
});
