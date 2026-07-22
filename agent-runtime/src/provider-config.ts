import { randomUUID } from "node:crypto";
import {
  chmod,
  mkdir,
  readFile,
  rename,
  unlink,
  writeFile,
} from "node:fs/promises";
import { dirname } from "node:path";
import type {
  Credential,
  CredentialInfo,
  CredentialStore,
  ProviderEnv,
} from "@earendil-works/pi-ai";

export const PROVIDER_CONFIG_VERSION = 1 as const;
const MAX_SECRET_BYTES = 64 * 1024;
const MAX_CONFIG_BYTES = 2 * 1024 * 1024;

export interface ModelSelection {
  provider: string;
  model: string;
}

export interface CustomProviderConfig {
  id: string;
  name: string;
  base_url: string;
  api_key?: string;
  models: CustomModelConfig[];
}

export interface CustomModelConfig {
  id: string;
  name?: string;
  input: Array<"text" | "image">;
  reasoning: boolean;
  context_window: number;
  max_tokens: number;
}

export interface ProviderConfigDocument {
  schema_version: typeof PROVIDER_CONFIG_VERSION;
  default_model: ModelSelection;
  credentials: Record<string, Credential>;
  custom_providers: CustomProviderConfig[];
}

export interface PublicProviderConfig {
  schema_version: typeof PROVIDER_CONFIG_VERSION;
  default_model: ModelSelection;
  credentials: CredentialInfo[];
  custom_providers: Array<
    Omit<CustomProviderConfig, "api_key"> & { configured: boolean }
  >;
}

function initialConfig(): ProviderConfigDocument {
  return {
    schema_version: PROVIDER_CONFIG_VERSION,
    default_model: { provider: "deepseek", model: "deepseek-v4-flash" },
    credentials: {},
    custom_providers: [],
  };
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function identifier(value: unknown, label: string): string {
  if (
    typeof value !== "string" ||
    !/^[a-z0-9][a-z0-9._-]{0,127}$/i.test(value)
  ) {
    throw new Error(`${label} is invalid`);
  }
  return value;
}

function modelIdentifier(value: unknown, label: string): string {
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
  if (value === undefined) return undefined;
  return text(value, label, MAX_SECRET_BYTES);
}

function providerEnv(value: unknown): ProviderEnv | undefined {
  if (value === undefined) return undefined;
  if (!record(value)) throw new Error("credential env is invalid");
  const entries = Object.entries(value);
  if (entries.length > 64) throw new Error("credential env is too large");
  return Object.fromEntries(
    entries.map(([key, item]) => [
      identifier(key, "credential env key"),
      text(item, "credential env value", MAX_SECRET_BYTES),
    ]),
  );
}

function credential(value: unknown): Credential {
  if (!record(value)) throw new Error("credential is invalid");
  if (value.type === "api_key") {
    const key = optionalSecret(value.key, "API key");
    const env = providerEnv(value.env);
    if (!key && !env) throw new Error("API key credential is empty");
    return {
      type: "api_key",
      ...(key ? { key } : {}),
      ...(env ? { env } : {}),
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
    const safe: Record<string, unknown> = {
      ...value,
      type: "oauth",
      access,
      refresh,
      expires: value.expires,
    };
    if (JSON.stringify(safe).length > MAX_SECRET_BYTES * 2) {
      throw new Error("OAuth credential is too large");
    }
    return safe as Credential;
  }
  throw new Error("credential type is unsupported");
}

function customProvider(value: unknown): CustomProviderConfig {
  if (!record(value)) throw new Error("custom provider is invalid");
  const models = value.models;
  if (!Array.isArray(models) || models.length === 0 || models.length > 200) {
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
    id: identifier(value.id, "custom provider id"),
    name: text(value.name, "custom provider name", 256),
    base_url: parsed.toString().replace(/\/$/, ""),
    ...(apiKey ? { api_key: apiKey } : {}),
    models: models.map(customModel),
  };
}

function customModel(value: unknown): CustomModelConfig {
  if (!record(value)) throw new Error("custom model is invalid");
  const input = value.input;
  if (
    !Array.isArray(input) ||
    input.length === 0 ||
    !input.every((item) => item === "text" || item === "image")
  ) {
    throw new Error("custom model input is invalid");
  }
  const number = (item: unknown, label: string, maximum: number) => {
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
    id: modelIdentifier(value.id, "custom model id"),
    ...(value.name === undefined
      ? {}
      : { name: text(value.name, "custom model name", 256) }),
    input: [...new Set(input)] as Array<"text" | "image">,
    reasoning: value.reasoning === true,
    context_window: number(
      value.context_window,
      "custom model context_window",
      10_000_000,
    ),
    max_tokens: number(value.max_tokens, "custom model max_tokens", 1_000_000),
  };
}

export function parseProviderConfig(value: unknown): ProviderConfigDocument {
  if (!record(value) || value.schema_version !== PROVIDER_CONFIG_VERSION) {
    throw new Error("provider config schema is unsupported");
  }
  if (!record(value.default_model) || !record(value.credentials)) {
    throw new Error("provider config is invalid");
  }
  const credentials = Object.fromEntries(
    Object.entries(value.credentials).map(([providerId, item]) => [
      identifier(providerId, "provider id"),
      credential(item),
    ]),
  );
  if (!Array.isArray(value.custom_providers)) {
    throw new Error("custom providers are invalid");
  }
  const customProviders = value.custom_providers.map(customProvider);
  if (
    new Set(customProviders.map((provider) => provider.id)).size !==
    customProviders.length
  ) {
    throw new Error("custom provider ids must be unique");
  }
  return {
    schema_version: PROVIDER_CONFIG_VERSION,
    default_model: {
      provider: identifier(value.default_model.provider, "default provider"),
      model: modelIdentifier(value.default_model.model, "default model"),
    },
    credentials,
    custom_providers: customProviders,
  };
}

export class FileProviderConfigStore implements CredentialStore {
  private document = initialConfig();
  private ready: Promise<void>;
  private writeQueue = Promise.resolve();

  constructor(readonly path: string) {
    this.ready = this.load();
  }

  private async load() {
    await mkdir(dirname(this.path), { recursive: true, mode: 0o700 });
    try {
      const source = await readFile(this.path, "utf8");
      if (Buffer.byteLength(source) > MAX_CONFIG_BYTES) {
        throw new Error("provider config is too large");
      }
      this.document = parseProviderConfig(JSON.parse(source) as unknown);
      await chmod(this.path, 0o600);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
      await this.persist();
    }
  }

  private persist() {
    const payload = JSON.stringify(this.document, null, 2);
    const task = this.writeQueue
      .catch(() => undefined)
      .then(async () => {
        const temporary = `${this.path}.${process.pid}.${randomUUID()}.tmp`;
        try {
          await writeFile(temporary, payload, {
            encoding: "utf8",
            mode: 0o600,
          });
          await rename(temporary, this.path);
          await chmod(this.path, 0o600);
        } catch (error) {
          await unlink(temporary).catch(() => undefined);
          throw error;
        }
      });
    this.writeQueue = task;
    return task;
  }

  private async mutate<T>(
    action: (document: ProviderConfigDocument) => T | Promise<T>,
  ) {
    await this.ready;
    const previous = this.writeQueue;
    let result!: T;
    const task = previous
      .catch(() => undefined)
      .then(async () => {
        const draft = structuredClone(this.document);
        result = await action(draft);
        this.document = parseProviderConfig(draft);
        const payload = JSON.stringify(this.document, null, 2);
        const temporary = `${this.path}.${process.pid}.${randomUUID()}.tmp`;
        try {
          await writeFile(temporary, payload, {
            encoding: "utf8",
            mode: 0o600,
          });
          await rename(temporary, this.path);
          await chmod(this.path, 0o600);
        } catch (error) {
          await unlink(temporary).catch(() => undefined);
          throw error;
        }
      });
    this.writeQueue = task;
    await task;
    return result;
  }

  async snapshot() {
    await this.ready;
    await this.writeQueue;
    return structuredClone(this.document);
  }

  async publicSnapshot(): Promise<PublicProviderConfig> {
    const config = await this.snapshot();
    return {
      schema_version: config.schema_version,
      default_model: config.default_model,
      credentials: Object.entries(config.credentials).map(
        ([providerId, value]) => ({
          providerId,
          type: value.type,
        }),
      ),
      custom_providers: config.custom_providers.map(
        ({ api_key, ...provider }) => ({
          ...provider,
          configured: Boolean(api_key),
        }),
      ),
    };
  }

  async read(providerId: string) {
    return structuredClone((await this.snapshot()).credentials[providerId]);
  }

  async list(): Promise<readonly CredentialInfo[]> {
    return (await this.publicSnapshot()).credentials;
  }

  async modify(
    providerId: string,
    update: (
      current: Credential | undefined,
    ) => Promise<Credential | undefined>,
  ) {
    identifier(providerId, "provider id");
    return this.mutate(async (config) => {
      const current = structuredClone(config.credentials[providerId]);
      const next = await update(current);
      if (next !== undefined) config.credentials[providerId] = credential(next);
      return structuredClone(config.credentials[providerId]);
    });
  }

  async delete(providerId: string) {
    identifier(providerId, "provider id");
    await this.mutate((config) => {
      delete config.credentials[providerId];
    });
  }

  async setDefaultModel(selection: ModelSelection) {
    const safe = {
      provider: identifier(selection.provider, "default provider"),
      model: modelIdentifier(selection.model, "default model"),
    };
    await this.mutate((config) => {
      config.default_model = safe;
    });
  }

  async saveCustomProvider(provider: CustomProviderConfig) {
    const safe = customProvider(provider);
    await this.mutate((config) => {
      config.custom_providers = config.custom_providers.filter(
        (item) => item.id !== safe.id,
      );
      config.custom_providers.push(safe);
    });
  }

  async deleteCustomProvider(providerId: string) {
    identifier(providerId, "custom provider id");
    await this.mutate((config) => {
      config.custom_providers = config.custom_providers.filter(
        (item) => item.id !== providerId,
      );
      delete config.credentials[providerId];
    });
  }
}
