import type { AgentMessage } from "@earendil-works/pi-agent-core";
import type { EventEnvelope } from "../protocol/messages.js";

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

export interface SessionCommit {
  metadata: PersistedSessionMetadata;
  messages: Array<{ ordinal: number; message: AgentMessage }>;
  events: EventEnvelope[];
  timestamp: string;
}

export interface SessionStore {
  loadAll(): Promise<
    Array<{ state: PersistedSession; events: EventEnvelope[] }>
  >;
  commit(update: SessionCommit): Promise<void>;
  delete(sessionId: string): Promise<void>;
}

/** 仅供测试与显式注入使用；不构成跨会话或长期记忆。 */
export class EphemeralSessionStore implements SessionStore {
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
    for (const record of snapshot.messages)
      messages.set(record.ordinal, record.message);
    for (const event of snapshot.events) events.set(event.seq, event);
    this.records.set(snapshot.metadata.session_id, {
      state: {
        ...snapshot.metadata,
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
}
