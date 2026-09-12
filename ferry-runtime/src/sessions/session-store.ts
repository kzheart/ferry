/** Ferry 对话会话的持久化端口与进程内实现。 */
import type { AgentMessage } from "@earendil-works/pi-agent-core";
import type { EventEnvelope } from "../server/messages.js";

export interface PersistedSession {
  session_id: string;
  provider_id: string;
  model_id: string;
  contains_images: boolean;
  next_seq: number;
  status: "idle" | "running";
  active_run_id: string | null;
  messages: AgentMessage[];
  context_checkpoint?: import("./context.js").ContextCheckpoint;
  created_at?: string;
  updated_at?: string;
  title?: string | null;
  /** 用户手动改过名:自动命名从此不再覆盖。 */
  title_locked?: boolean;
  pinned?: boolean;
  thinking_level?: string;
  role_id?: string;
  resolved_persona?: string;
  resolved_tools?: string[];
  resolved_apply_policy?: "manual" | "auto";
  /** 只记 id;重启时重新 resolveFor,磁盘上的技能可能已经被删了。 */
  resolved_skills?: string[];
}

export type PersistedSessionMetadata = Omit<PersistedSession, "messages">;

export type SessionSummary = Omit<
  PersistedSessionMetadata,
  "context_checkpoint"
>;

export interface SessionCommit {
  metadata: PersistedSessionMetadata;
  messages: Array<{ ordinal: number; message: AgentMessage }>;
  events: EventEnvelope[];
  timestamp: string;
}

export interface SessionStore {
  list(): Promise<SessionSummary[]>;
  load(
    sessionId: string,
  ): Promise<{ state: PersistedSession; events: EventEnvelope[] } | null>;
  commit(update: SessionCommit): Promise<void>;
  delete(sessionId: string): Promise<void>;
  /** 编辑重发:删掉 ordinal >= fromOrdinal 的消息与 seq >= fromSeq 的事件。 */
  truncate(
    sessionId: string,
    fromOrdinal: number,
    fromSeq: number,
  ): Promise<void>;
}

/** 仅供测试与显式注入使用；不构成跨会话或长期记忆。 */
export class EphemeralSessionStore implements SessionStore {
  readonly records = new Map<
    string,
    { state: PersistedSession; events: EventEnvelope[] }
  >();

  async list() {
    return [...this.records.values()].map(({ state }) => {
      const {
        messages: _messages,
        context_checkpoint: _checkpoint,
        ...metadata
      } = state;
      return structuredClone(metadata);
    });
  }

  async load(sessionId: string) {
    const record = this.records.get(sessionId);
    return record ? structuredClone(record) : null;
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
    for (const record of snapshot.messages)
      messages.set(record.ordinal, record.message);
    for (const event of snapshot.events) events.set(event.seq, event);
    this.records.set(snapshot.metadata.session_id, {
      state: {
        ...snapshot.metadata,
        created_at: existing?.state.created_at ?? snapshot.timestamp,
        updated_at: snapshot.timestamp,
        messages: [...messages.entries()]
          .sort(([a], [b]) => a - b)
          .map(([, value]) => value),
      },
      events: [...events.values()].sort((a, b) => a.seq - b.seq),
    });
  }

  async delete(sessionId: string) {
    this.records.delete(sessionId);
  }

  async truncate(sessionId: string, fromOrdinal: number, fromSeq: number) {
    const record = this.records.get(sessionId);
    if (!record) return;
    record.state.messages = record.state.messages.slice(0, fromOrdinal);
    record.events = record.events.filter((event) => event.seq < fromSeq);
  }
}
