import { mkdtemp, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { FileModelsStore } from "../src/model-catalog-store.js";

describe("FileModelsStore", () => {
  it("persists Pi dynamic model catalogs atomically", async () => {
    const directory = await mkdtemp(join(tmpdir(), "ferry-model-catalog-"));
    const store = new FileModelsStore(directory);
    await store.write("radius", {
      checkedAt: 1,
      models: [
        {
          id: "dynamic-model",
          name: "Dynamic Model",
          api: "openai-completions",
          provider: "radius",
          baseUrl: "https://api.example.com/v1",
          reasoning: false,
          input: ["text"],
          cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
          contextWindow: 128_000,
          maxTokens: 8_192,
        },
      ],
    });

    expect(await store.read("radius")).toMatchObject({
      checkedAt: 1,
      models: [{ id: "dynamic-model", provider: "radius" }],
    });
    expect((await stat(join(directory, "radius.json"))).mode & 0o777).toBe(
      0o600,
    );
    await store.delete("radius");
    expect(await store.read("radius")).toBeUndefined();
  });
});
