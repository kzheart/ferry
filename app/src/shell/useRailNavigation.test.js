import { test } from "vitest";
import assert from "node:assert/strict";
import {
  DEFAULT_RAIL_ORDER,
  normalizeRailOrder,
  railKeys,
  reorderRailOrder,
} from "./useRailNavigation.js";

const on = () => true;
const off = () => false;

test("轨道顺序只保留已知且不重复的工作区", () => {
  assert.deepEqual(
    normalizeRailOrder(["history", "unknown", "history", "library"], on),
    ["history", "library", "overview", "askferry"],
  );
  assert.deepEqual(normalizeRailOrder(null, on), DEFAULT_RAIL_ORDER);
});

test("特性关着时它那条工作区整个不存在,存过的自定义顺序同样被过滤", () => {
  assert.deepEqual(railKeys(off), ["overview", "library", "history"]);
  assert.deepEqual(
    normalizeRailOrder(["askferry", "history"], off),
    ["history", "overview", "library"],
  );
  assert.deepEqual(normalizeRailOrder(null, off), ["overview", "library", "history"]);
  // 缺省就是关:漏传判定函数时宁可少显示一个入口
  assert.deepEqual(normalizeRailOrder(["askferry"]), ["overview", "library", "history"]);
  assert.deepEqual(railKeys(), ["overview", "library", "history"]);
});

test("轨道拖拽按目标位置重排，并忽略无效目标", () => {
  assert.deepEqual(
    reorderRailOrder(DEFAULT_RAIL_ORDER, "history", "overview", "before"),
    ["history", "overview", "askferry", "library"],
  );
  assert.equal(
    reorderRailOrder(DEFAULT_RAIL_ORDER, "history", "missing", "after"),
    DEFAULT_RAIL_ORDER,
  );
});
