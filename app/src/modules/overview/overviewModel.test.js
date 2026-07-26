// 总览聚合是纯函数：所有用例固定 now，断言不依赖运行时钟。
import assert from "node:assert/strict";
import { test } from "vitest";

import {
  addTokens,
  buildPriceIndex,
  computeOverview,
  costOf,
  emptyTokens,
  matchPrice,
  sumTokens,
} from "./overviewModel.js";

// 2026-01-15 是周四，本地时区正午；周末用例依赖这一点。
const NOW = new Date(2026, 0, 15, 12, 0, 0).getTime();
const DAY = 86400e3;

// 第 offset 天前的本地零点，落在 trends/heatmap 的日桶边界上。
const dayStart = offset => new Date(2026, 0, 15 - offset).getTime();

const PRICES = {
  "gpt-test": { input: 3, output: 15, cache_read: 0.3, cache_write: 3.75 },
};

const tokens = (input, output = 0, cacheRead = 0, cacheWrite = 0) => ({
  input, output, cache_read: cacheRead, cache_write: cacheWrite,
});

const session = overrides => ({
  tool: "claude",
  dir: "/Users/me/code/ferry",
  model: "gpt-test",
  updated: dayStart(0),
  tokens: tokens(1000),
  ...overrides,
});

const insight = (result, kind) =>
  result.insights.find(item => item.kind === kind);

test("token 求和与累加忽略缺失字段", () => {
  assert.deepEqual(sumTokens(undefined), 0);
  assert.deepEqual(sumTokens(tokens(1, 2, 3, 4)), 10);
  assert.deepEqual(
    addTokens(emptyTokens(), { input: 5, output: undefined }),
    tokens(5),
  );
});

test("costOf 按百万 token 单价计算，缺价归零", () => {
  assert.equal(costOf(tokens(1_000_000), null), 0);
  assert.equal(costOf(tokens(1_000_000), PRICES["gpt-test"]), 3);
  assert.equal(
    costOf(tokens(1_000_000, 1_000_000), PRICES["gpt-test"]),
    18,
  );
  // 单价表缺某一档时该档按 0 计，不参与分摊。
  assert.equal(costOf(tokens(0, 1_000_000), { input: 3 }), 0);
});

test("buildPriceIndex 归一化模型名且首次命中优先", () => {
  const index = buildPriceIndex({
    "anthropic/claude-x": { input: 1 },
    "vip/claude-x": { input: 9 },
  });

  assert.deepEqual(Object.keys(index), ["claude-x"]);
  assert.equal(index["claude-x"].input, 1);
});

test("matchPrice 依次尝试原名、归一名与边界对齐的前缀", () => {
  const prices = {
    "claude-sonnet-4": { input: 3 },
    "gpt-5": { input: 1 },
  };

  assert.equal(matchPrice("claude-sonnet-4", prices).input, 3);
  assert.equal(matchPrice("vendor/claude-sonnet-4", prices).input, 3);
  // 长名是短名的前缀且断点落在分隔符上：视为同族。
  assert.equal(matchPrice("claude-sonnet-4-20250101", prices).input, 3);
  assert.equal(matchPrice("gpt-5-mini", prices).input, 1);
});

test("matchPrice 拒绝非边界前缀与空输入", () => {
  const prices = { "gpt-5": { input: 1 } };

  assert.equal(matchPrice("gpt-51", prices), null);
  assert.equal(matchPrice("mistral-large", prices), null);
  assert.equal(matchPrice("", prices), null);
  assert.equal(matchPrice("gpt-5", null), null);
});

test("computeOverview 按 scope 切窗并给出环比增量", () => {
  const sessions = [
    session({ updated: dayStart(1), tokens: tokens(100) }),
    session({ updated: dayStart(40), tokens: tokens(70) }),
    session({ updated: dayStart(200), tokens: tokens(5) }),
  ];

  const scoped = computeOverview({ sessions, prices: PRICES, now: NOW });
  assert.equal(scoped.kpis.sessions.value, 1);
  assert.equal(scoped.kpis.tokens.value, 100);
  assert.equal(scoped.kpis.sessions.delta, 0);
  assert.equal(scoped.kpis.tokens.delta, 30);

  const all = computeOverview({
    sessions, prices: PRICES, scope: "all", now: NOW,
  });
  assert.equal(all.kpis.sessions.value, 3);
  assert.equal(all.kpis.sessions.delta, null);
  assert.equal(all.kpis.tokens.delta, null);
});

test("computeOverview 按工具过滤会话与迁移记录", () => {
  const result = computeOverview({
    sessions: [session({}), session({ tool: "codex" })],
    history: [
      { src: "claude", dst: "codex" },
      { src: "pi", dst: "grok" },
    ],
    prices: PRICES,
    tool: "codex",
    now: NOW,
  });

  assert.equal(result.kpis.sessions.value, 1);
  assert.deepEqual(result.flows, [{ src: "claude", dst: "codex", count: 1 }]);
});

test("成本表合并同族模型并单独统计无价条目", () => {
  const result = computeOverview({
    sessions: [
      session({ model: "vip/gpt-test", tokens: tokens(1_000_000) }),
      session({ model: "gpt-test", tokens: tokens(1_000_000) }),
      session({ model: "unknown-model", tokens: tokens(500) }),
    ],
    prices: PRICES,
    now: NOW,
  });

  assert.equal(result.costRows.length, 1);
  assert.equal(result.costRows[0].model, "gpt-test");
  assert.equal(result.costRows[0].cost, 6);
  assert.deepEqual(result.unpriced, { models: 1, tokens: 500 });
  assert.equal(result.costTotal, 6);
});

test("trends 是近 14 天的日桶，窗口外的会话被丢弃", () => {
  const result = computeOverview({
    sessions: [
      session({ updated: dayStart(0), tokens: tokens(10) }),
      session({ updated: dayStart(13), tokens: tokens(20) }),
      session({ updated: dayStart(14), tokens: tokens(999) }),
      session({ updated: 0, tokens: tokens(999) }),
    ],
    prices: PRICES,
    scope: "all",
    now: NOW,
  });

  assert.equal(result.trends.tokens.length, 14);
  assert.equal(result.trends.tokens[13], 10);
  assert.equal(result.trends.tokens[0], 20);
  assert.equal(result.trends.sessions.reduce((n, v) => n + v, 0), 2);
});

test("heatmap 覆盖 52 周并把未来日标成 -1", () => {
  const result = computeOverview({
    sessions: [
      session({ updated: dayStart(0) }),
      session({ updated: dayStart(0) }),
      session({ updated: dayStart(1) }),
    ],
    prices: PRICES,
    now: NOW,
  });

  const { grid, max, total, weeks } = result.heatmap;
  assert.equal(weeks, 52);
  assert.equal(grid.length, 52);
  assert.ok(grid.every(column => column.length === 7));
  assert.equal(total, 3);
  assert.equal(max, 2);
  // 周四是本周第 4 天（周一为 0），其后三天尚未到来。
  assert.deepEqual(grid[51].slice(3), [2, -1, -1, -1]);
});

test("连续活跃天数从今天往回数，并记录历史最长", () => {
  const result = computeOverview({
    sessions: [
      session({ updated: dayStart(0) }),
      session({ updated: dayStart(1) }),
      session({ updated: dayStart(2) }),
      session({ updated: dayStart(20) }),
      session({ updated: dayStart(21) }),
    ],
    prices: PRICES,
    scope: "all",
    now: NOW,
  });

  assert.equal(result.kpis.streak.value, 3);
  assert.equal(result.kpis.streak.longest, 3);
  assert.ok(insight(result, "streak") === undefined);
});

test("costSpike 在近 7 天开销显著高于前一周时触发并附周序列", () => {
  const sessions = [
    session({ updated: dayStart(1), tokens: tokens(10_000_000) }),
    session({ updated: dayStart(8), tokens: tokens(5_000_000) }),
  ];

  const spike = insight(
    computeOverview({ sessions, prices: PRICES, now: NOW }), "cost",
  );

  assert.equal(spike.params.repo, "ferry");
  assert.equal(spike.params.cost, 30);
  assert.equal(spike.params.prev, 15);
  assert.equal(spike.params.pct, 100);
  // weeklyCost 近 5 周：最后一格是本周，倒数第二格是上一周。
  assert.deepEqual(spike.params.weeks, [0, 0, 0, 15, 30]);
});

test("costSpike 在上周无开销、涨幅不足或金额过小时都不触发", () => {
  const noBaseline = [
    session({ updated: dayStart(1), tokens: tokens(10_000_000) }),
  ];
  const flat = [
    session({ updated: dayStart(1), tokens: tokens(10_000_000) }),
    session({ updated: dayStart(8), tokens: tokens(9_000_000) }),
  ];
  const tiny = [
    session({ updated: dayStart(1), tokens: tokens(1_000_000) }),
    session({ updated: dayStart(8), tokens: tokens(100_000) }),
  ];

  for (const sessions of [noBaseline, flat, tiny]) {
    assert.equal(
      insight(computeOverview({ sessions, prices: PRICES, now: NOW }), "cost"),
      undefined,
    );
  }
});

test("weekendSilent 只看最近一个完整周末是否有会话", () => {
  const silent = computeOverview({
    sessions: [session({ updated: dayStart(1) })],
    prices: PRICES,
    now: NOW,
  });
  // 2026-01-15 是周四，最近一个完整周末是 1 月 10（六）与 11（日）。
  const active = computeOverview({
    sessions: [session({ updated: new Date(2026, 0, 10).getTime() })],
    prices: PRICES,
    now: NOW,
  });

  assert.ok(insight(silent, "weekend"));
  assert.equal(insight(active, "weekend"), undefined);
});

test("洞察池按优先级排序且兜底卡在数据稀薄时补位", () => {
  const result = computeOverview({
    sessions: [session({ updated: dayStart(1) })],
    prices: PRICES,
    now: NOW,
  });

  const priorities = result.insights.map(item => item.priority);
  assert.deepEqual(priorities, [...priorities].sort((a, b) => b - a));
  assert.ok(insight(result, "topRepo"));
  assert.ok(insight(result, "topModel"));
  assert.ok(insight(result, "avgDaily"));
});

test("空数据集给出 empty 标记且不产生 NaN", () => {
  const result = computeOverview({ now: NOW });

  assert.equal(result.empty, true);
  assert.equal(result.hasUsage, false);
  assert.equal(result.costTotal, 0);
  assert.equal(result.bump, null);
  assert.equal(result.nightShare, 0);
  assert.deepEqual(result.repos, []);
  assert.ok(result.composition.every(item => item.pct === 0));
});
