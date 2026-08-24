// 今日速览:细项只在悬浮层里,所以既要断言"平时不显示",也要断言"悬停/聚焦后显示"。
// 文案走真实 locale 文件,漏加 key 会渲染成裸 key 而被断言抓到。
import assert from "node:assert/strict";
import { test } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

import { TOOL_NAME } from "../../shared/contracts/tools.js";
import zhCN from "../../shared/i18n/locales/zh-CN/overview.json";
import en from "../../shared/i18n/locales/en/overview.json";
import { computeToday } from "./overviewModel.js";
import TodayPanel from "./TodayPanel.jsx";

const NOW = new Date(2026, 0, 15, 12, 0, 0).getTime();
const dayStart = offset => new Date(2026, 0, 15 - offset).getTime();
const PRICES = { "gpt-test": { input: 3, output: 15, cache_read: 0.3, cache_write: 3.75 } };
const tokens = input => ({ input, output: 0, cache_read: 0, cache_write: 0 });

// 极简 t：只从 zh-CN 取值并插值，key 缺失时原样返回，便于断言漏翻译
const t = (key, vars) => {
  const value = key.replace(/^overview:/, "").split(".")
    .reduce((node, part) => (node == null ? node : node[part]), zhCN);
  if (typeof value !== "string") return key;
  return value.replace(/\{\{(\w+)\}\}/g, (_, name) => String(vars?.[name] ?? ""));
};

const renderPanel = (sessions, streak = 3) => {
  const today = computeToday({ sessions, prices: PRICES, now: NOW });
  return render(
    <TodayPanel today={today} streak={streak} locale="zh-CN" t={t}
      fmtInt={String} fmtTokens={String} fmtCost={v => `$${v.toFixed(2)}`} />,
  );
};

const session = overrides => ({
  tool: "claude", dir: "/Users/me/code/ferry", model: "gpt-test",
  updated: dayStart(0), created: dayStart(0), tokens: tokens(1000), ...overrides,
});

test("两种语言的 today 文案 key 完全对齐", () => {
  assert.deepEqual(Object.keys(zhCN.today).sort(), Object.keys(en.today).sort());
  assert.ok(Object.values(zhCN.today).every(v => typeof v === "string" && v.length));
  assert.ok(Object.values(en.today).every(v => typeof v === "string" && v.length));
});

test("指标只显示数值，明细要悬停才出现", async () => {
  renderPanel([
    session({ tokens: tokens(600) }),
    session({ tool: "codex", dir: "/Users/me/code/blog", tokens: tokens(400) }),
  ]);

  // 值下方不再挂说明文案,平时只有数值本身
  assert.ok(screen.getByText("1000"));
  // 明细在悬停前不在文档里
  assert.equal(screen.queryByText("其中 2 个今日新建"), null);
  assert.equal(screen.queryByText("各 Agent 用量"), null);
  assert.equal(screen.queryByText(TOOL_NAME.claude), null);

  fireEvent.mouseEnter(screen.getByText("Token 总量").parentElement);
  assert.ok(screen.getByText("各 Agent 用量"));
  assert.ok(screen.getByText(TOOL_NAME.claude));
  assert.ok(screen.getByText(TOOL_NAME.codex));

  fireEvent.mouseLeave(screen.getByText("Token 总量").parentElement);
  assert.equal(screen.queryByText("各 Agent 用量"), null);

  // 今日新建的条数移进了项目悬浮层,不再有独立的备注行
  fireEvent.mouseEnter(screen.getByText("活跃会话").parentElement);
  assert.ok(screen.getByText("其中 2 个今日新建"));
});

test("悬浮层文字固定走白色系，避免暗色主题下深底叠深字", () => {
  renderPanel([session({})]);

  fireEvent.mouseEnter(screen.getByText("活跃会话").parentElement);
  const tip = screen.getByRole("tooltip");
  // --tooltip 亮/暗两套主题都是深底,前景一旦引用 --tx*/--accent-fg 就会在暗色下翻黑
  assert.equal(tip.style.color, "rgb(255, 255, 255)");
  assert.equal(tip.style.background, "var(--tooltip)");
});

test("键盘聚焦同样能唤出明细，明细不只鼠标可达", () => {
  renderPanel([session({})]);

  const metric = screen.getByText("估算费用").parentElement;
  assert.equal(metric.tabIndex, 0);
  fireEvent.focus(metric);
  assert.ok(screen.getByText("各模型估算费用"));
  fireEvent.blur(metric);
  assert.equal(screen.queryByText("各模型估算费用"), null);
});

test("昨日同时段无基线时对比退化为占位而不是 NaN%", () => {
  renderPanel([session({})]);

  const metric = screen.getByText("对比昨日同时段").parentElement;
  assert.ok(metric.textContent.includes("—"));
  assert.ok(!/NaN|Infinity/.test(metric.textContent));
});

test("昨日基数极小导致百分比飙升时截顶显示", () => {
  renderPanel([
    session({ tokens: tokens(1_000_000) }),
    session({ updated: dayStart(1) + 9 * 3600e3, tokens: tokens(100) }),
  ]);

  const metric = screen.getByText("对比昨日同时段").parentElement;
  assert.ok(metric.textContent.includes(">999%"));
});

test("今天没有会话时给出保底文案，不隐藏整条", () => {
  renderPanel([session({ updated: dayStart(2), created: dayStart(2) })], 5);

  assert.ok(screen.getByText("今日速览"));
  assert.ok(screen.getByText("连续活跃第 5 天"));
  assert.ok(screen.getByText("今天还没有会话 —— 连续活跃 5 天,别断了。"));
  assert.equal(screen.queryByText("活跃会话"), null);
});
