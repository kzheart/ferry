import { describe, expect, it } from "vitest";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { FileProviderConfigStore } from "../src/providers/provider-config.js";
import { ProviderHost } from "../src/providers/provider-host.js";
import { AgentRuntime } from "../src/runtime/runtime.js";

const DEEPSEEK_API_KEY_ENV = "DEEPSEEK_API_KEY";
const DEEPSEEK_PROVIDER_ID = "deepseek";
const DEEPSEEK_MODEL_ID = "deepseek-v4-flash";

describe("DeepSeek provider integration", () => {
  it("streams a real deepseek-v4-flash response through Pi", async () => {
    expect(process.env[DEEPSEEK_API_KEY_ENV]?.trim()).toBeTruthy();
    const directory = await mkdtemp(join(tmpdir(), "ferry-deepseek-real-"));
    const config = new FileProviderConfigStore(
      join(directory, "providers.json"),
    );
    await config.modify(DEEPSEEK_PROVIDER_ID, async () => ({
      type: "api_key",
      key: process.env[DEEPSEEK_API_KEY_ENV]!.trim(),
    }));
    const providerHost = await ProviderHost.create(config);
    const calls: string[] = [];
    try {
      const runtime = await AgentRuntime.create({
        providerHost,
        toolHandler: async (name) => {
          calls.push(name);
          return { sessions: [] };
        },
      });
      await expect(runtime.providerStatus()).resolves.toMatchObject({
        provider: DEEPSEEK_PROVIDER_ID,
        model: DEEPSEEK_MODEL_ID,
        credential: "available",
        provider_count: 36,
      });

      await runtime.createSession("deepseek-smoke");
      const { run_id } = await runtime.prompt(
        "deepseek-smoke",
        '必须先调用 session_search（query 传 "test"）搜索会话，然后仅回复 FERRY_DEEPSEEK_OK。',
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
      expect(calls).toEqual(["session_search"]);
      expect(events.some((event) => event.type === "tool.completed")).toBe(
        true,
      );
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  }, 60_000);
});
