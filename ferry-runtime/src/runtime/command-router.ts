import type { ThinkingLevel } from "../providers/provider-config.js";
import { parseThinkingLevel } from "../providers/provider-config-validation.js";
import { dispatchProviderCommand } from "../providers/commands.js";
import { FERRY_CONTRACT_HASH } from "../server/generated/ipc.js";
import type { AgentRuntime } from "./runtime.js";
import { dispatchRoleCommand } from "../roles/commands.js";
import {
  PROTOCOL_VERSION,
  ProtocolError,
  isObject,
  optionalInteger,
  optionalString,
  requireInteger,
  requireString,
  type CommandEnvelope,
  type ResponseEnvelope,
} from "../server/messages.js";

export async function dispatch(
  runtime: AgentRuntime,
  command: CommandEnvelope,
): Promise<ResponseEnvelope> {
  try {
    const params = command.params;
    let result: unknown;
    const providerCommand = await dispatchProviderCommand(
      runtime.providerService,
      command,
    );
    const roleCommand = await dispatchRoleCommand(runtime.roleService, command);
    if (providerCommand.handled) {
      result = providerCommand.result;
    } else if (roleCommand.handled) {
      result = roleCommand.result;
    } else
      switch (command.method) {
        case "health":
          result = {
            status: "ready",
            service: "ferry-runtime",
            contract_hash: FERRY_CONTRACT_HASH,
            pi_version: "0.81.1",
            ...(await runtime.providerService.status()),
          };
          break;
        case "session.create":
          if (
            (params.provider_id === undefined) !==
            (params.model_id === undefined)
          ) {
            throw new ProtocolError(
              "invalid_params",
              "provider_id and model_id must be provided together",
            );
          }
          let createThinking: ThinkingLevel | undefined;
          try {
            createThinking = parseThinkingLevel(params.thinking);
          } catch {
            throw new ProtocolError("invalid_params", "thinking is invalid");
          }
          result = await runtime.createSession(
            params.session_id === undefined
              ? undefined
              : requireString(params, "session_id", 128),
            params.provider_id === undefined
              ? undefined
              : {
                  provider: requireString(params, "provider_id", 128),
                  model: requireString(params, "model_id", 512),
                  ...(createThinking ? { thinking: createThinking } : {}),
                },
            optionalString(params, "role_id", 128),
          );
          break;
        case "session.rename":
          result = await runtime.renameSession(
            requireString(params, "session_id", 128),
            requireString(params, "title", 200),
          );
          break;
        case "session.pin":
          if (typeof params.pinned !== "boolean") {
            throw new ProtocolError(
              "invalid_params",
              "pinned must be a boolean",
            );
          }
          result = await runtime.pinSession(
            requireString(params, "session_id", 128),
            params.pinned,
          );
          break;
        case "session.delete":
          result = await runtime.deleteSession(
            requireString(params, "session_id", 128),
          );
          break;
        case "organization.start":
          result = await runtime.startOrganization({
            sessions: params.sessions,
            locale: params.locale,
          });
          break;
        case "prompt":
          result = await runtime.prompt(
            requireString(params, "session_id", 128),
            requireString(params, "text"),
            parseImages(params.images),
            optionalString(params, "display_text") ??
              requireString(params, "text"),
          );
          break;
        case "abort":
          result = runtime.abort(requireString(params, "session_id", 128));
          break;
        case "steer":
          result = runtime.steer(
            requireString(params, "session_id", 128),
            requireString(params, "text"),
            optionalString(params, "display_text") ??
              requireString(params, "text"),
          );
          break;
        case "follow_up":
          result = runtime.followUp(
            requireString(params, "session_id", 128),
            requireString(params, "text"),
            optionalString(params, "display_text") ??
              requireString(params, "text"),
          );
          break;
        case "state":
          result = runtime.state(requireString(params, "session_id", 128));
          break;
        case "sessions.list":
          result = runtime.listSessions();
          break;
        case "events.replay":
          result = runtime.replay(
            requireString(params, "session_id", 128),
            requireInteger(params, "after_seq"),
          );
          break;
        case "tool.result":
          result = runtime.completeTool(
            requireString(params, "request_id", 128),
            requireString(params, "session_id", 128),
            params.ok === true,
            params.ok === true ? params.result : params.error,
          );
          break;
      }
    return { protocol: PROTOCOL_VERSION, id: command.id, ok: true, result };
  } catch (error) {
    const protocolError =
      error instanceof ProtocolError
        ? error
        : new ProtocolError("internal_error", "internal runtime error");
    return {
      protocol: PROTOCOL_VERSION,
      id: command.id,
      ok: false,
      error: protocolError.toEnvelope(),
    };
  }
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
    if (!isObject(item))
      throw new ProtocolError("invalid_params", "image is invalid");
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
