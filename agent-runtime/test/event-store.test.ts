import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { FileSessionStore, type PersistedSession } from "../src/event-store.js";

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
});
