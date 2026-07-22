import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { FileProviderConfigStore } from "../src/provider-config.js";
import { MemorySessionStore } from "../src/event-store.js";
import {
  ProviderHost,
  UNSUPPORTED_PROVIDER_IDS,
} from "../src/provider-host.js";
import { AgentRuntime } from "../src/runtime.js";
import { createProtocolTestBackend } from "./test-backend.js";

const previousOpenAIKey = process.env.OPENAI_API_KEY;
afterEach(() => {
  if (previousOpenAIKey === undefined) delete process.env.OPENAI_API_KEY;
  else process.env.OPENAI_API_KEY = previousOpenAIKey;
});

async function host() {
  const directory = await mkdtemp(join(tmpdir(), "ferry-provider-host-"));
  const store = new FileProviderConfigStore(join(directory, "providers.json"));
  return { store, host: await ProviderHost.create(store) };
}

describe("ProviderHost", () => {
  it("registers all Pi providers except Bedrock and Vertex", async () => {
    const { host: providers } = await host();
    const ids = (await providers.providers()).map((provider) => provider.id);
    expect(ids).toHaveLength(36);
    for (const unsupported of UNSUPPORTED_PROVIDER_IDS) {
      expect(ids).not.toContain(unsupported);
    }
    expect(ids).toContain("deepseek");
    expect(ids).toContain("openai-codex");
    expect(ids).toContain("github-copilot");
  });

  it("never reads ambient provider environment variables", async () => {
    process.env.OPENAI_API_KEY = "ambient-key-must-not-be-used";
    const { host: providers } = await host();
    expect(await providers.isConfigured("openai")).toBe(false);
    await providers.saveApiKey("openai", "configured-key");
    expect(await providers.isConfigured("openai")).toBe(true);
  });

  it("reports OAuth capabilities from Pi providers", async () => {
    const { host: providers } = await host();
    const oauth = (await providers.providers())
      .filter((provider) => provider.auth_types.includes("oauth"))
      .map((provider) => provider.id)
      .sort();
    expect(oauth).toEqual([
      "anthropic",
      "github-copilot",
      "openai-codex",
      "radius",
      "xai",
    ]);
  });

  it("refreshes configured dynamic model catalogs without exposing errors", async () => {
    const { host: providers } = await host();
    await expect(providers.refreshModels()).resolves.toEqual({
      aborted: false,
      failed_provider_ids: [],
    });
  });

  it("adds a keyless OpenAI-compatible provider", async () => {
    const { host: providers } = await host();
    await providers.saveCustomProvider({
      id: "local-ollama",
      name: "Local Ollama",
      base_url: "http://127.0.0.1:11434/v1",
      models: [
        {
          id: "qwen/qwen3:30b",
          input: ["text"],
          reasoning: false,
          context_window: 128_000,
          max_tokens: 8_192,
        },
      ],
    });
    expect(await providers.isConfigured("local-ollama")).toBe(true);
    expect(providers.listModels("local-ollama")).toMatchObject([
      { id: "qwen/qwen3:30b", provider: "local-ollama" },
    ]);
  });

  it("switches models per conversation and blocks image-incompatible targets after restart", async () => {
    const { host: providers } = await host();
    await providers.saveCustomProvider({
      id: "switch-test",
      name: "Switch Test",
      base_url: "http://127.0.0.1:11434/v1",
      models: [
        {
          id: "vision-a",
          input: ["text", "image"],
          reasoning: false,
          context_window: 128_000,
          max_tokens: 8_192,
        },
        {
          id: "vision-b",
          input: ["text", "image"],
          reasoning: true,
          context_window: 128_000,
          max_tokens: 8_192,
        },
        {
          id: "text-only",
          input: ["text"],
          reasoning: false,
          context_window: 128_000,
          max_tokens: 8_192,
        },
      ],
    });
    await providers.selectDefault({
      provider: "switch-test",
      model: "vision-a",
    });
    const sessions = new MemorySessionStore();
    const backendFactory = (selection?: {
      provider: string;
      model: string;
    }) => {
      const backend = createProtocolTestBackend();
      if (!selection) return backend;
      const input: Array<"text" | "image"> = selection.model.startsWith(
        "vision-",
      )
        ? ["text", "image"]
        : ["text"];
      return {
        ...backend,
        model: {
          ...backend.model,
          provider: selection.provider,
          id: selection.model,
          input,
        },
        provider: selection.provider,
        modelId: selection.model,
      };
    };
    const runtime = await AgentRuntime.create({
      providerHost: providers,
      backendFactory,
      store: sessions,
    });
    await runtime.createSession("conversation");
    await runtime.prompt("conversation", "look", [
      { type: "image", mimeType: "image/png", data: "iVBORw0KGgo=" },
    ]);
    await runtime.waitForIdle("conversation");

    await expect(
      runtime.selectModel("conversation", {
        provider: "switch-test",
        model: "text-only",
      }),
    ).rejects.toMatchObject({ code: "model_capability_mismatch" });
    await runtime.selectModel("conversation", {
      provider: "switch-test",
      model: "vision-b",
    });

    const restored = await AgentRuntime.create({
      providerHost: providers,
      backendFactory,
      store: sessions,
    });
    expect(restored.state("conversation")).toMatchObject({
      provider_id: "switch-test",
      model_id: "vision-b",
      contains_images: true,
    });
    await expect(
      restored.selectModel("conversation", {
        provider: "switch-test",
        model: "text-only",
      }),
    ).rejects.toMatchObject({ code: "model_capability_mismatch" });
  });
});
