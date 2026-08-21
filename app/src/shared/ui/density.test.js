// 密度表是 CSS 变量之外的第二份事实源(虚拟列表要数值),两边对不上列表就会错位。
// 这里守的是读写归一化,以及「标准比紧凑每一项都更大」这条不变式。
import { beforeEach, test } from "vitest";
import assert from "node:assert/strict";

import {
  DEFAULT_DENSITY,
  DENSITY_METRICS,
  normalizeDensity,
  readDensity,
  writeDensity,
} from "./density.js";

beforeEach(() => {
  localStorage.clear();
  delete document.documentElement.dataset.density;
});

test("默认是标准(大气),存量的坏值也回落到它", () => {
  assert.equal(DEFAULT_DENSITY, "standard");
  assert.equal(readDensity(), "standard");
  assert.equal(normalizeDensity("roomy"), "standard");
  assert.equal(normalizeDensity(null), "standard");
  assert.equal(normalizeDensity("compact"), "compact");
});

test("写入同时落盘、打到根节点上并广播一次", () => {
  const seen = [];
  const listen = event => seen.push(event.detail);
  window.addEventListener("ferry-density-change", listen);
  writeDensity("compact");
  window.removeEventListener("ferry-density-change", listen);

  assert.equal(localStorage.getItem("ferry-density"), "compact");
  assert.equal(document.documentElement.dataset.density, "compact");
  assert.deepEqual(seen, ["compact"]);
});

test("标准的每一项都不小于紧凑,否则「大气」这个词就名不副实", () => {
  const { standard, compact } = DENSITY_METRICS;
  for (const key of Object.keys(standard)) {
    assert.ok(standard[key] > compact[key], `${key}: ${standard[key]} 应大于 ${compact[key]}`);
  }
});

test("资源栏默认宽落在同密度的上下限之间", () => {
  for (const m of Object.values(DENSITY_METRICS)) {
    assert.ok(m.paneMin <= m.paneDefault && m.paneDefault <= m.paneMax);
  }
});
