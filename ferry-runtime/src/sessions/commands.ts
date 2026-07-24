import type { ThinkingLevel } from "../providers/provider-config.js";
import { parseThinkingLevel } from "../providers/provider-config-validation.js";
import {
  ProtocolError,
  isObject,
  optionalString,
  requireInteger,
  requireString,
  type CommandEnvelope,
} from "../server/messages.js";
import type { AgentRuntime } from "../runtime/runtime.js";

type SessionCommandResult =
  | { handled: true; result: unknown }
  | { handled: false };

export async function dispatchSessionCommand(
  runtime: AgentRuntime,
  command: CommandEnvelope,
): Promise<SessionCommandResult> {
  const params = command.params;
  switch (command.method) {
    case "session.create":
      return { handled: true, result: await createSession(runtime, params) };
    case "session.rename":
      return {
        handled: true,
        result: await runtime.renameSession(
          requireString(params, "session_id", 128),
          requireString(params, "title", 200),
        ),
      };
    case "session.pin":
      if (typeof params.pinned !== "boolean") {
        throw new ProtocolError("invalid_params", "pinned must be a boolean");
      }
      return {
        handled: true,
        result: await runtime.pinSession(
          requireString(params, "session_id", 128),
          params.pinned,
        ),
      };
    case "session.delete":
      return {
        handled: true,
        result: await runtime.deleteSession(
          requireString(params, "session_id", 128),
        ),
      };
    case "prompt":
      return {
        handled: true,
        result: await runtime.prompt(
          requireString(params, "session_id", 128),
          requireString(params, "text"),
          parseImages(params.images),
          optionalString(params, "display_text") ??
            requireString(params, "text"),
        ),
      };
    case "abort":
      return {
        handled: true,
        result: runtime.abort(requireString(params, "session_id", 128)),
      };
    case "steer":
      return {
        handled: true,
        result: runtime.steer(
          requireString(params, "session_id", 128),
          requireString(params, "text"),
          optionalString(params, "display_text") ??
            requireString(params, "text"),
        ),
      };
    case "follow_up":
      return {
        handled: true,
        result: runtime.followUp(
          requireString(params, "session_id", 128),
          requireString(params, "text"),
          optionalString(params, "display_text") ??
            requireString(params, "text"),
        ),
      };
    case "state":
      return {
        handled: true,
        result: runtime.state(requireString(params, "session_id", 128)),
      };
    case "sessions.list":
      return { handled: true, result: runtime.listSessions() };
    case "events.replay":
      return {
        handled: true,
        result: runtime.replay(
          requireString(params, "session_id", 128),
          requireInteger(params, "after_seq"),
        ),
      };
    default:
      return { handled: false };
  }
}

async function createSession(
  runtime: AgentRuntime,
  params: Record<string, unknown>,
) {
  if ((params.provider_id === undefined) !== (params.model_id === undefined)) {
    throw new ProtocolError(
      "invalid_params",
      "provider_id and model_id must be provided together",
    );
  }
  let thinking: ThinkingLevel | undefined;
  try {
    thinking = parseThinkingLevel(params.thinking);
  } catch {
    throw new ProtocolError("invalid_params", "thinking is invalid");
  }
  return runtime.createSession(
    params.session_id === undefined
      ? undefined
      : requireString(params, "session_id", 128),
    params.provider_id === undefined
      ? undefined
      : {
          provider: requireString(params, "provider_id", 128),
          model: requireString(params, "model_id", 512),
          ...(thinking ? { thinking } : {}),
        },
    optionalString(params, "role_id", 128),
  );
}

function parseImages(value: unknown) {
  if (value === undefined) return [];
  if (!Array.isArray(value) || value.length > 8) {
    throw new ProtocolError(
      "invalid_params",
      "images must be an array of at most 8 items",
    );
  }
  let total = 0;
  return value.map((item) => {
    if (!isObject(item)) {
      throw new ProtocolError("invalid_params", "image is invalid");
    }
    const mimeType = requireString(item, "mime_type", 128);
    if (!/^image\/(?:png|jpeg|webp|gif)$/.test(mimeType)) {
      throw new ProtocolError(
        "invalid_params",
        "image MIME type is unsupported",
      );
    }
    const data = requireString(item, "data", 12 * 1024 * 1024);
    if (!/^[A-Za-z0-9+/]+={0,2}$/.test(data)) {
      throw new ProtocolError("invalid_params", "image data must be base64");
    }
    total += data.length;
    if (total > 12 * 1024 * 1024) {
      throw new ProtocolError("invalid_params", "image payload is too large");
    }
    return { type: "image" as const, data, mimeType };
  });
}
