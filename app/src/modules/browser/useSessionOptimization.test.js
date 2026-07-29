import { expect, test } from "vitest";
import {
  SESSION_OPTIMIZATION_PURPOSE,
  SESSION_OPTIMIZER_ROLE_ID,
  buildOptimizationInstruction,
  isOptimizerRole,
  parseReasons,
  stripReasonsLine,
} from "./useSessionOptimization.js";

test("常量与 runtime 契约一致", () => {
  expect(SESSION_OPTIMIZATION_PURPOSE).toBe("session-optimization");
  expect(SESSION_OPTIMIZER_ROLE_ID).toBe("session-optimizer");
});

test("优化器角色判定:显式标记 + 读写会话工具,缺一不可", () => {
  expect(isOptimizerRole({
    optimizer: true, tools: ["session_read", "session_edit"],
  })).toBe(true);
  // 只有工具没有标记:不再按工具推断
  expect(isOptimizerRole({ tools: ["session_read", "session_edit"] })).toBe(false);
  // 只有标记没有工具:跑不出 preview,依然不合格
  expect(isOptimizerRole({ optimizer: true, tools: ["session_read"] })).toBe(false);
  expect(isOptimizerRole({ optimizer: true, tools: [] })).toBe(false);
  expect(isOptimizerRole({})).toBe(false);
});

test("指令生成:整段与指定轮次两种口径,且只允许 preview", () => {
  const whole = buildOptimizationInstruction();
  expect(whole).toContain("通读全部轮次");
  expect(whole).toContain("preview");
  expect(whole).toContain("禁止调用 execute");

  const scoped = buildOptimizationInstruction([3, 7]);
  expect(scoped).toContain("第 3、7 轮");
  expect(scoped).not.toContain("通读全部轮次");
  // 点名的轮次是明确请求,指令要求每轮给出候选
  expect(scoped).toContain("每一轮都应给出改写候选");
});

test("空结果解释:剥掉 REASONS 行,保留 Agent 正文并截断", () => {
  const text = [
    "各轮提问都已足够清晰,无需改写。",
    'REASONS: {"reasons":[]}',
  ].join("\n");
  expect(stripReasonsLine(text)).toBe("各轮提问都已足够清晰,无需改写。");
  expect(stripReasonsLine("")).toBe("");
  expect(stripReasonsLine("x".repeat(500))).toHaveLength(300);
});

test("REASONS 解析:取最后一行,坏 JSON 与缺字段都安全回退", () => {
  const text = [
    "分析完成。",
    'REASONS: {"reasons":[{"locator":"fml_a","reason":"缺少上下文"},{"locator":"fml_b","reason":"指代不明"}]}',
  ].join("\n");
  expect(parseReasons(text)).toEqual({
    fml_a: "缺少上下文",
    fml_b: "指代不明",
  });

  expect(parseReasons("REASONS: {bad json")).toEqual({});
  expect(parseReasons("没有理由行")).toEqual({});
  expect(parseReasons('REASONS: {"reasons":[{"locator":1}]}')).toEqual({});
  expect(parseReasons("")).toEqual({});
});
