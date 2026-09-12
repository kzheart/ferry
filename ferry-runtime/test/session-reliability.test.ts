import { describe, expect, it, vi } from "vitest";
import { AgentRuntime } from "../src/runtime/runtime.js";
import type { ProviderHost } from "../src/providers/provider-host.js";
import type { ModelSelection } from "../src/providers/provider-config.js";
import {
  EphemeralSessionStore,
  type SessionCommit,
} from "../src/sessions/session-store.js";
import { createProtocolTestBackend } from "./test-backend.js";

function gate() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}
class DelayedStartStore extends EphemeralSessionStore {
  readonly entered = gate();
  readonly release = gate();
  override async commit(update: SessionCommit) {
    if (update.events.some((event) => event.type === "run.started")) {
      this.entered.resolve();
      await this.release.promise;
    }
    await super.commit(update);
  }
}
const createRuntime = (store = new EphemeralSessionStore()) =>
  AgentRuntime.create({ store, backendFactory: createProtocolTestBackend });

describe("session reliability", () => {
  it("rejects concurrent prompts and retains the accepted run identity", async () => {
    const store = new DelayedStartStore();
    const runtime = await createRuntime(store);
    await runtime.createSession("s1");
    const accepted = runtime.prompt("s1", "hello");
    await store.entered.promise;
    try {
      await expect(runtime.prompt("s1", "other")).rejects.toMatchObject({
        code: "run_in_progress",
      });
    } finally {
      store.release.resolve();
    }
    const { run_id } = await accepted;
    await runtime.waitForIdle("s1");
    const events = await runtime.replay("s1", 0);
    const output = events.filter((event) =>
      ["content.delta", "run.completed"].includes(event.type),
    );
    expect(output.some((event) => event.type === "content.delta")).toBe(true);
    expect(output.some((event) => event.type === "run.completed")).toBe(true);
    expect(output.every((event) => event.run_id === run_id)).toBe(true);
    expect(events.filter((event) => event.type === "run.started")).toHaveLength(
      1,
    );
  });
  it("retains long original messages across restart and subsequent persistence", async () => {
    const store = new EphemeralSessionStore();
    const runtime = await createRuntime(store);
    await runtime.createSession("s1");
    const original = "完整的历史文本".repeat(3000);
    await runtime.prompt("s1", original);
    await runtime.waitForIdle("s1");
    const before = (await store.load("s1"))!.state.messages;
    expect(before[0]).toMatchObject({
      content: [{ type: "text", text: original }],
    });
    const restored = await createRuntime(store);
    await restored.replay("s1", 0);
    await restored.renameSession("s1", "restored");
    expect((await store.load("s1"))!.state.messages).toEqual(before);
  });
  it("replays a removed model without resolving it and allows switching models", async () => {
    const store = new EphemeralSessionStore();
    const original = await createRuntime(store);
    await original.createSession("s1");
    await original.prompt("s1", "hello");
    await original.waitForIdle("s1");
    store.records.get("s1")!.state.model_id = "removed";
    const backendFactory = vi.fn((selection?: ModelSelection) => {
      if (selection?.model === "removed") throw new Error("model removed");
      return createProtocolTestBackend();
    });
    const host = {
      defaultModel: async () => undefined,
      backend: backendFactory,
      isConfigured: async () => true,
    } as unknown as ProviderHost;
    const restored = await AgentRuntime.create({
      store,
      backendFactory,
      providerHost: host,
    });
    expect(restored.listSessions()).toHaveLength(1);
    expect(await restored.replay("s1", 0)).not.toHaveLength(0);
    expect(
      backendFactory.mock.calls.some(
        ([selection]) => selection?.model === "removed",
      ),
    ).toBe(false);
    await restored.providerService.selectModel("s1", {
      provider: "protocol-test",
      model: "protocol-test-driver",
    });
    await restored.prompt("s1", "after switch");
    await restored.waitForIdle("s1");
    expect(restored.state("s1").model_id).toBe("protocol-test-driver");
  });
  it("isolates failed history loads from other sessions", async () => {
    class CorruptStore extends EphemeralSessionStore {
      override async load(id: string) {
        if (id === "bad") throw new Error("corrupt history");
        return super.load(id);
      }
    }
    const store = new CorruptStore();
    const original = await createRuntime(store);
    await original.createSession("bad");
    await original.createSession("good");
    const restored = await createRuntime(store);
    expect(restored.listSessions()).toHaveLength(2);
    await expect(restored.replay("bad", 0)).rejects.toThrow("corrupt history");
    await restored.prompt("good", "hello");
    await restored.waitForIdle("good");
    expect(
      (await restored.replay("good", 0)).some(
        (event) => event.type === "run.completed",
      ),
    ).toBe(true);
  });
  it("prevents prompt and replay from reviving a lazily loaded session during deletion", async () => {
    class DelayedDeleteStore extends EphemeralSessionStore {
      readonly entered = gate();
      readonly release = gate();
      override async delete(id: string) {
        this.entered.resolve();
        await this.release.promise;
        await super.delete(id);
      }
    }
    const store = new DelayedDeleteStore();
    const original = await createRuntime(store);
    await original.createSession("lazy");
    const restored = await createRuntime(store);
    const load = vi.spyOn(store, "load");
    const deletion = restored.deleteSession("lazy");
    await store.entered.promise;
    try {
      await expect(
        restored.prompt("lazy", "must not run"),
      ).rejects.toMatchObject({ code: "session_not_found" });
      await expect(restored.replay("lazy", 0)).rejects.toMatchObject({
        code: "session_not_found",
      });
      expect(load).not.toHaveBeenCalled();
    } finally {
      store.release.resolve();
    }
    await expect(deletion).resolves.toEqual({
      session_id: "lazy",
      deleted: true,
    });
    expect(restored.listSessions()).toEqual([]);
    expect(await store.load("lazy")).toBeNull();
    await expect(
      restored.prompt("lazy", "still deleted"),
    ).rejects.toMatchObject({ code: "session_not_found" });
  });

  it("honors cancellation while startup awaits persistence", async () => {
    const store = new DelayedStartStore();
    const runtime = await createRuntime(store);
    await runtime.createSession("s1");
    const request = runtime.prompt("s1", "hello");
    await store.entered.promise;
    runtime.abort("s1");
    store.release.resolve();
    const { run_id } = await request;
    await runtime.waitForIdle("s1");
    const events = await runtime.replay("s1", 0);
    expect(
      events.filter((event) => event.type === "run.cancelled"),
    ).toMatchObject([{ run_id }]);
    expect(
      events.some(
        (event) =>
          event.type === "content.delta" || event.type === "run.completed",
      ),
    ).toBe(false);
    expect(runtime.state("s1").status).toBe("idle");
  });
});
