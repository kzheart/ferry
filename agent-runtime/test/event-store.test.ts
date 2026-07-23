import { appendFile, mkdtemp, readFile, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  FileSessionStore,
  type PersistedSession,
  type SessionStore,
} from "../src/event-store.js";
import type { EventEnvelope } from "../src/protocol.js";
import { safeEvents, safeMessages } from "../src/runtime.js";

const directories: string[] = [];

afterEach(async () => {
  await Promise.all(
    directories
      .splice(0)
      .map((directory) => rm(directory, { recursive: true, force: true })),
  );
});

async function commitSnapshot(
  store: SessionStore,
  state: PersistedSession,
  events: EventEnvelope[],
) {
  const { messages, ...metadata } = state;
  const lastMessage = messages.at(-1);
  const committableMessageCount =
    state.status === "running" &&
    events.at(-1)?.type === "content.delta" &&
    lastMessage?.role === "assistant"
      ? messages.length - 1
      : messages.length;
  let boundary = events.length - 1;
  if (state.status === "running") {
    while (boundary >= 0 && events[boundary]!.type === "content.delta") {
      boundary -= 1;
    }
  }
  await store.commit({
    metadata,
    messages: messages
      .slice(0, committableMessageCount)
      .map((message, ordinal) => ({
        ordinal,
        message,
      })),
    events: events.slice(0, boundary + 1),
    timestamp: events.at(-1)?.timestamp ?? "2026-01-01T00:00:00.000Z",
  });
}

describe("FileSessionStore", () => {
  it("serializes concurrent commits for the same session", async () => {
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
        commitSnapshot(store, { ...base, title: `title-${index + 1}` }, []),
      ),
    );

    const [record] = await store.loadAll();
    expect(record?.state.title).toBe("title-20");
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
        payload: { name: "session_edit", args: { text: "private" } },
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

  it("keeps assistant text on the correct side of tool events", () => {
    const events = safeEvents([
      {
        protocol: "ferry-agent/v1",
        session_id: "s1",
        run_id: "r1",
        seq: 1,
        timestamp: "2026-01-01T00:00:00.000Z",
        type: "content.delta",
        payload: { delta: "先搜索。" },
      },
      {
        protocol: "ferry-agent/v1",
        session_id: "s1",
        run_id: "r1",
        seq: 2,
        timestamp: "2026-01-01T00:00:01.000Z",
        type: "tool.started",
        payload: { name: "ferry_search_sessions", args: {} },
      },
      {
        protocol: "ferry-agent/v1",
        session_id: "s1",
        run_id: "r1",
        seq: 3,
        timestamp: "2026-01-01T00:00:02.000Z",
        type: "content.delta",
        payload: { delta: "搜索失败。" },
      },
    ]);
    expect(events.map((event) => event.type)).toEqual([
      "content.delta",
      "tool.started",
      "content.delta",
    ]);
    expect(events.map((event) => event.payload.delta)).toEqual([
      "先搜索。",
      undefined,
      "搜索失败。",
    ]);
  });

  it("serializes deletion after pending writes", async () => {
    const directory = await mkdtemp(join(tmpdir(), "ferry-agent-store-"));
    directories.push(directory);
    const store = new FileSessionStore(directory);
    const state: PersistedSession = {
      session_id: "s1",
      provider_id: "test",
      model_id: "test-model",
      contains_images: false,
      next_seq: 1,
      status: "idle",
      active_run_id: null,
      messages: [],
      title: "测试",
      pinned: false,
    };
    await Promise.all([commitSnapshot(store, state, []), store.delete("s1")]);
    expect(await readdir(directory)).not.toContain("s1.jsonl");
  });

  it("appends ordered JSONL records and buffers an unfinished assistant block", async () => {
    const directory = await mkdtemp(join(tmpdir(), "ferry-agent-store-"));
    directories.push(directory);
    const store = new FileSessionStore(directory);
    const state: PersistedSession = {
      session_id: "s1",
      provider_id: "test",
      model_id: "test-model",
      contains_images: false,
      next_seq: 3,
      status: "running",
      active_run_id: "r1",
      messages: [
        { role: "user", content: "搜索", timestamp: 1 },
        {
          role: "assistant",
          content: [{ type: "text", text: "正在搜索" }],
          api: "test",
          provider: "test",
          model: "test-model",
          usage: {
            input: 0,
            output: 0,
            cacheRead: 0,
            cacheWrite: 0,
            totalTokens: 0,
            cost: {
              input: 0,
              output: 0,
              cacheRead: 0,
              cacheWrite: 0,
              total: 0,
            },
          },
          stopReason: "stop",
          timestamp: 2,
        },
      ],
    };
    const started: EventEnvelope = {
      protocol: "ferry-agent/v1",
      session_id: "s1",
      run_id: "r1",
      seq: 1,
      timestamp: "2026-01-01T00:00:00.000Z",
      type: "run.started",
      payload: { prompt: "搜索" },
    };
    const delta: EventEnvelope = {
      protocol: "ferry-agent/v1",
      session_id: "s1",
      run_id: "r1",
      seq: 2,
      timestamp: "2026-01-01T00:00:01.000Z",
      type: "content.delta",
      payload: { delta: "正在搜索" },
    };
    await commitSnapshot(store, state, [started, delta]);
    const beforeBoundary = await readFile(join(directory, "s1.jsonl"), "utf8");
    expect(beforeBoundary).toContain('"type":"run.started"');
    expect(beforeBoundary).not.toContain('"type":"content.delta"');

    const tool: EventEnvelope = {
      protocol: "ferry-agent/v1",
      session_id: "s1",
      run_id: "r1",
      seq: 3,
      timestamp: "2026-01-01T00:00:02.000Z",
      type: "tool.started",
      payload: { tool_call_id: "tool-1", name: "ferry_search_sessions" },
    };
    await commitSnapshot(store, { ...state, next_seq: 4 }, [
      started,
      delta,
      tool,
    ]);
    const [restored] = await store.loadAll();
    expect(restored?.events.map((event) => event.type)).toEqual([
      "run.started",
      "content.delta",
      "tool.started",
    ]);
    expect(restored?.state.messages).toHaveLength(2);
    expect(
      JSON.parse(
        await readFile(join(directory, "sessions-index.json"), "utf8"),
      ),
    ).toMatchObject({
      version: 1,
      entries: [{ session_id: "s1", latest_seq: 3 }],
    });
  });

  it("drops an incomplete crash tail before later appends", async () => {
    const directory = await mkdtemp(join(tmpdir(), "ferry-agent-store-"));
    directories.push(directory);
    const state: PersistedSession = {
      session_id: "s1",
      provider_id: "test",
      model_id: "test-model",
      contains_images: false,
      next_seq: 1,
      status: "idle",
      active_run_id: null,
      messages: [],
      title: "before",
    };
    const first = new FileSessionStore(directory);
    await first.loadAll();
    await commitSnapshot(first, state, []);
    await appendFile(
      join(directory, "s1.jsonl"),
      '{"version":1,"type":"event"',
    );

    const restored = new FileSessionStore(directory);
    expect((await restored.loadAll())[0]?.state.title).toBe("before");
    await commitSnapshot(restored, { ...state, title: "after" }, []);
    expect(
      (await new FileSessionStore(directory).loadAll())[0]?.state.title,
    ).toBe("after");
  });
});
