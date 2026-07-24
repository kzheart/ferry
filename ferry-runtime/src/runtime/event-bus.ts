import type { RuntimeEventType } from "../server/generated/events.js";
import { PROTOCOL_VERSION, type EventEnvelope } from "../server/messages.js";

export class RuntimeEventBus {
  private readonly listeners = new Set<(event: EventEnvelope) => void>();
  private sequence = 1;

  constructor(private readonly now: () => Date) {}

  subscribe(listener: (event: EventEnvelope) => void) {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  publish(event: EventEnvelope) {
    for (const listener of this.listeners) listener(event);
  }

  emit(
    type: RuntimeEventType,
    payload: Record<string, unknown>,
    sessionId = "runtime",
    runId: string | null = null,
  ) {
    this.publish({
      protocol: PROTOCOL_VERSION,
      session_id: sessionId,
      run_id: runId,
      seq: this.sequence++,
      timestamp: this.now().toISOString(),
      type,
      payload,
    });
  }
}
