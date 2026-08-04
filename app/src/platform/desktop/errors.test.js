import { expect, test } from "vitest";
import { initI18n } from "../../shared/i18n/index.js";
import { EngineError } from "./errors.js";

const i18n = initI18n();
await i18n.changeLanguage("zh-CN");

test("有 i18n 映射的错误码翻译成人话", () => {
  const error = new EngineError({ code: "agent.reference_invalid" });
  expect(error.code).toBe("agent.reference_invalid");
  expect(error.message).not.toContain("agent.reference_invalid");
  expect(error.message).toContain("会话引用已失效");
});

test("无映射的错误码走 fallback 文案而不是裸 key", () => {
  const error = new EngineError({ code: "made.up_code" });
  // returnNull: false 时 t() 缺 key 返回 key 本身,曾导致裸错误码直出 UI
  expect(error.message).not.toBe("made.up_code");
  expect(error.message).toContain("引擎错误");
  expect(error.message).toContain("made.up_code");
});
