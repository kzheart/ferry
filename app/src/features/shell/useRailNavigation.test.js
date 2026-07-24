import test from "node:test";
import assert from "node:assert/strict";
import { DEFAULT_RAIL_ORDER, normalizeRailOrder, reorderRailOrder } from "./useRailNavigation.js";

test("轨道顺序只保留已知且不重复的工作区", () => {
  assert.deepEqual(
    normalizeRailOrder(["history", "unknown", "history", "library"]),
    ["history", "library", "overview", "askferry"],
  );
  assert.deepEqual(normalizeRailOrder(null), DEFAULT_RAIL_ORDER);
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
