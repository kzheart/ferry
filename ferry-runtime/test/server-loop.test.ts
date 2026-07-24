import { PassThrough } from "node:stream";
import { describe, expect, it } from "vitest";
import { createProtocolTestBackend } from "./test-backend.js";
import { AgentRuntime } from "../src/runtime/runtime.js";
import { EngineSessionStore } from "../src/sessions/engine-store.js";
import { PROTOCOL_VERSION } from "../src/server/messages.js";
import { serveRuntime } from "../src/server/server-loop.js";

async function waitFor<T>(
  values: T[],
  predicate: (value: T) => boolean,
): Promise<T> {
  const deadline = Date.now() + 1_000;
  while (Date.now() < deadline) {
    const value = values.find(predicate);
    if (value) return value;
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
  throw new Error("timed out waiting for runtime response");
}

describe("runtime server startup", () => {
  it("accepts the restore gateway result while health waits for readiness", async () => {
    let nextId = 0;
    const runtime = await AgentRuntime.create({
      backendFactory: createProtocolTestBackend,
      deferRestore: true,
      idFactory: () => `id-${++nextId}`,
      storeFactory: (invoke) => new EngineSessionStore(invoke),
    });
    const input = new PassThrough();
    const output: Array<Record<string, unknown>> = [];
    const serving = serveRuntime(runtime, input, (value) => {
      const message = value as Record<string, unknown>;
      output.push(message);
      if (message.type !== "engine.request") return;
      const payload = message.payload as Record<string, unknown>;
      input.write(
        `${JSON.stringify({
          protocol: PROTOCOL_VERSION,
          id: "restore-result",
          method: "tool.result",
          params: {
            request_id: payload.request_id,
            session_id: message.session_id,
            ok: true,
            result: [],
          },
        })}\n`,
      );
    });

    input.write(
      `${JSON.stringify({
        protocol: PROTOCOL_VERSION,
        id: "health",
        method: "health",
        params: {},
      })}\n`,
    );

    const health = await waitFor(output, (value) => value.id === "health");
    expect(health).toMatchObject({
      ok: true,
      result: { service: "ferry-runtime" },
    });
    expect(output.some((value) => value.type === "engine.request")).toBe(true);

    input.end();
    await serving;
  });
});
