import { describe, expect, it } from "vitest";
import {
  MemorySessionStore,
  type PersistedSession,
} from "../src/event-store.js";
import { AgentRuntime } from "../src/runtime.js";
import { PROTOCOL_VERSION, type EventEnvelope } from "../src/protocol.js";
import { createProtocolTestBackend } from "./test-backend.js";

async function createRuntime(
  options: Parameters<typeof AgentRuntime.create>[0] = {},
) {
  let nextId = 0;
  return AgentRuntime.create({
    backendFactory: createProtocolTestBackend,
    ...options,
    idFactory: () => `id-${++nextId}`,
  });
}

describe("AgentRuntime", () => {
  it("streams ordered deltas and replays only events after the cursor", async () => {
    const runtime = await createRuntime();
    await runtime.createSession("s1");
    const { run_id } = await runtime.prompt("s1", "hello");
    await runtime.waitForIdle("s1");

    const events = runtime.replay("s1", 0);
    expect(events.some((event) => event.type === "content.delta")).toBe(true);
    expect(events.at(-1)?.type).toBe("run.completed");
    expect(events.every((event) => event.protocol === PROTOCOL_VERSION)).toBe(
      true,
    );
    expect(
      events.filter((event) => event.run_id === run_id).length,
    ).toBeGreaterThan(1);
    expect(events.map((event) => event.seq)).toEqual(
      events.map((_, index) => index + 1),
    );
    expect(runtime.replay("s1", events[1]!.seq)).toEqual(events.slice(2));
  });

  it("executes only an explicitly registered Ferry tool", async () => {
    const calls: string[] = [];
    const runtime = await createRuntime({
      toolHandler: async (name, args, context) => {
        calls.push(name);
        context.onUpdate({ phase: "gateway" });
        expect(args).toEqual({});
        return { agents: ["codex"] };
      },
    });
    await runtime.createSession("s1");
    await runtime.prompt("s1", "tool:list_capabilities");
    await runtime.waitForIdle("s1");

    const types = runtime.replay("s1", 0).map((event) => event.type);
    expect(calls).toEqual(["ferry_list_capabilities"]);
    expect(types).toContain("tool.started");
    expect(types).toContain("tool.progress");
    expect(types).toContain("tool.completed");
    expect(types.at(-1)).toBe("run.completed");
  });

  it("accepts an immediate gateway result after publishing tool.request", async () => {
    const runtime = await createRuntime();
    runtime.subscribe((event) => {
      if (event.type === "tool.request") {
        runtime.completeTool(
          event.payload.request_id as string,
          event.session_id,
          true,
          { agents: ["codex"] },
        );
      }
    });
    await runtime.createSession("s1");
    await runtime.prompt("s1", "tool:list_capabilities");
    await runtime.waitForIdle("s1");

    expect(runtime.replay("s1", 0).at(-1)?.type).toBe("run.completed");
  });

  it("opens the next prompt gate before publishing the terminal event", async () => {
    const runtime = await createRuntime();
    let secondRun: Promise<unknown> | undefined;
    runtime.subscribe((event) => {
      if (event.type === "run.completed" && !secondRun) {
        secondRun = runtime.prompt("s1", "second");
      }
    });
    await runtime.createSession("s1");
    await runtime.prompt("s1", "first");
    await runtime.waitForIdle("s1");
    await secondRun;
    await runtime.waitForIdle("s1");

    expect(
      runtime.replay("s1", 0).filter((event) => event.type === "run.completed"),
    ).toHaveLength(2);
  });

  it("removes a pending gateway request when the run is aborted", async () => {
    const runtime = await createRuntime();
    let requestId = "";
    runtime.subscribe((event) => {
      if (event.type === "tool.request") {
        requestId = event.payload.request_id as string;
        runtime.abort(event.session_id);
      }
    });
    await runtime.createSession("s1");
    await runtime.prompt("s1", "tool:list_capabilities");
    await runtime.waitForIdle("s1");

    expect(requestId).not.toBe("");
    expect(() => runtime.completeTool(requestId, "s1", true, {})).toThrow(
      "tool request not found",
    );
  });

  it("cancels an active model stream", async () => {
    const runtime = await createRuntime();
    await runtime.createSession("s1");
    await runtime.prompt("s1", `slow:${"x".repeat(100)}`);
    await new Promise((resolve) => setTimeout(resolve, 20));
    runtime.abort("s1");
    await runtime.waitForIdle("s1");

    expect(runtime.replay("s1", 0).at(-1)?.type).toBe("run.cancelled");
  });

  it.each([
    ["steer", (runtime: AgentRuntime) => runtime.steer("s1", "steered")],
    [
      "follow-up",
      (runtime: AgentRuntime) => runtime.followUp("s1", "followed"),
    ],
  ])("queues %s messages through Pi", async (_label, enqueue) => {
    const runtime = await createRuntime();
    await runtime.createSession("s1");
    await runtime.prompt("s1", "slow:x");
    await new Promise((resolve) => setTimeout(resolve, 5));
    enqueue(runtime);
    await runtime.waitForIdle("s1");

    const text = runtime
      .replay("s1", 0)
      .filter((event) => event.type === "content.delta")
      .map((event) => event.payload.delta)
      .join("");
    expect(text).toContain(_label === "steer" ? "steered" : "followed");
  });

  it("marks a persisted in-flight run interrupted without replaying it", async () => {
    const store = new MemorySessionStore();
    const state: PersistedSession = {
      session_id: "s1",
      provider_id: "test",
      model_id: "test-model",
      contains_images: false,
      next_seq: 2,
      status: "running",
      active_run_id: "old-run",
      messages: [],
    };
    const events: EventEnvelope[] = [
      {
        protocol: PROTOCOL_VERSION,
        session_id: "s1",
        run_id: "old-run",
        seq: 1,
        timestamp: "2026-01-01T00:00:00.000Z",
        type: "run.started",
        payload: {},
      },
    ];
    await store.save(state, events);

    const runtime = await createRuntime({ store });
    expect(runtime.replay("s1", 0).map((event) => event.type)).toEqual([
      "run.started",
      "run.interrupted",
    ]);
    expect(runtime.state("s1").status).toBe("idle");
  });
});
