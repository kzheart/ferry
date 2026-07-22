import { describe, expect, it } from "vitest";
import {
  DEEPSEEK_API_KEY_ENV,
  DEEPSEEK_MODEL_ID,
  DEEPSEEK_PROVIDER_ID,
} from "../src/deepseek-provider.js";
import { AgentRuntime } from "../src/runtime.js";

describe("DeepSeek provider integration", () => {
  it("streams a real deepseek-v4-flash response through Pi", async () => {
    expect(process.env[DEEPSEEK_API_KEY_ENV]?.trim()).toBeTruthy();
    const calls: string[] = [];
    const runtime = await AgentRuntime.create({
      toolHandler: async (name) => {
        calls.push(name);
        return { capabilities: ["session_search"] };
      },
    });
    expect(runtime.providerStatus()).toMatchObject({
      provider: DEEPSEEK_PROVIDER_ID,
      model: DEEPSEEK_MODEL_ID,
      credential: "available",
    });

    await runtime.createSession("deepseek-smoke");
    const { run_id } = await runtime.prompt(
      "deepseek-smoke",
      "必须先调用 ferry_list_capabilities 获取能力，然后仅回复 FERRY_DEEPSEEK_OK。",
    );
    await runtime.waitForIdle("deepseek-smoke");

    const events = runtime
      .replay("deepseek-smoke", 0)
      .filter((event) => event.run_id === run_id);
    const terminal = events.at(-1);
    expect(terminal, JSON.stringify(terminal?.payload)).toMatchObject({
      type: "run.completed",
    });
    expect(
      events.filter((event) => event.type === "content.delta").length,
    ).toBeGreaterThan(0);
    expect(calls).toEqual(["ferry_list_capabilities"]);
    expect(events.some((event) => event.type === "tool.completed")).toBe(true);
  }, 60_000);
});
