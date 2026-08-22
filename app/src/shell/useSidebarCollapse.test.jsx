// 侧栏收起态:记住用户的选择;收起后没有任何常驻入口,所以默认永远是展开的。
import { beforeEach, test } from "vitest";
import assert from "node:assert/strict";
import { act, renderHook } from "@testing-library/react";

import { useSidebarCollapse } from "./useSidebarCollapse.js";

beforeEach(() => localStorage.clear());

test("没有记录时侧栏是展开的,窗口多窄都不自动收起", () => {
  Object.defineProperty(window, "innerWidth", { value: 720, configurable: true });
  const { result } = renderHook(() => useSidebarCollapse());
  assert.equal(result.current.collapsed, false);
});

test("切换过之后按记录来,重开还是那个样子", () => {
  const first = renderHook(() => useSidebarCollapse());
  act(() => first.result.current.toggle());
  assert.equal(first.result.current.collapsed, true);
  first.unmount();

  const second = renderHook(() => useSidebarCollapse());
  assert.equal(second.result.current.collapsed, true);
  act(() => second.result.current.toggle());
  assert.equal(second.result.current.collapsed, false);
  second.unmount();

  assert.equal(renderHook(() => useSidebarCollapse()).result.current.collapsed, false);
});
