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
} from "@earendil-works/pi-ai";
import {
  createInitialProviderConfig,
  type CustomModelConfig,
  type CustomProviderConfig,
  type ModelSelection,
  type ProviderConfigDocument,
  type PublicProviderConfig,
} from "./provider-config.js";
import {
  fromCredential,
  parseCustomModel,
  parseCustomProvider,
  parseModelId,
  parseModelSelection,
  parseProviderConfig,
  parseProviderId,
  toCredential,
} from "./provider-config-validation.js";

const MAX_CONFIG_BYTES = 2 * 1024 * 1024;

export class FileProviderConfigStore implements CredentialStore {
  private document = createInitialProviderConfig();
  private ready: Promise<void>;
  private writeQueue = Promise.resolve();

  constructor(readonly path: string) {
    this.ready = this.load();
  }

  private async load() {
    await mkdir(dirname(this.path), { recursive: true, mode: 0o700 });
    const existing = await this.readDisk();
    if (existing) this.document = existing;
    else await this.writeDisk(this.document);
  }

  private async readDisk() {
    try {
      const source = await readFile(this.path, "utf8");
      if (Buffer.byteLength(source) > MAX_CONFIG_BYTES) {
        throw new Error("provider config is too large");
      }
      await chmod(this.path, 0o600);
      return parseProviderConfig(JSON.parse(source) as unknown);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") return undefined;
      throw error;
    }
  }

  private async writeDisk(document: ProviderConfigDocument) {
    const payload = JSON.stringify(document, null, 2);
    const temporary = `${this.path}.${process.pid}.${randomUUID()}.tmp`;
    try {
      await writeFile(temporary, payload, { encoding: "utf8", mode: 0o600 });
      await rename(temporary, this.path);
      await chmod(this.path, 0o600);
    } catch (error) {
      await unlink(temporary).catch(() => undefined);
      throw error;
    }
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
        await this.writeDisk(this.document);
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
        ([providerId, value]) => ({ providerId, type: value.type }),
      ),
      custom_providers: config.custom_providers.map(
        ({ api_key, ...provider }) => ({
          ...provider,
          configured: Boolean(api_key),
        }),
      ),
      enabled_providers: config.enabled_providers,
      visible_models: config.visible_models,
      custom_models: config.custom_models,
    };
  }

  async read(providerId: string) {
    const value = (await this.snapshot()).credentials[providerId];
    return value ? toCredential(value) : undefined;
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
    parseProviderId(providerId);
    return this.mutate(async (config) => {
      const stored = config.credentials[providerId];
      const current = stored ? toCredential(stored) : undefined;
      const next = await update(current);
      if (next !== undefined) {
        config.credentials[providerId] = fromCredential(next);
      }
      const saved = config.credentials[providerId];
      return saved ? toCredential(saved) : undefined;
    });
  }

  async delete(providerId: string) {
    parseProviderId(providerId);
    await this.mutate((config) => {
      delete config.credentials[providerId];
    });
  }

  async setDefaultModel(selection: ModelSelection) {
    const safe = parseModelSelection(selection);
    await this.mutate((config) => {
      config.default_model = safe;
    });
  }

  async setProviderEnabled(providerId: string, enabled: boolean) {
    parseProviderId(providerId);
    return this.mutate((config) => {
      const rest = config.enabled_providers.filter(
        (item) => item !== providerId,
      );
      config.enabled_providers = enabled ? [...rest, providerId] : rest;
      if (!enabled) delete config.visible_models[providerId];
      return config.enabled_providers;
    });
  }

  async setVisibleModels(providerId: string, modelIds: string[] | null) {
    parseProviderId(providerId);
    return this.mutate((config) => {
      if (modelIds === null) delete config.visible_models[providerId];
      else config.visible_models[providerId] = modelIds;
      return config.visible_models[providerId] ?? null;
    });
  }

  async saveCustomModel(providerId: string, model: CustomModelConfig) {
    parseProviderId(providerId);
    const safe = parseCustomModel(model);
    return this.mutate((config) => {
      const list = (config.custom_models[providerId] ?? []).filter(
        (item) => item.id !== safe.id,
      );
      config.custom_models[providerId] = [...list, safe];
      const visible = config.visible_models[providerId];
      if (visible && !visible.includes(safe.id)) visible.push(safe.id);
      return safe;
    });
  }

  async deleteCustomModel(providerId: string, modelId: string) {
    parseProviderId(providerId);
    parseModelId(modelId, "custom model id");
    return this.mutate((config) => {
      const list = (config.custom_models[providerId] ?? []).filter(
        (item) => item.id !== modelId,
      );
      if (list.length) config.custom_models[providerId] = list;
      else delete config.custom_models[providerId];
      const visible = config.visible_models[providerId];
      if (visible) {
        config.visible_models[providerId] = visible.filter(
          (item) => item !== modelId,
        );
      }
      return { provider_id: providerId, model_id: modelId };
    });
  }

  async saveCustomProvider(
    provider: CustomProviderConfig,
    clearApiKey = false,
  ) {
    const safe = parseCustomProvider(provider);
    await this.mutate((config) => {
      const existing = config.custom_providers.find(
        (item) => item.id === safe.id,
      );
      const replacement =
        !clearApiKey && !safe.api_key && existing?.api_key
          ? { ...safe, api_key: existing.api_key }
          : safe;
      config.custom_providers = config.custom_providers.filter(
        (item) => item.id !== safe.id,
      );
      config.custom_providers.push(replacement);
      if (!config.enabled_providers.includes(safe.id)) {
        config.enabled_providers.push(safe.id);
      }
    });
  }

  async deleteCustomProvider(providerId: string) {
    parseProviderId(providerId, "custom provider id");
    await this.mutate((config) => {
      config.custom_providers = config.custom_providers.filter(
        (item) => item.id !== providerId,
      );
      delete config.credentials[providerId];
      delete config.visible_models[providerId];
      delete config.custom_models[providerId];
      config.enabled_providers = config.enabled_providers.filter(
        (item) => item !== providerId,
      );
    });
  }
}
