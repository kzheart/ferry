import { describe, expect, it } from "vitest";
import { EphemeralSessionStore } from "../src/sessions/session-store.js";

describe("session summaries", () => {
  it("retains creation time while updating summary time without loading history", async () => {
    const store = new EphemeralSessionStore();
    const metadata = {
      session_id: "s1",
      provider_id: "test",
      model_id: "test",
      contains_images: false,
      next_seq: 0,
      status: "idle" as const,
      active_run_id: null,
      context_checkpoint: { messageCount: 0, messages: [] },
    };
    await store.commit({ metadata, messages: [], events: [], timestamp: "t1" });
    await store.commit({ metadata, messages: [], events: [], timestamp: "t2" });
    const [summary] = await store.list();
    expect(summary).toMatchObject({ created_at: "t1", updated_at: "t2" });
    expect(summary).not.toHaveProperty("messages");
    expect(summary).not.toHaveProperty("context_checkpoint");
    expect((await store.load("s1"))?.state.context_checkpoint).toEqual(
      metadata.context_checkpoint,
    );
    expect(await store.load("missing")).toBeNull();
  });
});
