// 特性开关框架自己的护栏:声明式过滤的语义,以及「宿主是唯一事实源」这条——
// 前端只抄一份显示副本,契约只提供默认值。
import { expect, test, vi } from "vitest";
import { act, render } from "@testing-library/react";

import { FEATURES } from "../contracts/generated/features.js";
import {
  filterByFeatures,
  refreshFeatures,
  useFeature,
  useFeaturesList,
  useIsFeatureEnabled,
} from "./features.jsx";

const host = vi.hoisted(() => ({ states: null }));

vi.mock("../../platform/desktop/client.js", async (importOriginal) => ({
  ...(await importOriginal()),
  featuresList: async () => {
    if (!host.states) throw new Error("宿主不可用");
    return host.states;
  },
}));

test("没标 feature 的表项恒显示,标了的跟着开关走", () => {
  const items = [
    { key: "a" },
    { key: "b", feature: "builtin-agent" },
    { key: "c", feature: "something-else" },
  ];
  const only = (id) => (feature) => feature === id;

  expect(filterByFeatures(items, only("builtin-agent")).map((i) => i.key))
    .toEqual(["a", "b"]);
  expect(filterByFeatures(items, () => false).map((i) => i.key)).toEqual(["a"]);
  expect(filterByFeatures(items, () => true).map((i) => i.key))
    .toEqual(["a", "b", "c"]);
  expect(filterByFeatures([], () => true)).toEqual([]);
});

test("回读不到宿主时停在契约默认,不擅自把入口打开", async () => {
  host.states = null;
  let seen = null;
  function Probe() {
    seen = useFeaturesList();
    return null;
  }
  render(<Probe />);
  await act(async () => { await refreshFeatures(); });

  expect(seen.map((feature) => feature.id))
    .toEqual(FEATURES.map((feature) => feature.id));
  for (const feature of seen) expect(feature.enabled).toBe(feature.default);
});

test("宿主的快照落地后,全部消费点同步看到新值", async () => {
  host.states = [
    { id: "builtin-agent", stage: "experimental", default: false, enabled: true },
  ];
  let flag = null;
  let predicate = null;
  function Probe() {
    flag = useFeature("builtin-agent");
    predicate = useIsFeatureEnabled();
    return null;
  }
  render(<Probe />);
  await act(async () => { await refreshFeatures(); });

  expect(flag).toBe(true);
  expect(predicate("builtin-agent")).toBe(true);
  // 契约里没有的 id 一律当关:表里漏标不会意外放行
  expect(predicate("never-declared")).toBe(false);
});
