import { expect, test } from "vitest";
import {
  SESSION_OPTIMIZATION_PURPOSE,
  SESSION_OPTIMIZER_ROLE_ID,
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
