import { describe, expect, it } from "vitest";
import { RuntimeEventBus } from "../src/runtime/event-bus.js";
import { RuntimeGateway } from "../src/tools/gateway.js";
import type { EventEnvelope } from "../src/server/messages.js";

function setup(
  options: { publish?: () => Promise<void>; deadline?: number } = {},
) {
  const events = new RuntimeEventBus(() => new Date());
  const observed: EventEnvelope[] = [];
  events.subscribe((event) => observed.push(event));
  const controller = new AbortController();
  const gateway = new RuntimeGateway({
    newId: () => "request",
    events,
    emitToolRequest: options.publish ?? (async () => {}),
    toolDeadlinesMs: { bash: options.deadline ?? 1_000 },
  });
  const invoke = () =>
    gateway.invokeTool(
      "bash",
      { command: "echo test" },
      {
        sessionId: "session",
        runId: "run",
        toolCallId: "call",
        signal: controller.signal,
        onUpdate() {},
      },
    );
  return { controller, gateway, observed, invoke };
}

describe("native tool cancellation", () => {
  it("sends cancellation with the full request identity", async () => {
    const state = setup();
    const result = state.invoke();
    const rejected = expect(result).rejects.toThrow("tool request aborted");
    await Promise.resolve();
    state.controller.abort();
    await rejected;
    expect(state.observed).toMatchObject([
      {
        type: "tool.cancel",
        session_id: "session",
        run_id: "run",
        payload: { request_id: "request", reason: "aborted" },
      },
    ]);
  });

  it("rejects immediately but publishes cancellation after a delayed request", async () => {
    let publish!: () => void;
    const state = setup({
      publish: () =>
        new Promise<void>((resolve) => {
          publish = resolve;
        }),
    });
    const result = state.invoke();
    const rejected = expect(result).rejects.toThrow("tool request aborted");
    state.controller.abort();
    await rejected;
    expect(state.observed).toHaveLength(0);
    publish();
    await Promise.resolve();
    expect(state.observed[0]?.type).toBe("tool.cancel");
  });

  it("cancels native execution when its gateway deadline expires", async () => {
    const state = setup({ deadline: 5 });
    await expect(state.invoke()).rejects.toThrow("tool gateway timed out");
    expect(state.observed[0]?.payload).toEqual({
      request_id: "request",
      reason: "timeout",
    });
  });

  it("does not dispatch already aborted requests or cancel completed requests", async () => {
    let published = 0;
    const early = setup({
      publish: async () => {
        published++;
      },
    });
    early.controller.abort();
    await expect(early.invoke()).rejects.toThrow("tool request aborted");
    expect(published).toBe(0);
    expect(early.observed).toHaveLength(0);
    const completed = setup();
    const result = completed.invoke();
    completed.gateway.complete("request", "session", true, {});
    await result;
    completed.controller.abort();
    expect(completed.observed).toHaveLength(0);
  });
});
