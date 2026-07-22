import type { AgentRuntime } from "./runtime.js";
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
    const params = command.params ?? {};
    let result: unknown;
    switch (command.method) {
      case "health":
        result = {
          status: "ok",
          protocol: PROTOCOL_VERSION,
          runtime: "ferry-agent",
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
        result = await runtime.createSession(
          params.session_id === undefined
            ? undefined
            : requireString(params, "session_id", 128),
          params.provider_id === undefined
            ? undefined
            : {
                provider: requireString(params, "provider_id", 128),
                model: requireString(params, "model_id", 512),
              },
        );
        break;
      case "prompt":
        result = await runtime.prompt(
          requireString(params, "session_id", 128),
          requireString(params, "text"),
          parseImages(params.images),
        );
        break;
      case "abort":
        result = runtime.abort(requireString(params, "session_id", 128));
        break;
      case "steer":
        result = runtime.steer(
          requireString(params, "session_id", 128),
          requireString(params, "text"),
        );
        break;
      case "follow_up":
        result = runtime.followUp(
          requireString(params, "session_id", 128),
          requireString(params, "text"),
        );
        break;
      case "state":
        result = runtime.state(requireString(params, "session_id", 128));
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
      case "model.select":
        result = await runtime.selectModel(
          optionalString(params, "session_id", 128),
          {
            provider: requireString(params, "provider_id", 128),
            model: requireString(params, "model_id", 512),
          },
        );
        break;
      case "custom_provider.upsert":
        result = await runtime.saveCustomProvider(parseCustomProvider(params));
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
        : new ProtocolError("internal_error", "internal runtime error");
    return {
      protocol: PROTOCOL_VERSION,
      id: command.id,
      ok: false,
      error: { code: protocolError.code, message: protocolError.message },
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
