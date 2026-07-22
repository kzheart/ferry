import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { FileProviderConfigStore } from "../src/provider-config.js";
import {
  ProviderHost,
  UNSUPPORTED_PROVIDER_IDS,
} from "../src/provider-host.js";

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

  it("adds a keyless OpenAI-compatible provider", async () => {
    const { host: providers } = await host();
    await providers.saveCustomProvider({
      id: "local-ollama",
      name: "Local Ollama",
      base_url: "http://127.0.0.1:11434/v1",
      models: ["qwen/qwen3:30b"],
    });
    expect(await providers.isConfigured("local-ollama")).toBe(true);
    expect(providers.listModels("local-ollama")).toMatchObject([
      { id: "qwen/qwen3:30b", provider: "local-ollama" },
    ]);
  });
});
