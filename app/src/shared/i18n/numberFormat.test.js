import assert from "node:assert/strict";
import { test } from "vitest";

import {
  formatCompactNumber,
  formatCurrency,
  formatInteger,
} from "./numberFormat.js";

test("中文紧凑数字使用万亿进位", () => {
  assert.equal(formatCompactNumber(25_780_000_000, "zh-CN"), "257.8亿");
  assert.equal(formatCompactNumber(38_040_621_876, "zh-CN"), "380.41亿");
  assert.equal(formatCompactNumber(10_000, "zh-CN"), "1万");
});

test("英文紧凑数字使用 K/M/B", () => {
  assert.equal(formatCompactNumber(25_780_000_000, "en-US"), "25.78B");
  assert.equal(formatCompactNumber(1_250_000, "en-US"), "1.25M");
});

test("整数和美元成本跟随 locale", () => {
  assert.equal(formatInteger(2016, "zh-CN"), "2,016");
  assert.match(formatCurrency(11394, "en-US"), /^\$11,394$/);
});
