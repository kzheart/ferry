// 总览聚合是纯函数：所有用例固定 now，断言不依赖运行时钟。
import assert from "node:assert/strict";
import { test } from "vitest";

import {
  addTokens,
  buildPriceIndex,
  computeDayDetail,
  computeOverview,
  computeToday,
  costOf,
  emptyTokens,
  heatLevel,
  matchPrice,
  modelUsageOf,
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

test("matchPrice 只接受完整 key 或裸模型精确匹配", () => {
  const prices = {
    "claude-sonnet-4": { input: 3 },
    "gpt-5": { input: 1 },
  };

  assert.equal(matchPrice("claude-sonnet-4", prices).input, 3);
  assert.equal(matchPrice("vendor/claude-sonnet-4", prices).input, 3);
  assert.equal(matchPrice("claude-sonnet-4-20250101", prices), null);
  assert.equal(matchPrice("gpt-5-mini", prices), null);
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

test("computeOverview 按工具过滤会话", () => {
  const result = computeOverview({
    sessions: [session({}), session({ tool: "codex" })],
    prices: PRICES,
    tool: "codex",
    now: NOW,
  });

  assert.equal(result.kpis.sessions.value, 1);
  assert.deepEqual(result.daily.tools, ["codex"]);
});

test("按模型分桶优先于会话代表模型，成本和排行不误归父模型", () => {
  const mixed = session({
    model: "gpt-test",
    tokens: tokens(3_000_000),
    usage_by_model: {
      "gpt-test": tokens(1_000_000),
      "other-model": tokens(2_000_000),
    },
  });
  assert.deepEqual(modelUsageOf(mixed), [
    { model: "gpt-test", tokens: tokens(1_000_000) },
    { model: "other-model", tokens: tokens(2_000_000) },
  ]);

  const result = computeOverview({ sessions: [mixed], prices: PRICES, now: NOW });
  assert.equal(result.kpis.tokens.value, 3_000_000);
  assert.equal(result.costTotal, 3);
  assert.equal(result.costRows[0].model, "gpt-test");
  assert.deepEqual(result.unpriced, { models: 1, tokens: 2_000_000 });
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
});

test("computeToday 只取今天的会话，并按 agent/模型/项目拆分", () => {
  const today = computeToday({
    sessions: [
      session({ updated: dayStart(0), created: dayStart(0), tokens: tokens(600) }),
      session({ updated: dayStart(0), created: dayStart(3), tool: "codex",
        dir: "/Users/me/code/blog", tokens: tokens(400) }),
      session({ updated: dayStart(1), tokens: tokens(9999) }),
    ],
    prices: PRICES,
    now: NOW,
  });

  assert.equal(today.empty, false);
  assert.equal(today.sessions, 2);
  assert.equal(today.created, 1);            // 今天新建的只有第一个
  assert.equal(today.tokens, 1000);          // 昨天那条不计入
  assert.deepEqual(today.byAgent.map(a => [a.tool, a.tokens, a.pct]), [
    ["claude", 600, 60], ["codex", 400, 40],
  ]);
  // 同票按名称排,保证渲染顺序稳定
  assert.deepEqual(today.byProject, [
    { name: "blog", sessions: 1 }, { name: "ferry", sessions: 1 },
  ]);
  assert.equal(today.topModel, "gpt-test");
  assert.equal(today.topModelPct, 100);
  assert.equal(today.cost, costOf(tokens(1000), PRICES["gpt-test"]));
  assert.deepEqual(today.costByModel, [{ model: "gpt-test", cost: today.cost }]);
});

test("computeToday 的昨日对比截到同一钟点，昨日为空时百分比退化为 null", () => {
  // NOW 是正午；昨天上午的会话进对比窗，昨天下午的不进。
  const morning = dayStart(1) + 9 * 3600e3;
  const evening = dayStart(1) + 20 * 3600e3;
  const compared = computeToday({
    sessions: [
      session({ updated: dayStart(0), tokens: tokens(150) }),
      session({ updated: morning, tokens: tokens(100) }),
      session({ updated: evening, tokens: tokens(9999) }),
    ],
    prices: PRICES,
    now: NOW,
  }).compare;

  assert.deepEqual(compared.current, { sessions: 1, tokens: 150, cost: compared.current.cost });
  assert.equal(compared.yesterday.sessions, 1);
  assert.equal(compared.yesterday.tokens, 100);
  assert.equal(compared.tokensPct, 50);

  const noBaseline = computeToday({
    sessions: [session({ updated: dayStart(0) })], prices: PRICES, now: NOW,
  }).compare;
  assert.equal(noBaseline.tokensPct, null);
  assert.equal(noBaseline.costPct, null);
});

test("computeToday 的近 7 天点阵以今天收尾，色阶与热力图同档", () => {
  const { week } = computeToday({
    sessions: [
      session({ updated: dayStart(0) }),
      session({ updated: dayStart(3) }), session({ updated: dayStart(3) }),
      session({ updated: dayStart(9) }),      // 窗口外
    ],
    prices: PRICES,
    now: NOW,
  });

  assert.equal(week.length, 7);
  assert.equal(week[6].day, dayStart(0));
  assert.deepEqual(week.map(d => d.count), [0, 0, 0, 2, 0, 0, 1]);
  assert.deepEqual(week.map(d => d.level), [0, 0, 0, 4, 0, 0, 2]);
  assert.equal(heatLevel(0, 5), 0);
  assert.equal(heatLevel(5, 5), 4);
});

test("computeToday 今天没有会话时给出 empty 且各项归零", () => {
  const today = computeToday({
    sessions: [session({ updated: dayStart(2) })], prices: PRICES, now: NOW,
  });

  assert.equal(today.empty, true);
  assert.equal(today.sessions, 0);
  assert.equal(today.tokens, 0);
  assert.equal(today.cost, 0);
  assert.equal(today.topModel, null);
  assert.deepEqual(today.byAgent, []);
  assert.deepEqual(today.byProject, []);
  assert.ok(today.composition.every(c => c.value === 0));
});

test("computeOverview 的 today 不随 scope 变化，但跟随 agent 筛选", () => {
  const sessions = [
    session({ updated: dayStart(0), tokens: tokens(100) }),
    session({ updated: dayStart(0), tool: "codex", tokens: tokens(300) }),
  ];
  const base = { sessions, prices: PRICES, now: NOW };

  assert.equal(computeOverview({ ...base, scope: "7" }).today.tokens, 400);
  assert.equal(computeOverview({ ...base, scope: "all" }).today.tokens, 400);
  assert.equal(computeOverview({ ...base, tool: "codex" }).today.tokens, 300);
});

test("空数据集给出 empty 标记且不产生 NaN", () => {
  const result = computeOverview({ now: NOW });

  assert.equal(result.empty, true);
  assert.equal(result.hasUsage, false);
  assert.equal(result.costTotal, 0);
  assert.deepEqual(result.daily.tools, []);
  assert.deepEqual(result.agentShare, []);
  assert.equal(result.nightShare, 0);
  assert.deepEqual(result.repos, []);
  assert.ok(result.composition.every(item => item.pct === 0));
});

test("daily 按日分桶并按 agent 拆分,桶数跟随 scope", () => {
  const sessions = [
    session({ updated: dayStart(0), tokens: tokens(100) }),
    session({ updated: dayStart(0), tool: "codex", tokens: tokens(50) }),
    session({ updated: dayStart(2), tokens: tokens(30) }),
    session({ updated: dayStart(10), tokens: tokens(999) }),   // 7 天窗口外
  ];
  const { daily } = computeOverview({ sessions, prices: PRICES, scope: "7", now: NOW });

  assert.equal(daily.unit, "day");
  assert.equal(daily.buckets.length, 7);
  assert.equal(daily.buckets[6].start, dayStart(0));
  assert.equal(daily.buckets[6].tokens, 150);
  assert.equal(daily.buckets[6].sessions, 2);
  assert.equal(daily.buckets[6].byTool.claude.tokens, 100);
  assert.equal(daily.buckets[6].byTool.codex.tokens, 50);
  assert.equal(daily.buckets[4].tokens, 30);
  assert.equal(daily.maxTokens, 150);
  // 堆叠顺序:token 总量大的在前
  assert.deepEqual(daily.tools, ["claude", "codex"]);
});

test("daily 在 scope=all 时按周分桶,首桶为最早会话所在周", () => {
  const sessions = [
    session({ updated: dayStart(0), tokens: tokens(100) }),
    session({ updated: dayStart(15), tokens: tokens(70) }),
  ];
  const { daily } = computeOverview({ sessions, prices: PRICES, scope: "all", now: NOW });

  assert.equal(daily.unit, "week");
  assert.equal(daily.truncated, false);
  // 2026-01-15 是周四:本周始于 1-12;15 天前(2025-12-31)在 3 周前
  assert.equal(daily.buckets.length, 3);
  assert.equal(daily.buckets[0].tokens, 70);
  assert.equal(daily.buckets[2].tokens, 100);
});

test("agentShare 给出 token 占比与环比百分点,scope=all 时无环比", () => {
  const sessions = [
    session({ updated: dayStart(1), tokens: tokens(300) }),
    session({ updated: dayStart(1), tool: "codex", tokens: tokens(100) }),
    // 上一 30 天周期:claude 50%、codex 50%
    session({ updated: dayStart(40), tokens: tokens(100) }),
    session({ updated: dayStart(40), tool: "codex", tokens: tokens(100) }),
  ];
  const { agentShare } = computeOverview({ sessions, prices: PRICES, now: NOW });

  assert.equal(agentShare.length, 2);
  assert.equal(agentShare[0].tool, "claude");
  assert.equal(agentShare[0].pct, 75);
  assert.equal(agentShare[0].delta, 25);
  assert.equal(agentShare[1].tool, "codex");
  assert.equal(agentShare[1].delta, -25);

  const all = computeOverview({ sessions, prices: PRICES, scope: "all", now: NOW });
  assert.ok(all.agentShare.every(row => row.delta === null));
});

test("computeDayDetail 只取当天会话并拆分 agent 与 Top 模型", () => {
  const detail = computeDayDetail({
    sessions: [
      session({ updated: dayStart(1), tokens: tokens(1_000_000) }),
      session({
        updated: dayStart(1), tool: "codex", model: "other-model",
        tokens: tokens(3_000_000),
      }),
      session({ updated: dayStart(0), tokens: tokens(999) }),   // 不在选中日
    ],
    prices: PRICES,
    day: dayStart(1),
  });

  assert.equal(detail.sessions, 2);
  assert.equal(detail.tokens, 4_000_000);
  assert.equal(detail.cost, costOf(tokens(1_000_000), PRICES["gpt-test"]));
  assert.deepEqual(detail.byAgent.map(a => a.tool), ["codex", "claude"]);
  assert.equal(detail.byAgent[0].pct, 75);
  assert.deepEqual(detail.topModels.map(m => m.model), ["other-model", "gpt-test"]);
});

test("computeDayDetail 空日给出零值且不产生 NaN", () => {
  const detail = computeDayDetail({ sessions: [], prices: PRICES, day: dayStart(0) });

  assert.equal(detail.sessions, 0);
  assert.equal(detail.tokens, 0);
  assert.equal(detail.cost, 0);
  assert.deepEqual(detail.byAgent, []);
  assert.deepEqual(detail.topModels, []);
});
