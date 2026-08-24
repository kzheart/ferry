// 总览页冒烟:用量走势/Agent 对比替换旧的 bump/迁移流向,热力图点击出当天下钻。
// cimode 下 t() 原样返回 key,断言绑定结构而非文案。
import assert from "node:assert/strict";
import { test } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

import zhCN from "../../shared/i18n/locales/zh-CN/overview.json";
import en from "../../shared/i18n/locales/en/overview.json";
import Overview from "./Overview.jsx";

const PRICES = { "gpt-test": { input: 3, output: 15, cache_read: 0.3, cache_write: 3.75 } };
const tokens = input => ({ input, output: 0, cache_read: 0, cache_write: 0 });

const session = overrides => ({
  tool: "claude", dir: "/Users/me/code/ferry", model: "gpt-test",
  updated: Date.now(), created: Date.now(), tokens: tokens(1000), ...overrides,
});

const renderOverview = sessions => render(
  <Overview sessions={sessions} prices={PRICES} pricing={null} scanning={false} navigationTarget={null} />,
);

test("新旧区块的 i18n key 两种语言对齐,旧 key 已删", () => {
  for (const block of ["daily", "day", "agents"]) {
    assert.deepEqual(Object.keys(zhCN[block]).sort(), Object.keys(en[block]).sort());
  }
  for (const locale of [zhCN, en]) {
    assert.equal(locale.bump, undefined);
    assert.equal(locale.flow, undefined);
  }
});

test("渲染用量走势与 Agent 对比,不再渲染 bump/迁移流向", () => {
  renderOverview([
    session(),
    session({ tool: "codex", tokens: tokens(500) }),
  ]);

  assert.ok(screen.getByText("overview:daily.title"));
  assert.ok(screen.getByRole("img", { name: "overview:daily.aria" }));
  assert.ok(screen.getByText("overview:agents.title"));
  assert.equal(screen.queryByText("overview:bump.title"), null);
  assert.equal(screen.queryByText("overview:flow.title"), null);
});

test("点热力图今天的格子展开当天明细,收起后消失", () => {
  renderOverview([session(), session({ tool: "codex", tokens: tokens(3000) })]);

  const heat = screen.getByRole("img", { name: "overview:heat.aria" });
  const cells = heat.querySelectorAll("rect");
  fireEvent.click(cells[cells.length - 1]);   // 未来日不渲染,最后一格恒为今天

  assert.ok(screen.getByText("overview:day.sessions"));
  assert.ok(screen.getByText("overview:day.topModels"));

  fireEvent.click(screen.getByRole("button", { name: "overview:day.close" }));
  assert.equal(screen.queryByText("overview:day.sessions"), null);
});

test("无用量时 Agent 对比给出空态而不是空卡", () => {
  renderOverview([session({ tokens: tokens(0) })]);

  assert.ok(screen.getByText("overview:agents.empty"));
});
