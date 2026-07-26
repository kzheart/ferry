import { expect, test } from "vitest";
import {
  SESSION_OPTIMIZATION_PURPOSE,
  SESSION_OPTIMIZER_ROLE_ID,
  buildSessionOptimizationDraft,
  normalizeSessionPurpose,
} from "./sessionOptimization.js";

test("常量与 runtime 契约一致", () => {
  expect(SESSION_OPTIMIZATION_PURPOSE).toBe("session-optimization");
  expect(SESSION_OPTIMIZER_ROLE_ID).toBe("session-optimizer");
});

test("purpose 归一化:只认 session-optimization,其余一律 general", () => {
  expect(normalizeSessionPurpose("session-optimization"))
    .toBe("session-optimization");
  expect(normalizeSessionPurpose("general")).toBe("general");
  expect(normalizeSessionPurpose(undefined)).toBe("general");
  // newChat 被直接绑成 onClick 时收到的是事件对象
  expect(normalizeSessionPurpose({ type: "click" })).toBe("general");
});

test("整段草稿不含轮次,单轮草稿指向第 N 轮", () => {
  const whole = buildSessionOptimizationDraft();
  expect(whole).toContain("通读附件会话");
  expect(whole).not.toContain("第");

  const single = buildSessionOptimizationDraft({ turn: 3 });
  expect(single).toContain("第 3 轮");
  expect(single).toContain("preview");

  // 非法 turn 回落为整段口径
  expect(buildSessionOptimizationDraft({ turn: 0 })).toBe(whole);
  expect(buildSessionOptimizationDraft({ turn: "x" })).toBe(whole);
});
