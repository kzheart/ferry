import type { CommandEnvelope } from "../server/messages.js";
import {
  ProtocolError,
  isObject,
  optionalInteger,
  optionalString,
  requireInteger,
  requireString,
} from "../server/messages.js";
import type { ProviderService } from "./provider-service.js";
import type { ThinkingLevel } from "./provider-config.js";
import { parseThinkingLevel } from "./provider-config-validation.js";

type ProviderCommandResult =
  | { handled: true; result: unknown }
  | { handled: false };

export async function dispatchProviderCommand(
  service: ProviderService,
  command: CommandEnvelope,
): Promise<ProviderCommandResult> {
  const params = command.params;
  switch (command.method) {
    case "providers.list":
      return { handled: true, result: await service.providers() };
    case "models.list":
      return {
        handled: true,
        result: service.models(
          requireString(params, "provider_id", 128),
          optionalString(params, "query", 256) ?? "",
          optionalInteger(params, "limit") ?? 100,
        ),
      };
    case "models.enabled":
      return { handled: true, result: await service.enabledModels() };
    case "models.catalog":
      return { handled: true, result: await service.catalogModels() };
    case "custom_model.add":
      return {
        handled: true,
        result: await service.saveCustomModel(
          requireString(params, "provider_id", 128),
          parseCustomModel(params),
        ),
      };
    case "custom_model.delete":
      return {
        handled: true,
        result: await service.deleteCustomModel(
          requireString(params, "provider_id", 128),
          requireString(params, "model_id", 512),
        ),
      };
    case "provider.test":
      return {
        handled: true,
        result: await service.testProvider(
          requireString(params, "provider_id", 128),
          optionalString(params, "model_id", 512),
        ),
      };
    case "provider.enabled.set":
      if (typeof params.enabled !== "boolean") {
        throw new ProtocolError("invalid_params", "enabled must be a boolean");
      }
      return {
        handled: true,
        result: await service.setProviderEnabled(
          requireString(params, "provider_id", 128),
          params.enabled,
        ),
      };
    case "models.visibility.set":
      return {
        handled: true,
        result: await service.setVisibleModels(
          requireString(params, "provider_id", 128),
          parseModelIds(params.model_ids),
        ),
      };
    case "models.refresh":
      return { handled: true, result: await service.refreshModels() };
    case "config.get":
      return { handled: true, result: await service.config() };
    case "credential.set": {
      const fields = params.fields;
      if (fields !== undefined && !isObject(fields)) {
        throw new ProtocolError("invalid_params", "fields must be an object");
      }
      return {
        handled: true,
        result: await service.saveApiKey(
          requireString(params, "provider_id", 128),
          requireString(params, "key", 64 * 1024),
          fields as Record<string, string> | undefined,
        ),
      };
    }
    case "provider.logout":
      return {
        handled: true,
        result: await service.logoutProvider(
          requireString(params, "provider_id", 128),
        ),
      };
    case "model.select":
      return {
        handled: true,
        result: await service.selectModel(
          optionalString(params, "session_id", 128),
          parseModelSelection(params),
        ),
      };
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
      return {
        handled: true,
        result: await service.saveCustomProvider(
          parseCustomProvider(params),
          params.clear_api_key === true,
        ),
      };
    case "custom_provider.delete":
      return {
        handled: true,
        result: await service.deleteCustomProvider(
          requireString(params, "provider_id", 128),
        ),
      };
    case "auth.login.start": {
      const authType = requireString(params, "auth_type", 16);
      if (authType !== "api_key" && authType !== "oauth") {
        throw new ProtocolError(
          "invalid_params",
          "auth_type must be api_key or oauth",
        );
      }
      return {
        handled: true,
        result: service.startAuthentication(
          requireString(params, "provider_id", 128),
          authType,
        ),
      };
    }
    case "auth.login.respond":
      return {
        handled: true,
        result: service.respondAuthentication(
          requireString(params, "login_id", 128),
          requireString(params, "prompt_id", 128),
          requireString(params, "value", 64 * 1024),
        ),
      };
    case "auth.login.cancel":
      return {
        handled: true,
        result: service.cancelAuthentication(
          requireString(params, "login_id", 128),
        ),
      };
    default:
      return { handled: false };
  }
}

function parseCustomModel(params: Record<string, unknown>) {
  const name = optionalString(params, "name", 256);
  const contextWindow = optionalInteger(params, "context_window");
  const maxTokens = optionalInteger(params, "max_tokens");
  return {
    id: requireString(params, "model_id", 512),
    ...(name ? { name } : {}),
    ...(typeof params.image === "boolean"
      ? {
          input: params.image
            ? (["text", "image"] as Array<"text" | "image">)
            : (["text"] as Array<"text" | "image">),
        }
      : {}),
    ...(typeof params.reasoning === "boolean"
      ? { reasoning: params.reasoning }
      : {}),
    ...(contextWindow ? { context_window: contextWindow } : {}),
    ...(maxTokens ? { max_tokens: maxTokens } : {}),
  };
}

function parseModelSelection(params: Record<string, unknown>) {
  let thinking: ThinkingLevel | undefined;
  try {
    thinking = parseThinkingLevel(params.thinking);
  } catch {
    throw new ProtocolError("invalid_params", "thinking is invalid");
  }
  return {
    provider: requireString(params, "provider_id", 128),
    model: requireString(params, "model_id", 512),
    ...(thinking ? { thinking } : {}),
  };
}

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
    if (!isObject(value)) {
      throw new ProtocolError("invalid_params", "custom model is invalid");
    }
    if (!Array.isArray(value.input)) {
      throw new ProtocolError("invalid_params", "model input is invalid");
    }
    return {
      id: requireString(value, "id", 512),
      ...(value.name === undefined
        ? {}
        : { name: requireString(value, "name", 256) }),
      input: value.input as Array<"text" | "image">,
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
