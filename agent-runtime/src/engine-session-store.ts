import type {
  PersistedSession,
  SessionCommit,
  SessionStore,
} from "./event-store.js";
import type { EventEnvelope } from "./protocol.js";

export type RuntimeEngineMethod =
  | "runtime_sessions.load_all"
  | "runtime_sessions.commit"
  | "runtime_sessions.delete";

export type RuntimeEngineInvoke = (
  method: RuntimeEngineMethod,
  params: Record<string, unknown>,
  sessionId: string,
) => Promise<unknown>;

export class EngineSessionStore implements SessionStore {
  constructor(private readonly invoke: RuntimeEngineInvoke) {}

  async loadAll() {
    const result = await this.invoke(
      "runtime_sessions.load_all",
      {},
      "runtime",
    );
    if (!Array.isArray(result))
      throw new Error("runtime session store returned invalid data");
    return result as Array<{
      state: PersistedSession;
      events: EventEnvelope[];
    }>;
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
}
