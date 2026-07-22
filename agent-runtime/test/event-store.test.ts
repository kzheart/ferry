import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { FileSessionStore, type PersistedSession } from "../src/event-store.js";
import { safeEvents, safeMessages } from "../src/runtime.js";

const directories: string[] = [];

afterEach(async () => {
  await Promise.all(
    directories
      .splice(0)
      .map((directory) => rm(directory, { recursive: true, force: true })),
  );
});

describe("FileSessionStore", () => {
  it("serializes concurrent saves for the same session", async () => {
    const directory = await mkdtemp(join(tmpdir(), "ferry-agent-store-"));
    directories.push(directory);
    const store = new FileSessionStore(directory);
    const base: PersistedSession = {
      session_id: "s1",
      provider_id: "test",
      model_id: "test-model",
      contains_images: false,
      next_seq: 1,
      status: "idle",
      active_run_id: null,
      messages: [],
    };

    await Promise.all(
      Array.from({ length: 20 }, (_, index) =>
        store.save({ ...base, next_seq: index + 1 }, []),
      ),
    );

    const [record] = await store.loadAll();
    expect(record?.state.next_seq).toBe(20);
  });

  it("omits tool payloads and credentials from persisted records", () => {
    const [message] = safeMessages([
      {
        role: "user",
        content: "DEEPSEEK_API_KEY=secret-value Bearer abcdefghijklmnop",
        timestamp: 1,
      },
    ]);
    expect(JSON.stringify(message)).not.toContain("secret-value");
    expect(JSON.stringify(message)).not.toContain("abcdefghijklmnop");

    const [event] = safeEvents([
      {
        protocol: "ferry-agent/v1",
        session_id: "s1",
        run_id: "r1",
        seq: 1,
        timestamp: "2026-01-01T00:00:00.000Z",
        type: "tool.request",
        payload: { name: "ferry_propose_edit", args: { text: "private" } },
      },
    ]);
    expect(event?.payload.args).toBe("[omitted]");
    expect(JSON.stringify(event)).not.toContain("private");

    const streamed = safeEvents([
      {
        protocol: "ferry-agent/v1",
        session_id: "s1",
        run_id: "r1",
        seq: 2,
        timestamp: "2026-01-01T00:00:00.000Z",
        type: "content.delta",
        payload: { delta: "Bearer split-" },
      },
      {
        protocol: "ferry-agent/v1",
        session_id: "s1",
        run_id: "r1",
        seq: 3,
        timestamp: "2026-01-01T00:00:00.000Z",
        type: "content.delta",
        payload: { delta: "credential-value" },
      },
    ]);
    expect(JSON.stringify(streamed)).not.toContain("split-credential-value");
    expect(streamed[0]?.payload.delta).toBe("[REDACTED]");

    const [failed] = safeEvents([
      {
        protocol: "ferry-agent/v1",
        session_id: "s1",
        run_id: "r1",
        seq: 4,
        timestamp: "2026-01-01T00:00:00.000Z",
        type: "run.failed",
        payload: { message: "DEEPSEEK_API_KEY=secret-value" },
      },
    ]);
    expect(JSON.stringify(failed)).not.toContain("secret-value");
  });
});
