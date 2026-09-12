import type {
  PersistedSession,
  SessionSummary,
  SessionCommit,
  SessionStore,
} from "./session-store.js";
import type { EventEnvelope } from "../server/messages.js";

export type RuntimeEngineMethod =
  | "runtime_sessions.list"
  | "runtime_sessions.load"
  | "runtime_sessions.commit"
  | "runtime_sessions.delete"
  | "runtime_sessions.truncate";

export type RuntimeEngineInvoke = (
  method: RuntimeEngineMethod,
  params: Record<string, unknown>,
  sessionId: string,
) => Promise<unknown>;

export class EngineSessionStore implements SessionStore {
  constructor(private readonly invoke: RuntimeEngineInvoke) {}

  async list(): Promise<SessionSummary[]> {
    const result = await this.invoke("runtime_sessions.list", {}, "runtime");
    if (!Array.isArray(result))
      throw new Error("runtime session store returned invalid data");
    return result as SessionSummary[];
  }

  async load(sessionId: string) {
    const result = await this.invoke(
      "runtime_sessions.load",
      { session_id: sessionId },
      sessionId,
    );
    return result as {
      state: PersistedSession;
      events: EventEnvelope[];
    } | null;
  }

  async commit(update: SessionCommit) {
    await this.invoke(
      "runtime_sessions.commit",
      { update },
      update.metadata.session_id,
    );
  }

  async delete(sessionId: string) {
    await this.invoke(
      "runtime_sessions.delete",
      { session_id: sessionId },
      sessionId,
    );
  }

  async truncate(sessionId: string, fromOrdinal: number, fromSeq: number) {
    await this.invoke(
      "runtime_sessions.truncate",
      {
        session_id: sessionId,
        from_ordinal: fromOrdinal,
        from_seq: fromSeq,
      },
      sessionId,
    );
  }
}
