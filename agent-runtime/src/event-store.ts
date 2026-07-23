import { randomUUID } from "node:crypto";
import {
  appendFile,
  chmod,
  mkdir,
  readFile,
  readdir,
  rename,
  truncate,
  unlink,
  writeFile,
} from "node:fs/promises";
import { join } from "node:path";
import type { AgentMessage } from "@earendil-works/pi-agent-core";
import type { EventEnvelope } from "./protocol.js";

const STORE_VERSION = 1 as const;

export interface PersistedSession {
  session_id: string;
  provider_id: string;
  model_id: string;
  contains_images: boolean;
  next_seq: number;
  status: "idle" | "running";
  active_run_id: string | null;
  messages: AgentMessage[];
  title?: string | null;
  pinned?: boolean;
  thinking_level?: string;
  role_id?: string;
  resolved_persona?: string;
  resolved_tools?: string[];
  resolved_apply_policy?: "manual" | "auto";
}

export type PersistedSessionMetadata = Omit<PersistedSession, "messages">;

type SessionRecordMetadata = Omit<PersistedSession, "messages" | "next_seq">;

type SessionRecord =
  | {
      version: typeof STORE_VERSION;
      type: "session.meta";
      session_id: string;
      timestamp: string;
      state: SessionRecordMetadata;
    }
  | {
      version: typeof STORE_VERSION;
      type: "message";
      session_id: string;
      ordinal: number;
      message: AgentMessage;
    }
  | {
      version: typeof STORE_VERSION;
      type: "event";
      session_id: string;
      event: EventEnvelope;
    };

interface SessionCursor {
  eventSeq: number;
  messageCount: number;
  metadata: string;
}

interface SessionIndexEntry {
  session_id: string;
  title: string | null;
  pinned: boolean;
  provider_id: string;
  model_id: string;
  status: "idle" | "running";
  created_at: string | null;
  updated_at: string | null;
  message_count: number;
  latest_seq: number;
}

export interface SessionStore {
  loadAll(): Promise<
    Array<{ state: PersistedSession; events: EventEnvelope[] }>
  >;
  commit(update: SessionCommit): Promise<void>;
  delete(sessionId: string): Promise<void>;
}

export interface SessionCommit {
  metadata: PersistedSessionMetadata;
  messages: Array<{ ordinal: number; message: AgentMessage }>;
  events: EventEnvelope[];
  timestamp: string;
}

export class MemorySessionStore implements SessionStore {
  readonly records = new Map<
    string,
    { state: PersistedSession; events: EventEnvelope[] }
  >();

  async loadAll() {
    return [...this.records.values()].map((record) => structuredClone(record));
  }

  async commit(update: SessionCommit) {
    const snapshot = structuredClone(update);
    const existing = this.records.get(snapshot.metadata.session_id);
    const messages = new Map<number, AgentMessage>(
      existing?.state.messages.map((message, ordinal) => [ordinal, message]),
    );
    const events = new Map<number, EventEnvelope>(
      existing?.events.map((event) => [event.seq, event]),
    );
    for (const record of snapshot.messages) {
      messages.set(record.ordinal, record.message);
    }
    for (const event of snapshot.events) events.set(event.seq, event);
    this.records.set(snapshot.metadata.session_id, {
      state: {
        ...snapshot.metadata,
        messages: [...messages.entries()]
          .sort(([left], [right]) => left - right)
          .map(([, message]) => message),
      },
      events: [...events.values()].sort((left, right) => left.seq - right.seq),
    });
  }

  async delete(sessionId: string) {
    this.records.delete(sessionId);
  }
}

export class FileSessionStore implements SessionStore {
  private readonly writes = new Map<string, Promise<void>>();
  private readonly cursors = new Map<string, SessionCursor>();
  private readonly index = new Map<string, SessionIndexEntry>();
  private indexWrite: Promise<void> = Promise.resolve();

  constructor(private readonly directory: string) {}

  async loadAll() {
    await mkdir(this.directory, { recursive: true, mode: 0o700 });
    const names = (await readdir(this.directory)).filter((name) =>
      name.endsWith(".jsonl"),
    );
    const records: Array<{ state: PersistedSession; events: EventEnvelope[] }> =
      [];
    this.cursors.clear();
    this.index.clear();
    for (const name of names) {
      const sessionRecords = await this.readRecords(join(this.directory, name));
      const restored = this.restore(sessionRecords);
      if (!restored) continue;
      records.push(restored);
      const metadata = metadataOf(restored.state);
      this.cursors.set(restored.state.session_id, {
        eventSeq: restored.events.at(-1)?.seq ?? 0,
        messageCount: restored.state.messages.length,
        metadata: JSON.stringify(metadata),
      });
      this.index.set(
        restored.state.session_id,
        indexEntry(
          restored.state,
          undefined,
          restored.events,
          this.cursors.get(restored.state.session_id)!,
          restored.events.at(-1)?.timestamp ?? new Date().toISOString(),
        ),
      );
    }
    await this.writeIndex();
    return records;
  }

  async commit(update: SessionCommit) {
    const snapshot = structuredClone(update);
    const id = snapshot.metadata.session_id;
    sessionFilename(id);
    await this.enqueue(id, async () => {
      await mkdir(this.directory, { recursive: true, mode: 0o700 });
      const cursor = this.cursors.get(id) ?? {
        eventSeq: 0,
        messageCount: 0,
        metadata: "",
      };
      const metadata = metadataOf(snapshot.metadata);
      const metadataJson = JSON.stringify(metadata);
      const records: SessionRecord[] = [];
      if (metadataJson !== cursor.metadata) {
        records.push({
          version: STORE_VERSION,
          type: "session.meta",
          session_id: id,
          timestamp: snapshot.timestamp,
          state: metadata,
        });
      }

      const messages = snapshot.messages.filter(
        (record) => record.ordinal >= cursor.messageCount,
      );
      for (const record of messages) {
        records.push({
          version: STORE_VERSION,
          type: "message",
          session_id: id,
          ordinal: record.ordinal,
          message: record.message,
        });
      }

      const events = snapshot.events.filter(
        (event) => event.seq > cursor.eventSeq,
      );
      for (const event of events) {
        records.push({
          version: STORE_VERSION,
          type: "event",
          session_id: id,
          event,
        });
      }
      if (records.length === 0) return;

      const target = join(this.directory, sessionFilename(id));
      await appendFile(
        target,
        `${records.map((record) => JSON.stringify(record)).join("\n")}\n`,
        { encoding: "utf8", mode: 0o600 },
      );
      await chmod(target, 0o600);
      const nextCursor = {
        eventSeq: events.at(-1)?.seq ?? cursor.eventSeq,
        messageCount: Math.max(
          cursor.messageCount,
          ...messages.map((record) => record.ordinal + 1),
        ),
        metadata: metadataJson,
      };
      this.cursors.set(id, nextCursor);
      this.index.set(
        id,
        indexEntry(
          snapshot.metadata,
          this.index.get(id),
          events,
          nextCursor,
          snapshot.timestamp,
        ),
      );
      await this.writeIndex();
    });
  }

  async delete(sessionId: string) {
    sessionFilename(sessionId);
    await this.enqueue(sessionId, async () => {
      try {
        await unlink(join(this.directory, sessionFilename(sessionId)));
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
      }
      this.cursors.delete(sessionId);
      this.index.delete(sessionId);
      await this.writeIndex();
    });
  }

  private async readRecords(path: string): Promise<SessionRecord[]> {
    const content = await readFile(path, "utf8");
    const lines = content.split("\n");
    const records: SessionRecord[] = [];
    for (let index = 0; index < lines.length; index += 1) {
      const line = lines[index]!.trim();
      if (!line) continue;
      try {
        const record = JSON.parse(line) as SessionRecord;
        if (record.version !== STORE_VERSION) {
          throw new Error(`unsupported Ferry session version in ${path}`);
        }
        records.push(record);
      } catch (error) {
        const isTail = lines
          .slice(index + 1)
          .every((candidate) => !candidate.trim());
        if (isTail) {
          const validPrefix = lines.slice(0, index).join("\n");
          await truncate(
            path,
            Buffer.byteLength(validPrefix ? `${validPrefix}\n` : ""),
          );
          break;
        }
        throw error;
      }
    }
    return records;
  }

  private restore(records: SessionRecord[]) {
    let metadata: SessionRecordMetadata | undefined;
    const messages = new Map<number, AgentMessage>();
    const events: EventEnvelope[] = [];
    for (const record of records) {
      if (record.type === "session.meta") metadata = record.state;
      else if (record.type === "message")
        messages.set(record.ordinal, record.message);
      else if (record.type === "event") events.push(record.event);
    }
    if (!metadata) return null;
    events.sort((left, right) => left.seq - right.seq);
    const orderedMessages = [...messages.entries()]
      .sort(([left], [right]) => left - right)
      .map(([, message]) => message);
    return {
      state: {
        ...metadata,
        next_seq: (events.at(-1)?.seq ?? 0) + 1,
        messages: orderedMessages,
      },
      events,
    };
  }

  private async writeIndex() {
    const write = async () => {
      const target = join(this.directory, "sessions-index.json");
      const temporary = `${target}.${process.pid}.${randomUUID()}.tmp`;
      const payload = JSON.stringify({
        version: STORE_VERSION,
        entries: [...this.index.values()].sort((left, right) =>
          String(right.updated_at ?? "").localeCompare(
            String(left.updated_at ?? ""),
          ),
        ),
      });
      try {
        await writeFile(temporary, payload, { encoding: "utf8", mode: 0o600 });
        await rename(temporary, target);
      } catch (error) {
        await unlink(temporary).catch(() => undefined);
        throw error;
      }
    };
    const next = this.indexWrite.catch(() => undefined).then(write);
    this.indexWrite = next;
    await next;
  }

  private async enqueue(id: string, write: () => Promise<void>) {
    const previous = this.writes.get(id) ?? Promise.resolve();
    const next = previous.catch(() => undefined).then(write);
    this.writes.set(id, next);
    try {
      await next;
    } finally {
      if (this.writes.get(id) === next) this.writes.delete(id);
    }
  }
}

function metadataOf(state: PersistedSessionMetadata): SessionRecordMetadata {
  const { next_seq: _nextSeq, ...metadata } = state;
  return metadata;
}

function sessionFilename(sessionId: string) {
  if (!/^[A-Za-z0-9_-]{1,128}$/.test(sessionId)) {
    throw new Error("invalid Ferry session id");
  }
  return `${sessionId}.jsonl`;
}

function indexEntry(
  state: PersistedSessionMetadata,
  previous: SessionIndexEntry | undefined,
  events: EventEnvelope[],
  cursor: SessionCursor,
  timestamp: string,
): SessionIndexEntry {
  const latest = events.at(-1);
  return {
    session_id: state.session_id,
    title: state.title ?? null,
    pinned: state.pinned ?? false,
    provider_id: state.provider_id,
    model_id: state.model_id,
    status: state.status,
    created_at: previous?.created_at ?? latest?.timestamp ?? timestamp,
    updated_at: latest?.timestamp ?? timestamp,
    message_count: cursor.messageCount,
    latest_seq: cursor.eventSeq,
  };
}
