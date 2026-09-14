import { describe, expect, it } from "vitest";
import { normalizeBaseUrl } from "./CustomProviderSettings.jsx";

describe("normalizeBaseUrl", () => {
  it("补全缺失的 https 协议头并去掉尾部斜杠", () => {
    expect(normalizeBaseUrl("cpa.example.com/v1")).toBe("https://cpa.example.com/v1");
    expect(normalizeBaseUrl("http://127.0.0.1:8318/v1/")).toBe("http://127.0.0.1:8318/v1");
  });
  it("空值与带空格的输入视为不合法", () => {
    expect(normalizeBaseUrl("  ")).toBeNull();
    expect(normalizeBaseUrl("https://a b/v1")).toBeNull();
  });
});
