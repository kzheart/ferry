import {
  createAssistantMessageEventStream,
  type AssistantMessage,
  type Model,
  type StreamFunction,
} from "@earendil-works/pi-ai";
import { describe, expect, it } from "vitest";

import { AgentRuntime } from "../src/runtime/runtime.js";
import type { AgentBackend } from "../src/providers/provider-service.js";
import {
  EphemeralSessionStore,
  type SessionCommit,
} from "../src/sessions/session-store.js";

const usage = {
  input: 0,
  output: 0,
  cacheRead: 0,
  cacheWrite: 0,
  totalTokens: 0,
  cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
};

const model: Model<string> = {
  id: "streaming-test-driver",
  name: "Streaming test driver",
  api: "protocol-test",
  provider: "protocol-test",
  baseUrl: "http://127.0.0.1",
  reasoning: false,
  input: ["text"],
  cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
  contextWindow: 16_384,
  maxTokens: 4_096,
};

function assistant(
  content: AssistantMessage["content"],
  stopReason: AssistantMessage["stopReason"],
): AssistantMessage {
  return {
    role: "assistant",
    content,
    api: model.api,
    provider: model.provider,
    model: model.id,
    usage,
    stopReason,
    timestamp: Date.now(),
  };
}

/** 每次回答切成 `chunks` 个 text_delta,用来观察提交次数是否随 token 数增长。 */
function chunkedBackend(chunks: number) {
  const streamFn: StreamFunction = () => {
    const stream = createAssistantMessageEventStream();
    const partial = assistant([], "stop");
    stream.push({ type: "start", partial });
    stream.push({ type: "text_start", contentIndex: 0, partial });
    let text = "";
    for (let index = 0; index < chunks; index += 1) {
      text += `chunk-${index} `;
      stream.push({
        type: "text_delta",
        contentIndex: 0,
        delta: `chunk-${index} `,
        partial: assistant([{ type: "text", text }], "stop"),
      });
    }
    const complete = assistant([{ type: "text", text }], "stop");
    stream.push({
      type: "text_end",
      contentIndex: 0,
      content: text,
      partial: complete,
    });
    stream.push({ type: "done", reason: "stop", message: complete });
    return stream;
  };
  return (): AgentBackend => ({
    model,
    streamFn,
    provider: model.provider,
    modelId: model.id,
    credentialAvailable: () => true,
  });
}

class RecordingSessionStore extends EphemeralSessionStore {
  readonly commits: SessionCommit[] = [];

  override async commit(update: SessionCommit) {
    this.commits.push(structuredClone(update));
    await super.commit(update);
  }
}

async function runOnce(chunks: number) {
  const store = new RecordingSessionStore();
  let nextId = 0;
  const runtime = await AgentRuntime.create({
    backendFactory: chunkedBackend(chunks),
    store,
    idFactory: () => `id-${++nextId}`,
  });
  await runtime.createSession("s1");
  await runtime.prompt("s1", "hello");
  await runtime.waitForIdle("s1");
  return { store, runtime };
}

describe("streaming persistence", () => {
  it("keeps commit count independent of delta count", async () => {
    const few = await runOnce(3);
    const many = await runOnce(200);

    const deltasOf = (store: RecordingSessionStore) =>
      store.commits.flatMap((commit) =>
        commit.events.filter((event) => event.type === "content.delta"),
      ).length;
    expect(deltasOf(few.store)).toBe(3);
    expect(deltasOf(many.store)).toBe(200);
    expect(many.store.commits.length).toBe(few.store.commits.length);
  });

  it("never writes an empty commit", async () => {
    const { store } = await runOnce(50);
    for (const commit of store.commits) {
      expect(commit.messages.length + commit.events.length).toBeGreaterThan(0);
    }
  });

  it("persists the user message while the answer is still streaming", async () => {
    const { store } = await runOnce(50);
    const userCommit = store.commits.findIndex((commit) =>
      commit.messages.some((entry) => entry.message.role === "user"),
    );
    const assistantCommit = store.commits.findIndex((commit) =>
      commit.messages.some((entry) => entry.message.role === "assistant"),
    );
    // 用户消息必须早于助手回答落盘:流式中途崩溃时它仍要能被恢复出来。
    expect(userCommit).toBeGreaterThanOrEqual(0);
    expect(userCommit).toBeLessThan(assistantCommit);
    const [restored] = await store.loadAll();
    expect(
      restored?.state.messages.some((entry) => entry.role === "user"),
    ).toBe(true);
  });
});
