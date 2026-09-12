import { Agent } from "@earendil-works/pi-agent-core";
import {
  createAssistantMessageEventStream,
  type AssistantMessage,
} from "@earendil-works/pi-ai";
import { describe, expect, it, vi } from "vitest";
import { createFerryTools, type FerryToolName } from "../src/tools/catalog.js";
import { createProtocolTestBackend } from "./test-backend.js";

async function startBatch(names: FerryToolName[]) {
  const backend = createProtocolTestBackend();
  const started: string[] = [];
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  const tools = createFerryTools(
    {
      invoke: async (name) => {
        started.push(name);
        await gate;
        return { ok: true };
      },
    },
    () => ({ sessionId: "s", runId: "r", applyPolicy: "manual" }),
    names,
  );
  const agent = new Agent({
    initialState: { model: backend.model, tools },
    toolExecution: "parallel",
    streamFn: (model, context, options) => {
      if (context.messages.at(-1)?.role === "toolResult")
        return backend.streamFn(model, context, options);
      const stream = createAssistantMessageEventStream();
      const message: AssistantMessage = {
        role: "assistant",
        api: model.api,
        provider: model.provider,
        model: model.id,
        content: names.map((name, index) => ({
          type: "toolCall",
          id: String(index),
          name,
          arguments:
            name === "session_search"
              ? { query: "ferry" }
              : name === "bash"
                ? { command: "pwd" }
                : {},
        })),
        usage: {
          input: 0,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          totalTokens: 0,
          cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
        },
        stopReason: "toolUse",
        timestamp: Date.now(),
      };
      stream.push({ type: "done", reason: "toolUse", message });
      return stream;
    },
  });
  const run = agent.prompt("execute batch");
  return { started, release, run };
}

describe("Ferry tool scheduling through pi", () => {
  it("starts independent read tools before either has completed", async () => {
    const batch = await startBatch(["session_search", "usage"]);
    try {
      await vi.waitFor(() =>
        expect(batch.started).toEqual(["session_search", "usage"]),
      );
    } finally {
      batch.release();
      await batch.run;
    }
  });

  it("serializes a batch containing a side-effecting tool", async () => {
    const batch = await startBatch(["session_search", "bash"]);
    try {
      await vi.waitFor(() => expect(batch.started).toEqual(["session_search"]));
    } finally {
      batch.release();
      await batch.run;
    }
    expect(batch.started).toEqual(["session_search", "bash"]);
  });
});
