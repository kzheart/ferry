import { readFile, stat } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { mkdtemp } from "node:fs/promises";
import { describe, expect, it } from "vitest";
import { FileProviderConfigStore } from "../src/provider-config.js";

async function store() {
  const directory = await mkdtemp(join(tmpdir(), "ferry-provider-config-"));
  return new FileProviderConfigStore(join(directory, "providers.json"));
}

describe("FileProviderConfigStore", () => {
  it("stores plaintext credentials atomically without exposing them in public snapshots", async () => {
    const config = await store();
    await config.modify("deepseek", async () => ({
      type: "api_key",
      key: "sk-plain-text-value",
      env: { REGION: "cn" },
    }));

    const source = await readFile(config.path, "utf8");
    expect(source).toContain("sk-plain-text-value");
    expect(source).toContain('"fields"');
    expect(source).not.toContain('"env"');
    expect((await stat(config.path)).mode & 0o777).toBe(0o600);
    expect(JSON.stringify(await config.publicSnapshot())).not.toContain(
      "sk-plain-text-value",
    );
    expect(await config.list()).toEqual([
      { providerId: "deepseek", type: "api_key" },
    ]);
  });

  it("serializes credential refresh updates", async () => {
    const config = await store();
    await Promise.all([
      config.modify("anthropic", async () => ({
        type: "oauth",
        access: "access-1",
        refresh: "refresh-1",
        expires: 1,
      })),
      config.modify("openai", async () => ({
        type: "api_key",
        key: "openai-key",
      })),
    ]);
    expect((await config.snapshot()).credentials).toMatchObject({
      anthropic: { type: "oauth", access: "access-1" },
      openai: { type: "api_key", key: "openai-key" },
    });
  });

  it("persists model selection and redacts custom provider keys", async () => {
    const config = await store();
    await config.setDefaultModel({ provider: "openai", model: "gpt-5-mini" });
    await config.saveCustomProvider({
      id: "local-ollama",
      name: "Local Ollama",
      base_url: "http://127.0.0.1:11434/v1/",
      api_key: "ollama-plain-key",
      models: [
        {
          id: "qwen3.5",
          input: ["text"],
          reasoning: false,
          context_window: 128_000,
          max_tokens: 8_192,
        },
      ],
    });

    expect(await config.publicSnapshot()).toMatchObject({
      default_model: { provider: "openai", model: "gpt-5-mini" },
      custom_providers: [
        {
          id: "local-ollama",
          base_url: "http://127.0.0.1:11434/v1",
          configured: true,
        },
      ],
    });
    expect(JSON.stringify(await config.publicSnapshot())).not.toContain(
      "ollama-plain-key",
    );

    await config.saveCustomProvider({
      id: "local-ollama",
      name: "Renamed Ollama",
      base_url: "http://127.0.0.1:11434/v1",
      models: [
        {
          id: "qwen3.5",
          input: ["text"],
          reasoning: false,
          context_window: 128_000,
          max_tokens: 8_192,
        },
      ],
    });
    expect((await config.snapshot()).custom_providers[0]?.api_key).toBe(
      "ollama-plain-key",
    );
    await config.saveCustomProvider(
      {
        id: "local-ollama",
        name: "Renamed Ollama",
        base_url: "http://127.0.0.1:11434/v1",
        models: [
          {
            id: "qwen3.5",
            input: ["text"],
            reasoning: false,
            context_window: 128_000,
            max_tokens: 8_192,
          },
        ],
      },
      true,
    );
    expect(
      (await config.snapshot()).custom_providers[0]?.api_key,
    ).toBeUndefined();
  });

  it("merges writes made by separate store instances", async () => {
    const first = await store();
    const second = new FileProviderConfigStore(first.path);
    await Promise.all([
      first.modify("deepseek", async () => ({
        type: "api_key",
        key: "deepseek-key",
      })),
      second.modify("openai", async () => ({
        type: "api_key",
        key: "openai-key",
      })),
    ]);
    expect((await first.snapshot()).credentials).toMatchObject({
      deepseek: { key: "deepseek-key" },
      openai: { key: "openai-key" },
    });
  });

  it("rolls back only the credential written by a cancelled login", async () => {
    const config = await store();
    const previous = { type: "api_key" as const, key: "previous-key" };
    const oauth = {
      type: "oauth" as const,
      access: "new-access",
      refresh: "new-refresh",
      expires: 123,
    };
    await config.modify("radius", async () => previous);
    await config.modify("radius", async () => oauth);
    await config.restoreCredential("radius", oauth, previous);
    expect(await config.read("radius")).toEqual(previous);

    await config.modify("radius", async () => oauth);
    await config.modify("radius", async () => ({
      ...oauth,
      access: "newer-access",
    }));
    await config.restoreCredential("radius", oauth, previous);
    expect(await config.read("radius")).toMatchObject({
      access: "newer-access",
    });
  });

  it("rejects unsupported URLs and malformed credentials", async () => {
    const config = await store();
    await expect(
      config.saveCustomProvider({
        id: "bad",
        name: "Bad",
        base_url: "file:///tmp/provider",
        models: [
          {
            id: "model",
            input: ["text"],
            reasoning: false,
            context_window: 128_000,
            max_tokens: 8_192,
          },
        ],
      }),
    ).rejects.toThrow("HTTP or HTTPS");
    await expect(
      config.modify("bad", async () => ({ type: "api_key" })),
    ).rejects.toThrow("empty");
  });
});
