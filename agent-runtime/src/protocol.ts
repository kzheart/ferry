import {
  FERRY_IPC_PROTOCOL,
  type IpcError,
  type IpcRequest,
  type IpcResponse,
} from "./contracts/ipc.js";

export const PROTOCOL_VERSION = FERRY_IPC_PROTOCOL;

export type CommandMethod =
  | "health"
  | "session.create"
  | "session.rename"
  | "session.pin"
  | "session.delete"
  | "roles.list"
  | "role.create"
  | "role.update"
  | "role.copy"
  | "role.delete"
  | "organization.start"
  | "prompt"
  | "abort"
  | "steer"
  | "follow_up"
  | "state"
  | "sessions.list"
  | "events.replay"
  | "providers.list"
  | "provider.enabled.set"
  | "provider.test"
  | "models.list"
  | "models.enabled"
  | "models.catalog"
  | "custom_model.add"
  | "custom_model.delete"
  | "models.visibility.set"
  | "models.refresh"
  | "config.get"
  | "credential.set"
  | "provider.logout"
  | "model.select"
  | "custom_provider.upsert"
  | "custom_provider.delete"
  | "auth.login.start"
  | "auth.login.respond"
  | "auth.login.cancel"
  | "tool.result";

export type CommandEnvelope = IpcRequest<CommandMethod>;

export type ResponseEnvelope = IpcResponse;

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
    readonly category = "validation",
    readonly retryable = false,
  ) {
    super(message);
  }

  toEnvelope(): IpcError {
    return {
      code: this.code,
      category: this.category,
      retryable: this.retryable,
      params: { message: this.message },
    };
  }
}

export function isObject(value: unknown): value is Record<string, unknown> {
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
    "session.rename",
    "session.pin",
    "session.delete",
    "roles.list",
    "role.create",
    "role.update",
    "role.copy",
    "role.delete",
    "organization.start",
    "prompt",
    "abort",
    "steer",
    "follow_up",
    "state",
    "sessions.list",
    "events.replay",
    "providers.list",
    "provider.enabled.set",
    "provider.test",
    "models.list",
    "models.enabled",
    "models.catalog",
    "custom_model.add",
    "custom_model.delete",
    "models.visibility.set",
    "models.refresh",
    "config.get",
    "credential.set",
    "provider.logout",
    "model.select",
    "custom_provider.upsert",
    "custom_provider.delete",
    "auth.login.start",
    "auth.login.respond",
    "auth.login.cancel",
    "tool.result",
  ];
  if (typeof input.method !== "string" || !methods.includes(input.method)) {
    throw new ProtocolError("unknown_method", "unsupported command method");
  }
  if (!isObject(input.params)) {
    throw new ProtocolError("invalid_request", "params must be an object");
  }
  const fields = Object.keys(input);
  if (
    fields.length !== 4 ||
    !fields.every((field) =>
      ["protocol", "id", "method", "params"].includes(field),
    )
  ) {
    throw new ProtocolError(
      "invalid_request",
      "command envelope fields do not match ferry-ipc/1",
    );
  }
  return input as unknown as CommandEnvelope;
}

export function optionalString(
  params: Record<string, unknown>,
  key: string,
  max = 200_000,
): string | undefined {
  if (params[key] === undefined) return undefined;
  return requireString(params, key, max);
}

export function optionalInteger(
  params: Record<string, unknown>,
  key: string,
): number | undefined {
  if (params[key] === undefined) return undefined;
  return requireInteger(params, key);
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
