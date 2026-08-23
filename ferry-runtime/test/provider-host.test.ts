import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { FileProviderConfigStore } from "../src/providers/provider-config-store.js";
import { FileModelsStore } from "../src/providers/model-store.js";
import { EphemeralSessionStore } from "../src/sessions/session-store.js";
import { ProviderHost } from "../src/providers/provider-host.js";
import { UNSUPPORTED_PROVIDER_IDS } from "../src/providers/provider-models.js";
import { AgentRuntime } from "../src/runtime/runtime.js";
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
    for (const unsupported of UNSUPPORTED_PROVIDER_IDS) {
      expect(ids).not.toContain(unsupported);
    }
    expect(ids).toContain("deepseek");
    expect(ids).toContain("openai-codex");
    expect(ids).toContain("github-copilot");
  });

  it("only offers models from enabled, configured providers the user kept visible", async () => {
    const { host: providers } = await host();
    // 默认点亮但未配置凭据 → 选择器里什么都没有
    expect(await providers.enabledModels()).toEqual([]);

    await providers.saveApiKey("openai", "configured-key");
    const all = await providers.enabledModels();
    expect(all.length).toBeGreaterThan(0);
    expect(all.every((model) => model.provider === "openai")).toBe(true);
    expect(all[0]?.provider_name).toBe("OpenAI");

    const kept = all[0]!.id;
    await providers.setVisibleModels("openai", [kept]);
    expect((await providers.enabledModels()).map((model) => model.id)).toEqual([
      kept,
    ]);
    const summary = (await providers.providers()).find(
      (provider) => provider.id === "openai",
    );
    expect(summary?.visible_model_count).toBe(1);
    expect(summary?.model_count).toBeGreaterThan(1);

    await providers.setProviderEnabled("openai", false);
    expect(await providers.enabledModels()).toEqual([]);
    await expect(
      providers.setVisibleModels("openai", ["not-a-real-model"]),
    ).rejects.toThrow("model is not available");
  });

  it("overlays hand-typed model ids onto a built-in provider", async () => {
    const { host: providers } = await host();
    await providers.saveApiKey("anthropic", "configured-key");
    const template = (await providers.enabledModels())[0]!;

    await providers.saveCustomModel("anthropic", {
      id: "claude-from-the-future",
      name: "Future",
    });
    const added = (await providers.catalogModels()).find(
      (model) => model.id === "claude-from-the-future",
    );
    expect(added?.name).toBe("Future");
    expect(added?.custom).toBe(true);
    // 缺省字段沿用同 Provider 的既有模型,streaming 走同一个 API
    expect(added?.api).toBe(template.api);
    expect(
      providers.model({ provider: "anthropic", model: added!.id }),
    ).toBeTruthy();

    // 白名单存在时新模型要自动可见,否则加完就看不见
    await providers.setVisibleModels("anthropic", [template.id]);
    await providers.saveCustomModel("anthropic", { id: "another-one" });
    expect(
      (await providers.enabledModels()).map((model) => model.id),
    ).toContain("another-one");

    await providers.deleteCustomModel("anthropic", "claude-from-the-future");
    expect(
      (await providers.catalogModels()).some((model) => model.custom),
    ).toBe(true);
    await providers.deleteCustomModel("anthropic", "another-one");
    expect(
      (await providers.catalogModels()).some((model) => model.custom),
    ).toBe(false);
  });

  it("reveals stored API keys, falling back to custom provider config", async () => {
    const { host: providers } = await host();
    await providers.saveApiKey("openai", "sk-openai-secret");
    expect(await providers.revealApiKey("openai")).toEqual({
      provider_id: "openai",
      key: "sk-openai-secret",
    });
    // 未配置凭据的内置 Provider 返回 null
    expect((await providers.revealApiKey("anthropic")).key).toBeNull();

    await providers.saveCustomProvider({
      id: "reveal-test",
      name: "Reveal Test",
      base_url: "http://127.0.0.1:11434/v1",
      api_key: "sk-from-config",
      models: [],
    });
    // 自定义提供商:凭据库为空时回落到配置里的 api_key
    expect((await providers.revealApiKey("reveal-test")).key).toBe(
      "sk-from-config",
    );
    await providers.saveApiKey("reveal-test", "sk-from-store");
    expect((await providers.revealApiKey("reveal-test")).key).toBe(
      "sk-from-store",
    );
    await expect(providers.revealApiKey("no-such")).rejects.toThrow(
      "provider not found",
    );
  });

  it("never reads ambient provider environment variables", async () => {
    process.env.OPENAI_API_KEY = "ambient-key-must-not-be-used";
    const { host: providers } = await host();
    expect(await providers.isConfigured("openai")).toBe(false);
    await providers.saveApiKey("openai", "configured-key");
    expect(await providers.isConfigured("openai")).toBe(true);
  });

  it("reports a failing dynamic catalog as a failed id instead of throwing", async () => {
    const { host: providers } = await host();
    await expect(providers.refreshModels()).resolves.toEqual({
      aborted: false,
      failed_provider_ids: [],
    });

    // 127.0.0.1:9(discard)连不上,拉模型列表必然失败。
    await providers.saveCustomProvider({
      id: "unreachable-gateway",
      name: "Unreachable Gateway",
      base_url: "http://127.0.0.1:9",
      api_key: "sk-test",
      models: [
        {
          id: "some-model",
          input: ["text"],
          reasoning: false,
          context_window: 8_192,
          max_tokens: 1_024,
        },
      ],
    });
    await expect(providers.refreshModels()).resolves.toEqual({
      aborted: false,
      failed_provider_ids: ["unreachable-gateway"],
    });
  });

  it("restores a configured Radius model catalog before sessions load", async () => {
    const directory = await mkdtemp(join(tmpdir(), "ferry-radius-restore-"));
    const config = new FileProviderConfigStore(
      join(directory, "providers.json"),
    );
    await config.modify("radius", async () => ({
      type: "oauth",
      access: "test-access",
      refresh: "test-refresh",
      expires: Date.now() + 60_000,
    }));
    await new FileModelsStore(join(directory, "model-catalogs")).write(
      "radius",
      {
        checkedAt: Date.now(),
        models: [
          {
            id: "restored-radius-model",
            name: "Restored Radius Model",
            api: "openai-completions",
            provider: "radius",
            baseUrl: "https://api.tryradius.ai/v1",
            reasoning: false,
            input: ["text"],
            cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
            contextWindow: 128_000,
            maxTokens: 8_192,
          },
        ],
      },
    );
    const providers = await ProviderHost.create(config);
    expect(providers.listModels("radius")).toMatchObject([
      { id: "restored-radius-model", provider: "radius" },
    ]);
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
      {
        id: "qwen/qwen3:30b",
        provider: "local-ollama",
        api: "openai-completions",
      },
    ]);
  });

  it("adds an Anthropic-compatible custom provider", async () => {
    const { host: providers } = await host();
    await providers.saveCustomProvider({
      id: "my-gateway",
      name: "My Gateway",
      base_url: "https://gateway.example.com/anthropic",
      api: "anthropic-messages",
      api_key: "sk-test",
      models: [
        {
          id: "some-model",
          input: ["text"],
          reasoning: true,
          context_window: 200_000,
          max_tokens: 64_000,
        },
      ],
    });
    expect(await providers.isConfigured("my-gateway")).toBe(true);
    expect(providers.listModels("my-gateway")).toMatchObject([
      { id: "some-model", provider: "my-gateway", api: "anthropic-messages" },
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
    const sessions = new EphemeralSessionStore();
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
      runtime.providerService.selectModel("conversation", {
        provider: "switch-test",
        model: "text-only",
      }),
    ).rejects.toMatchObject({ code: "model_capability_mismatch" });
    await runtime.providerService.selectModel("conversation", {
      provider: "switch-test",
      model: "vision-b",
      thinking: "high",
    });
    expect(runtime.state("conversation")).toMatchObject({
      thinking_level: "high",
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
      thinking_level: "high",
    });
    await expect(
      restored.providerService.selectModel("conversation", {
        provider: "switch-test",
        model: "text-only",
      }),
    ).rejects.toMatchObject({ code: "model_capability_mismatch" });
  });
});
