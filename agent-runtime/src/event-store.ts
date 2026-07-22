import { randomUUID } from "node:crypto";
import {
  mkdir,
  readFile,
  readdir,
  rename,
  unlink,
  writeFile,
} from "node:fs/promises";
import { join } from "node:path";
import type { AgentMessage } from "@earendil-works/pi-agent-core";
import type { EventEnvelope } from "./protocol.js";

export interface PersistedSession {
  session_id: string;
  provider_id: string;
  model_id: string;
  contains_images: boolean;
  next_seq: number;
  status: "idle" | "running";
  active_run_id: string | null;
  messages: AgentMessage[];
}

export interface SessionStore {
  loadAll(): Promise<
    Array<{ state: PersistedSession; events: EventEnvelope[] }>
  >;
  save(state: PersistedSession, events: EventEnvelope[]): Promise<void>;
}

export class MemorySessionStore implements SessionStore {
  readonly records = new Map<
    string,
    { state: PersistedSession; events: EventEnvelope[] }
  >();

  async loadAll() {
    return [...this.records.values()].map((record) => structuredClone(record));
  }

  async save(state: PersistedSession, events: EventEnvelope[]) {
    this.records.set(state.session_id, structuredClone({ state, events }));
  }
}

export class FileSessionStore implements SessionStore {
  private readonly writes = new Map<string, Promise<void>>();

  constructor(private readonly directory: string) {}

  async loadAll() {
    await mkdir(this.directory, { recursive: true, mode: 0o700 });
    const names = (await readdir(this.directory)).filter((name) =>
      name.endsWith(".json"),
    );
    const records: Array<{ state: PersistedSession; events: EventEnvelope[] }> =
      [];
    for (const name of names) {
      const parsed = JSON.parse(
        await readFile(join(this.directory, name), "utf8"),
      ) as {
        state: PersistedSession;
        events: EventEnvelope[];
      };
      records.push(parsed);
    }
    return records;
  }

  async save(state: PersistedSession, events: EventEnvelope[]) {
    const id = state.session_id;
    const payload = JSON.stringify(structuredClone({ state, events }));
    const previous = this.writes.get(id) ?? Promise.resolve();
    const next = previous
      .catch(() => undefined)
      .then(async () => {
        await mkdir(this.directory, { recursive: true, mode: 0o700 });
        const target = join(this.directory, `${id}.json`);
        const temporary = `${target}.${process.pid}.${randomUUID()}.tmp`;
        try {
          await writeFile(temporary, payload, {
            encoding: "utf8",
            mode: 0o600,
          });
          await rename(temporary, target);
        } catch (error) {
          await unlink(temporary).catch(() => undefined);
          throw error;
        }
      });
    this.writes.set(id, next);
    try {
      await next;
    } finally {
      if (this.writes.get(id) === next) this.writes.delete(id);
    }
  }
}
