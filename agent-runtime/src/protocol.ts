export const PROTOCOL_VERSION = "ferry-agent/v1" as const;

export type CommandMethod =
  | "health"
  | "session.create"
  | "prompt"
  | "abort"
  | "steer"
  | "follow_up"
  | "state"
  | "events.replay"
  | "tool.result";

export interface CommandEnvelope {
  protocol: typeof PROTOCOL_VERSION;
  id: string;
  method: CommandMethod;
  params?: Record<string, unknown>;
}

export interface ResponseEnvelope {
  protocol: typeof PROTOCOL_VERSION;
  id: string;
  ok: boolean;
  result?: unknown;
  error?: { code: string; message: string };
}

export interface EventEnvelope {
  protocol: typeof PROTOCOL_VERSION;
  session_id: string;
  run_id: string | null;
  seq: number;
  timestamp: string;
  type: string;
  payload: Record<string, unknown>;
}

export class ProtocolError extends Error {
  constructor(
    readonly code: string,
    message: string,
  ) {
    super(message);
  }
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function parseCommand(input: unknown): CommandEnvelope {
  if (!isObject(input))
    throw new ProtocolError("invalid_request", "command must be an object");
  if (input.protocol !== PROTOCOL_VERSION) {
    throw new ProtocolError(
      "unsupported_protocol",
      `expected ${PROTOCOL_VERSION}`,
    );
  }
  if (
    typeof input.id !== "string" ||
    input.id.length === 0 ||
    input.id.length > 128
  ) {
    throw new ProtocolError("invalid_request", "id must be a non-empty string");
  }
  const methods: readonly string[] = [
    "health",
    "session.create",
    "prompt",
    "abort",
    "steer",
    "follow_up",
    "state",
    "events.replay",
    "tool.result",
  ];
  if (typeof input.method !== "string" || !methods.includes(input.method)) {
    throw new ProtocolError("unknown_method", "unsupported command method");
  }
  if (input.params !== undefined && !isObject(input.params)) {
    throw new ProtocolError("invalid_request", "params must be an object");
  }
  return input as unknown as CommandEnvelope;
}

export function requireString(
  params: Record<string, unknown>,
  key: string,
  max = 200_000,
): string {
  const value = params[key];
  if (typeof value !== "string" || value.length === 0 || value.length > max) {
    throw new ProtocolError(
      "invalid_params",
      `${key} must be a non-empty string`,
    );
  }
  return value;
}

export function requireInteger(
  params: Record<string, unknown>,
  key: string,
): number {
  const value = params[key];
  if (!Number.isSafeInteger(value) || (value as number) < 0) {
    throw new ProtocolError(
      "invalid_params",
      `${key} must be a non-negative integer`,
    );
  }
  return value as number;
}
