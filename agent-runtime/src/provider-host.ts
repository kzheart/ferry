import {
  createModels,
  createProvider,
  type AuthEvent,
  type AuthInteraction,
  type AuthPrompt,
  type AuthType,
  type CredentialInfo,
  type Model,
  type MutableModels,
  type Provider,
} from "@earendil-works/pi-ai";
import { openAICompletionsApi } from "@earendil-works/pi-ai/api/openai-completions.lazy";
import { registerBunOAuthFlows } from "@earendil-works/pi-ai/bun-oauth";
import { builtinProviders } from "@earendil-works/pi-ai/providers/all";
import {
  FileProviderConfigStore,
  type CustomProviderConfig,
  type ModelSelection,
} from "./provider-config.js";

export const UNSUPPORTED_PROVIDER_IDS = new Set([
  "amazon-bedrock",
  "google-vertex",
]);

export interface ProviderSummary {
  id: string;
  name: string;
  configured: boolean;
  credential_type: CredentialInfo["type"] | null;
  auth_types: AuthType[];
  custom: boolean;
  model_count: number;
}

export interface ModelSummary {
  id: string;
  name: string;
  provider: string;
  api: string;
  reasoning: boolean;
  input: Array<"text" | "image">;
  context_window: number;
  max_tokens: number;
}

function customProvider(config: CustomProviderConfig): Provider {
  const models: Model<"openai-completions">[] = config.models.map((item) => ({
    id: item.id,
    name: item.name ?? item.id,
    api: "openai-completions",
    provider: config.id,
    baseUrl: config.base_url,
    reasoning: item.reasoning,
    input: item.input,
    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    contextWindow: item.context_window,
    maxTokens: item.max_tokens,
  }));
  return createProvider({
    id: config.id,
    name: config.name,
    baseUrl: config.base_url,
    auth: {
      apiKey: {
        name: `${config.name} API key`,
        async resolve() {
          return {
            auth: {
              ...(config.api_key ? { apiKey: config.api_key } : {}),
              baseUrl: config.base_url,
            },
            source: "Ferry provider config",
          };
        },
      },
    },
    models,
    api: openAICompletionsApi(),
  });
}

function summary(model: Model<string>): ModelSummary {
  return {
    id: model.id,
    name: model.name,
    provider: model.provider,
    api: model.api,
    reasoning: model.reasoning,
    input: [...model.input],
    context_window: model.contextWindow,
    max_tokens: model.maxTokens,
  };
}

export class ProviderHost {
  private constructor(
    readonly store: FileProviderConfigStore,
    readonly models: MutableModels,
    private customIds: Set<string>,
  ) {}

  static async create(store: FileProviderConfigStore) {
    registerBunOAuthFlows();
    const models = createModels({
      credentials: store,
      authContext: {
        async env() {
          return undefined;
        },
        async fileExists() {
          return false;
        },
      },
    });
    for (const provider of builtinProviders()) {
      if (!UNSUPPORTED_PROVIDER_IDS.has(provider.id)) {
        models.setProvider(provider);
      }
    }
    const config = await store.snapshot();
    const customIds = new Set<string>();
    for (const item of config.custom_providers) {
      if (models.getProvider(item.id)) {
        throw new Error(
          `custom provider conflicts with built-in provider: ${item.id}`,
        );
      }
      models.setProvider(customProvider(item));
      customIds.add(item.id);
    }
    return new ProviderHost(store, models, customIds);
  }

  async reloadCustomProviders() {
    for (const id of this.customIds) this.models.deleteProvider(id);
    this.customIds = new Set();
    for (const item of (await this.store.snapshot()).custom_providers) {
      if (this.models.getProvider(item.id)) {
        throw new Error(
          `custom provider conflicts with built-in provider: ${item.id}`,
        );
      }
      this.models.setProvider(customProvider(item));
      this.customIds.add(item.id);
    }
  }

  async providers(): Promise<ProviderSummary[]> {
    const credentials = new Map(
      (await this.store.list()).map((item) => [item.providerId, item.type]),
    );
    const output: ProviderSummary[] = [];
    for (const provider of this.models.getProviders()) {
      const credentialType = credentials.get(provider.id) ?? null;
      const authTypes: AuthType[] = [];
      if (provider.auth.apiKey) authTypes.push("api_key");
      if (provider.auth.oauth) authTypes.push("oauth");
      output.push({
        id: provider.id,
        name: provider.name,
        configured: this.customIds.has(provider.id) || credentialType !== null,
        credential_type: credentialType,
        auth_types: authTypes,
        custom: this.customIds.has(provider.id),
        model_count: provider.getModels().length,
      });
    }
    return output.sort((left, right) => left.name.localeCompare(right.name));
  }

  model(selection: ModelSelection) {
    const model = this.models.getModel(selection.provider, selection.model);
    if (!model) throw new Error("model is not available");
    return model;
  }

  listModels(providerId: string, query = "", limit = 100): ModelSummary[] {
    if (!this.models.getProvider(providerId))
      throw new Error("provider not found");
    const normalized = query.trim().toLocaleLowerCase();
    return this.models
      .getModels(providerId)
      .filter(
        (model) =>
          !normalized ||
          model.id.toLocaleLowerCase().includes(normalized) ||
          model.name.toLocaleLowerCase().includes(normalized),
      )
      .slice(0, Math.max(1, Math.min(limit, 200)))
      .map(summary);
  }

  async defaultModel() {
    return (await this.store.snapshot()).default_model;
  }

  isCustom(providerId: string) {
    return this.customIds.has(providerId);
  }

  async selectDefault(selection: ModelSelection) {
    this.model(selection);
    await this.store.setDefaultModel(selection);
    return selection;
  }

  async isConfigured(providerId: string) {
    if (this.customIds.has(providerId)) return true;
    return (await this.models.checkAuth(providerId)) !== undefined;
  }

  backend(selection: ModelSelection) {
    const model = this.model(selection);
    return {
      model,
      streamFn: this.models.streamSimple.bind(this.models),
      provider: model.provider,
      modelId: model.id,
    };
  }

  async saveApiKey(
    providerId: string,
    key: string,
    fields?: Record<string, string>,
  ) {
    const provider = this.models.getProvider(providerId);
    if (!provider?.auth.apiKey)
      throw new Error("provider does not support API key auth");
    await this.store.modify(providerId, async () => ({
      type: "api_key",
      key,
      ...(fields && Object.keys(fields).length > 0 ? { env: fields } : {}),
    }));
  }

  async logout(providerId: string) {
    if (!this.models.getProvider(providerId))
      throw new Error("provider not found");
    await this.models.logout(providerId);
  }

  async login(
    providerId: string,
    type: AuthType,
    interaction: AuthInteraction,
  ) {
    if (!this.models.getProvider(providerId))
      throw new Error("provider not found");
    const credential = await this.models.login(providerId, type, interaction);
    await this.refreshModels();
    return credential;
  }

  async refreshModels() {
    const result = await this.models.refresh({
      allowNetwork: true,
      force: true,
    });
    return {
      aborted: result.aborted,
      failed_provider_ids: [...result.errors.keys()].sort(),
    };
  }

  async saveCustomProvider(config: CustomProviderConfig) {
    if (!this.customIds.has(config.id) && this.models.getProvider(config.id)) {
      throw new Error("custom provider id conflicts with a built-in provider");
    }
    await this.store.saveCustomProvider(config);
    await this.reloadCustomProviders();
  }

  async deleteCustomProvider(providerId: string) {
    if (!this.customIds.has(providerId))
      throw new Error("custom provider not found");
    const defaultModel = await this.defaultModel();
    if (defaultModel.provider === providerId) {
      throw new Error("custom provider is the current default");
    }
    await this.store.deleteCustomProvider(providerId);
    await this.reloadCustomProviders();
  }
}

export type { AuthEvent, AuthPrompt };
