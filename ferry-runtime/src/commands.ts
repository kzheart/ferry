import { parseThinkingLevel, type ThinkingLevel } from "./provider-config.js";
import { FERRY_CONTRACT_HASH } from "./contracts/ipc.js";
import type { AgentRuntime } from "./runtime.js";
import type { RoleInput } from "./roles.js";
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
} from "./protocol.js";

export async function dispatch(
  runtime: AgentRuntime,
  command: CommandEnvelope,
): Promise<ResponseEnvelope> {
  try {
    const params = command.params;
    let result: unknown;
    switch (command.method) {
      case "health":
        result = {
          status: "ready",
          service: "ferry-runtime",
          contract_hash: FERRY_CONTRACT_HASH,
          pi_version: "0.81.1",
          ...(await runtime.providerStatus()),
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
          throw new ProtocolError("invalid_params", "pinned must be a boolean");
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
      case "roles.list":
        result = await runtime.roles();
        break;
      case "role.create":
        result = await runtime.createRole(requireRole(params));
        break;
      case "role.update":
        result = await runtime.updateRole(
          requireString(params, "role_id", 128),
          requireRole(params),
        );
        break;
      case "role.copy":
        result = await runtime.copyRole(
          requireString(params, "source_role_id", 128),
          requireString(params, "role_id", 128),
          optionalString(params, "name", 200),
        );
        break;
      case "role.delete":
        result = await runtime.deleteRole(
          requireString(params, "role_id", 128),
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
      case "providers.list":
        result = await runtime.providers();
        break;
      case "models.list":
        result = runtime.models(
          requireString(params, "provider_id", 128),
          optionalString(params, "query", 256) ?? "",
          optionalInteger(params, "limit") ?? 100,
        );
        break;
      case "models.enabled":
        result = await runtime.enabledModels();
        break;
      case "models.catalog":
        result = await runtime.catalogModels();
        break;
      case "custom_model.add": {
        const name = optionalString(params, "name", 256);
        const contextWindow = optionalInteger(params, "context_window");
        const maxTokens = optionalInteger(params, "max_tokens");
        result = await runtime.saveCustomModel(
          requireString(params, "provider_id", 128),
          {
            id: requireString(params, "model_id", 512),
            ...(name ? { name } : {}),
            ...(typeof params.image === "boolean"
              ? { input: params.image ? ["text", "image"] : ["text"] }
              : {}),
            ...(typeof params.reasoning === "boolean"
              ? { reasoning: params.reasoning }
              : {}),
            ...(contextWindow ? { context_window: contextWindow } : {}),
            ...(maxTokens ? { max_tokens: maxTokens } : {}),
          },
        );
        break;
      }
      case "custom_model.delete":
        result = await runtime.deleteCustomModel(
          requireString(params, "provider_id", 128),
          requireString(params, "model_id", 512),
        );
        break;
      case "provider.test":
        result = await runtime.testProvider(
          requireString(params, "provider_id", 128),
          optionalString(params, "model_id", 512),
        );
        break;
      case "provider.enabled.set":
        if (typeof params.enabled !== "boolean") {
          throw new ProtocolError(
            "invalid_params",
            "enabled must be a boolean",
          );
        }
        result = await runtime.setProviderEnabled(
          requireString(params, "provider_id", 128),
          params.enabled,
        );
        break;
      case "models.visibility.set":
        result = await runtime.setVisibleModels(
          requireString(params, "provider_id", 128),
          parseModelIds(params.model_ids),
        );
        break;
      case "models.refresh":
        result = await runtime.refreshModels();
        break;
      case "config.get":
        result = await runtime.config();
        break;
      case "credential.set": {
        const fields = params.fields;
        if (fields !== undefined && !isObject(fields)) {
          throw new ProtocolError("invalid_params", "fields must be an object");
        }
        result = await runtime.saveApiKey(
          requireString(params, "provider_id", 128),
          requireString(params, "key", 64 * 1024),
          fields as Record<string, string> | undefined,
        );
        break;
      }
      case "provider.logout":
        result = await runtime.logoutProvider(
          requireString(params, "provider_id", 128),
        );
        break;
      case "model.select": {
        let thinking: ThinkingLevel | undefined;
        try {
          thinking = parseThinkingLevel(params.thinking);
        } catch {
          throw new ProtocolError("invalid_params", "thinking is invalid");
        }
        result = await runtime.selectModel(
          optionalString(params, "session_id", 128),
          {
            provider: requireString(params, "provider_id", 128),
            model: requireString(params, "model_id", 512),
            ...(thinking ? { thinking } : {}),
          },
        );
        break;
      }
      case "custom_provider.upsert":
        if (
          params.clear_api_key !== undefined &&
          typeof params.clear_api_key !== "boolean"
        ) {
          throw new ProtocolError(
            "invalid_params",
            "clear_api_key must be a boolean",
          );
        }
        result = await runtime.saveCustomProvider(
          parseCustomProvider(params),
          params.clear_api_key === true,
        );
        break;
      case "custom_provider.delete":
        result = await runtime.deleteCustomProvider(
          requireString(params, "provider_id", 128),
        );
        break;
      case "auth.login.start": {
        const authType = requireString(params, "auth_type", 16);
        if (authType !== "api_key" && authType !== "oauth") {
          throw new ProtocolError(
            "invalid_params",
            "auth_type must be api_key or oauth",
          );
        }
        result = runtime.startAuthentication(
          requireString(params, "provider_id", 128),
          authType,
        );
        break;
      }
      case "auth.login.respond":
        result = runtime.respondAuthentication(
          requireString(params, "login_id", 128),
          requireString(params, "prompt_id", 128),
          requireString(params, "value", 64 * 1024),
        );
        break;
      case "auth.login.cancel":
        result = runtime.cancelAuthentication(
          requireString(params, "login_id", 128),
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
        : new ProtocolError(
            "internal_error",
            "internal runtime error",
            "internal",
          );
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

function requireRole(params: Record<string, unknown>): RoleInput {
  if (!isObject(params.role)) {
    throw new ProtocolError("invalid_params", "role must be an object");
  }
  return params.role as unknown as RoleInput;
}

// null 表示恢复「该 Provider 全部模型可见」
function parseModelIds(value: unknown): string[] | null {
  if (value === null || value === undefined) return null;
  if (!Array.isArray(value) || value.length > 500) {
    throw new ProtocolError(
      "invalid_params",
      "model_ids must be an array of at most 500 items",
    );
  }
  return value.map((item) => {
    if (typeof item !== "string" || !item || item.length > 512) {
      throw new ProtocolError("invalid_params", "model id is invalid");
    }
    return item;
  });
}

function parseCustomProvider(params: Record<string, unknown>) {
  const values = params.models;
  if (!Array.isArray(values)) {
    throw new ProtocolError("invalid_params", "models must be an array");
  }
  const models = values.map((value) => {
    if (!isObject(value))
      throw new ProtocolError("invalid_params", "custom model is invalid");
    const input = value.input;
    if (!Array.isArray(input))
      throw new ProtocolError("invalid_params", "model input is invalid");
    return {
      id: requireString(value, "id", 512),
      ...(value.name === undefined
        ? {}
        : { name: requireString(value, "name", 256) }),
      input: input as Array<"text" | "image">,
      reasoning: value.reasoning === true,
      context_window: requireInteger(value, "context_window"),
      max_tokens: requireInteger(value, "max_tokens"),
    };
  });
  return {
    id: requireString(params, "provider_id", 128),
    name: requireString(params, "name", 256),
    base_url: requireString(params, "base_url", 4_096),
    ...(params.api_key === undefined
      ? {}
      : { api_key: requireString(params, "api_key", 64 * 1024) }),
    models,
  };
}
