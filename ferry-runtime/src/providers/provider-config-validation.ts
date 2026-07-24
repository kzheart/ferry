import type { Credential, ProviderEnv } from "@earendil-works/pi-ai";
import {
  PROVIDER_CONFIG_VERSION,
  THINKING_LEVELS,
  type CustomModelConfig,
  type CustomProviderConfig,
  type ModelSelection,
  type ProviderConfigDocument,
  type StoredCredential,
  type ThinkingLevel,
} from "./provider-config.js";

const MAX_SECRET_BYTES = 64 * 1024;
const MAX_VISIBLE_MODELS = 500;

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function parseProviderId(value: unknown, label = "provider id"): string {
  if (
    typeof value !== "string" ||
    !/^[a-z0-9][a-z0-9._-]{0,127}$/i.test(value)
  ) {
    throw new Error(`${label} is invalid`);
  }
  return value;
}

export function parseModelId(value: unknown, label = "model id"): string {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value.length > 512 ||
    value.trim() !== value ||
    /[\0\r\n]/.test(value)
  ) {
    throw new Error(`${label} is invalid`);
  }
  return value;
}

function text(value: unknown, label: string, max = 8_192): string {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    Buffer.byteLength(value) > max ||
    value.includes("\0")
  ) {
    throw new Error(`${label} is invalid`);
  }
  return value;
}

function optionalSecret(value: unknown, label: string): string | undefined {
  return value === undefined ? undefined : text(value, label, MAX_SECRET_BYTES);
}

function providerFields(value: unknown): ProviderEnv | undefined {
  if (value === undefined) return undefined;
  if (!record(value)) throw new Error("credential fields are invalid");
  const entries = Object.entries(value);
  if (entries.length > 64) throw new Error("credential fields are too large");
  return Object.fromEntries(
    entries.map(([key, item]) => [
      parseProviderId(key, "credential field key"),
      text(item, "credential field value", MAX_SECRET_BYTES),
    ]),
  );
}

function storedCredential(value: unknown): StoredCredential {
  if (!record(value)) throw new Error("credential is invalid");
  if (value.type === "api_key") {
    const key = optionalSecret(value.key, "API key");
    const fields = providerFields(value.fields);
    if (!key && !fields) throw new Error("API key credential is empty");
    return {
      type: "api_key",
      ...(key ? { key } : {}),
      ...(fields ? { fields } : {}),
    };
  }
  if (value.type === "oauth") {
    const access = text(value.access, "OAuth access token", MAX_SECRET_BYTES);
    const refresh = text(
      value.refresh,
      "OAuth refresh token",
      MAX_SECRET_BYTES,
    );
    if (!Number.isSafeInteger(value.expires) || (value.expires as number) < 0) {
      throw new Error("OAuth expiry is invalid");
    }
    const safe: StoredCredential = {
      ...value,
      type: "oauth",
      access,
      refresh,
      expires: value.expires as number,
    };
    if (JSON.stringify(safe).length > MAX_SECRET_BYTES * 2) {
      throw new Error("OAuth credential is too large");
    }
    return safe;
  }
  throw new Error("credential type is unsupported");
}

export function fromCredential(value: Credential): StoredCredential {
  if (value.type === "api_key") {
    return storedCredential({
      type: "api_key",
      ...(value.key ? { key: value.key } : {}),
      ...(value.env ? { fields: value.env } : {}),
    });
  }
  return storedCredential(value);
}

export function toCredential(value: StoredCredential): Credential {
  if (value.type === "api_key") {
    return {
      type: "api_key",
      ...(value.key ? { key: value.key } : {}),
      ...(value.fields ? { env: value.fields } : {}),
    };
  }
  return structuredClone(value) as Credential;
}

export function parseCustomModel(value: unknown): CustomModelConfig {
  if (!record(value)) throw new Error("custom model is invalid");
  if (
    !Array.isArray(value.input) ||
    value.input.length === 0 ||
    !value.input.every((item) => item === "text" || item === "image")
  ) {
    throw new Error("custom model input is invalid");
  }
  const positiveInteger = (
    item: unknown,
    label: string,
    maximum: number,
  ): number => {
    if (
      !Number.isSafeInteger(item) ||
      (item as number) <= 0 ||
      (item as number) > maximum
    ) {
      throw new Error(`${label} is invalid`);
    }
    return item as number;
  };
  return {
    id: parseModelId(value.id, "custom model id"),
    ...(value.name === undefined
      ? {}
      : { name: text(value.name, "custom model name", 256) }),
    input: [...new Set(value.input)] as Array<"text" | "image">,
    reasoning: value.reasoning === true,
    context_window: positiveInteger(
      value.context_window,
      "custom model context_window",
      10_000_000,
    ),
    max_tokens: positiveInteger(
      value.max_tokens,
      "custom model max_tokens",
      1_000_000,
    ),
  };
}

export function parseCustomProvider(value: unknown): CustomProviderConfig {
  if (!record(value)) throw new Error("custom provider is invalid");
  if (
    !Array.isArray(value.models) ||
    value.models.length === 0 ||
    value.models.length > 200
  ) {
    throw new Error("custom provider models are invalid");
  }
  const baseUrl = text(value.base_url, "custom provider base_url", 4_096);
  const parsed = new URL(baseUrl);
  if (!["http:", "https:"].includes(parsed.protocol)) {
    throw new Error("custom provider base_url must use HTTP or HTTPS");
  }
  if (parsed.username || parsed.password || parsed.hash) {
    throw new Error(
      "custom provider base_url cannot contain credentials or fragments",
    );
  }
  const apiKey = optionalSecret(value.api_key, "custom provider API key");
  return {
    id: parseProviderId(value.id, "custom provider id"),
    name: text(value.name, "custom provider name", 256),
    base_url: parsed.toString().replace(/\/$/, ""),
    ...(apiKey ? { api_key: apiKey } : {}),
    models: value.models.map(parseCustomModel),
  };
}

export function parseThinkingLevel(value: unknown): ThinkingLevel | undefined {
  if (value === undefined) return undefined;
  if (!THINKING_LEVELS.includes(value as ThinkingLevel)) {
    throw new Error("thinking level is invalid");
  }
  return value as ThinkingLevel;
}

export function parseModelSelection(value: ModelSelection): ModelSelection {
  const thinking = parseThinkingLevel(value.thinking);
  return {
    provider: parseProviderId(value.provider, "default provider"),
    model: parseModelId(value.model, "default model"),
    ...(thinking ? { thinking } : {}),
  };
}

function providerIdList(value: unknown, label: string): string[] {
  if (!Array.isArray(value) || value.length > 200) {
    throw new Error(`${label} is invalid`);
  }
  return [...new Set(value.map((item) => parseProviderId(item, label)))];
}

function visibleModels(value: unknown): Record<string, string[]> {
  if (!record(value)) throw new Error("visible models are invalid");
  return Object.fromEntries(
    Object.entries(value).map(([providerId, ids]) => {
      if (!Array.isArray(ids) || ids.length > MAX_VISIBLE_MODELS) {
        throw new Error("visible models are invalid");
      }
      return [
        parseProviderId(providerId),
        [...new Set(ids.map((id) => parseModelId(id, "visible model id")))],
      ];
    }),
  );
}

function customModelMap(value: unknown): Record<string, CustomModelConfig[]> {
  if (!record(value)) throw new Error("custom models are invalid");
  return Object.fromEntries(
    Object.entries(value).map(([providerId, list]) => {
      if (!Array.isArray(list) || list.length > 200) {
        throw new Error("custom models are invalid");
      }
      const models = list.map(parseCustomModel);
      if (new Set(models.map((item) => item.id)).size !== models.length) {
        throw new Error("custom model ids must be unique");
      }
      return [parseProviderId(providerId), models];
    }),
  );
}

export function parseProviderConfig(value: unknown): ProviderConfigDocument {
  if (!record(value) || value.schema_version !== PROVIDER_CONFIG_VERSION) {
    throw new Error("provider config schema is unsupported");
  }
  if (!record(value.default_model) || !record(value.credentials)) {
    throw new Error("provider config is invalid");
  }
  if (!Array.isArray(value.custom_providers)) {
    throw new Error("custom providers are invalid");
  }
  const customProviders = value.custom_providers.map(parseCustomProvider);
  if (
    new Set(customProviders.map((provider) => provider.id)).size !==
    customProviders.length
  ) {
    throw new Error("custom provider ids must be unique");
  }
  return {
    schema_version: PROVIDER_CONFIG_VERSION,
    default_model: parseModelSelection(
      value.default_model as unknown as ModelSelection,
    ),
    credentials: Object.fromEntries(
      Object.entries(value.credentials).map(([providerId, item]) => [
        parseProviderId(providerId),
        storedCredential(item),
      ]),
    ),
    custom_providers: customProviders,
    enabled_providers: providerIdList(
      value.enabled_providers,
      "enabled provider id",
    ),
    visible_models: visibleModels(value.visible_models),
    custom_models: customModelMap(value.custom_models),
  };
}
