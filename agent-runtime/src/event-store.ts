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
}

type SessionMetadata = Omit<PersistedSession, "messages" | "next_seq">;

type SessionRecord =
  | {
      version: typeof STORE_VERSION;
      type: "session.meta";
      session_id: string;
      timestamp: string;
      state: SessionMetadata;
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
  save(state: PersistedSession, events: EventEnvelope[]): Promise<void>;
  delete(sessionId: string): Promise<void>;
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
        indexEntry(restored.state, restored.events),
      );
    }
    await this.writeIndex();
    return records;
  }

  async save(state: PersistedSession, events: EventEnvelope[]) {
    const snapshot = structuredClone({ state, events });
    const id = snapshot.state.session_id;
    sessionFilename(id);
    await this.enqueue(id, async () => {
      await mkdir(this.directory, { recursive: true, mode: 0o700 });
      const cursor = this.cursors.get(id) ?? {
        eventSeq: 0,
        messageCount: 0,
        metadata: "",
      };
      const metadata = metadataOf(snapshot.state);
      const metadataJson = JSON.stringify(metadata);
      const records: SessionRecord[] = [];
      if (metadataJson !== cursor.metadata) {
        records.push({
          version: STORE_VERSION,
          type: "session.meta",
          session_id: id,
          timestamp:
            snapshot.events.at(-1)?.timestamp ?? new Date().toISOString(),
          state: metadata,
        });
      }

      const finalizedMessages = finalizedMessageCount(
        snapshot.state,
        snapshot.events,
      );
      for (
        let ordinal = cursor.messageCount;
        ordinal < finalizedMessages;
        ordinal += 1
      ) {
        records.push({
          version: STORE_VERSION,
          type: "message",
          session_id: id,
          ordinal,
          message: snapshot.state.messages[ordinal]!,
        });
      }

      const committableSeq = committableEventSeq(
        snapshot.state,
        snapshot.events,
      );
      const newEvents = snapshot.events.filter(
        (event) => event.seq > cursor.eventSeq && event.seq <= committableSeq,
      );
      for (const event of newEvents) {
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
        eventSeq: newEvents.at(-1)?.seq ?? cursor.eventSeq,
        messageCount: Math.max(cursor.messageCount, finalizedMessages),
        metadata: metadataJson,
      };
      this.cursors.set(id, nextCursor);
      this.index.set(
        id,
        indexEntry(
          snapshot.state,
          snapshot.events,
          nextCursor.eventSeq,
          nextCursor.messageCount,
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
    let metadata: SessionMetadata | undefined;
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

function metadataOf(state: PersistedSession): SessionMetadata {
  const { messages: _messages, next_seq: _nextSeq, ...metadata } = state;
  return metadata;
}

function finalizedMessageCount(
  state: PersistedSession,
  events: EventEnvelope[],
) {
  const lastMessage = state.messages.at(-1);
  const lastEvent = events.at(-1);
  return state.status === "running" &&
    lastEvent?.type === "content.delta" &&
    lastMessage?.role === "assistant"
    ? state.messages.length - 1
    : state.messages.length;
}

function committableEventSeq(state: PersistedSession, events: EventEnvelope[]) {
  if (state.status === "idle") return events.at(-1)?.seq ?? 0;
  let index = events.length - 1;
  while (index >= 0 && events[index]!.type === "content.delta") index -= 1;
  return events[index]?.seq ?? 0;
}

function sessionFilename(sessionId: string) {
  if (!/^[A-Za-z0-9_-]{1,128}$/.test(sessionId)) {
    throw new Error("invalid Ferry session id");
  }
  return `${sessionId}.jsonl`;
}

function indexEntry(
  state: PersistedSession,
  events: EventEnvelope[],
  latestSeq = events.at(-1)?.seq ?? 0,
  messageCount = state.messages.length,
): SessionIndexEntry {
  const committed = events.filter((event) => event.seq <= latestSeq);
  return {
    session_id: state.session_id,
    title: state.title ?? null,
    pinned: state.pinned ?? false,
    provider_id: state.provider_id,
    model_id: state.model_id,
    status: state.status,
    created_at: committed[0]?.timestamp ?? null,
    updated_at: committed.at(-1)?.timestamp ?? null,
    message_count: messageCount,
    latest_seq: latestSeq,
  };
}
