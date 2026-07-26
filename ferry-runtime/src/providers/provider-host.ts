import {
  createModels,
  type AuthInteraction,
  type AuthType,
  type MutableModels,
  type Provider,
} from "@earendil-works/pi-ai";
import { registerBunOAuthFlows } from "@earendil-works/pi-ai/bun-oauth";
import { builtinProviders } from "@earendil-works/pi-ai/providers/all";
import { dirname, join } from "node:path";
import { FileModelsStore } from "./model-store.js";
import {
  organizerPrompt,
  validateOrganizerResult,
  type OrganizerInput,
} from "../organizing/organizer.js";
import type {
  CustomModelConfig,
  CustomProviderConfig,
  ModelSelection,
} from "./provider-config.js";
import { FileProviderConfigStore } from "./provider-config-store.js";
import {
  UNSUPPORTED_PROVIDER_IDS,
  customProvider,
  modelSummary,
  withCustomModels,
  type ModelSummary,
  type ProviderSummary,
} from "./provider-models.js";

type CompleteArgs = Parameters<MutableModels["completeSimple"]>;

export class ProviderHost {
  // 未叠加手填模型的原始 Provider,重新叠加时要从它出发,否则会套娃
  private baseProviders = new Map<string, Provider>();

  private constructor(
    readonly store: FileProviderConfigStore,
    readonly models: MutableModels,
    private customIds: Set<string>,
  ) {}

  static async create(store: FileProviderConfigStore) {
    registerBunOAuthFlows();
    const models = createModels({
      credentials: store,
      modelsStore: new FileModelsStore(
        join(dirname(store.path), "model-catalogs"),
      ),
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
    ProviderHost.registerCustomProviders(
      models,
      config.custom_providers,
      customIds,
    );
    await models.refresh({ allowNetwork: false });
    const host = new ProviderHost(store, models, customIds);
    await host.applyCustomModels();
    return host;
  }

  /** 注册自定义 Provider 并记录其 id;与内置 Provider 撞 id 直接报错。 */
  private static registerCustomProviders(
    models: MutableModels,
    configs: readonly CustomProviderConfig[],
    ids: Set<string>,
  ) {
    for (const item of configs) {
      if (models.getProvider(item.id)) {
        throw new Error(
          `custom provider conflicts with built-in provider: ${item.id}`,
        );
      }
      models.setProvider(customProvider(item));
      ids.add(item.id);
    }
  }

  /** 发一条一次性补全,取回拼接后的文本;超时与错误 stopReason 统一在这里处理。 */
  private async completeText(
    model: CompleteArgs[0],
    request: CompleteArgs[1],
    options: { maxTokens: number; timeoutMs: number; errorMessage: string },
  ) {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), options.timeoutMs);
    try {
      const message = await this.models.completeSimple(model, request, {
        maxTokens: options.maxTokens,
        signal: controller.signal,
      });
      if (message.stopReason === "error" || message.stopReason === "aborted") {
        throw new Error(message.errorMessage ?? options.errorMessage);
      }
      return message.content
        .filter((part) => part.type === "text")
        .map((part) => part.text)
        .join("");
    } finally {
      clearTimeout(timer);
    }
  }

  private async applyCustomModels() {
    const config = await this.store.snapshot();
    for (const provider of this.models.getProviders()) {
      if (!this.baseProviders.has(provider.id)) {
        this.baseProviders.set(provider.id, provider);
      }
    }
    for (const [id, base] of [...this.baseProviders]) {
      if (!this.models.getProvider(id)) {
        this.baseProviders.delete(id);
        continue;
      }
      const configs = config.custom_models[id];
      this.models.setProvider(
        configs?.length ? withCustomModels(base, configs) : base,
      );
    }
  }

  async reloadCustomProviders() {
    for (const id of this.customIds) {
      this.models.deleteProvider(id);
      this.baseProviders.delete(id);
    }
    this.customIds = new Set();
    ProviderHost.registerCustomProviders(
      this.models,
      (await this.store.snapshot()).custom_providers,
      this.customIds,
    );
    await this.applyCustomModels();
  }

  // 手填一个模型 ID:未指定的能力字段沿用该 Provider 现有模型的参数
  async saveCustomModel(
    providerId: string,
    input: Partial<CustomModelConfig> & { id: string },
  ) {
    const base = this.baseProviders.get(providerId);
    if (!base) throw new Error("provider not found");
    const template = base.getModels()[0];
    if (!template) {
      throw new Error("provider has no reference model; refresh the catalog");
    }
    const contextWindow = input.context_window ?? template.contextWindow;
    const saved = await this.store.saveCustomModel(providerId, {
      id: input.id,
      ...(input.name ? { name: input.name } : {}),
      input: input.input ?? [...template.input],
      reasoning: input.reasoning ?? template.reasoning,
      context_window: contextWindow,
      // 沿用模板的输出上限,但不能超过用户填的上下文窗口
      max_tokens:
        input.max_tokens ?? Math.min(template.maxTokens, contextWindow),
    });
    await this.applyCustomModels();
    return { provider_id: providerId, model: saved };
  }

  // 连通性自检:拿该 Provider 的一个模型发一条最小请求,验证凭据与网络确实能打通
  async testProvider(providerId: string, modelId?: string) {
    const provider = this.models.getProvider(providerId);
    if (!provider) throw new Error("provider not found");
    if (!(await this.isConfigured(providerId))) {
      throw new Error("provider is not configured");
    }
    const models = this.models.getModels(providerId);
    const model = modelId
      ? models.find((item) => item.id === modelId)
      : models[0];
    if (!model) throw new Error("provider has no model to test");
    const started = Date.now();
    await this.completeText(
      model,
      { messages: [{ role: "user", content: "ping", timestamp: started }] },
      { maxTokens: 16, timeoutMs: 30_000, errorMessage: "request failed" },
    );
    return {
      provider_id: providerId,
      model: model.id,
      latency_ms: Date.now() - started,
    };
  }

  /**
   * 给会话取一个短标题。用会话自己的 provider/model,失败抛错由调用方吞掉。
   * 输入只截首轮问答的开头——标题只需要话题,不需要全文。
   */
  async summarizeTitle(
    prompt: string,
    reply: string,
    selection: ModelSelection,
  ) {
    const model = this.model(selection);
    if (!(await this.isConfigured(selection.provider))) {
      throw new Error("provider is not configured");
    }
    const title = await this.completeText(
      model,
      {
        systemPrompt:
          "You name chat conversations. Reply with a single short title of at most 8 words (or 20 Chinese characters) that names the topic. Use the same language as the user. No quotes, no punctuation at the end, no preamble.",
        messages: [
          {
            role: "user",
            content: `User asked:\n${prompt.slice(0, 1_000)}\n\nAssistant replied:\n${reply.slice(0, 1_000)}\n\nTitle:`,
            timestamp: Date.now(),
          },
        ],
      },
      {
        maxTokens: 64,
        timeoutMs: 20_000,
        errorMessage: "title request failed",
      },
    );
    return title.trim();
  }

  async organize(input: OrganizerInput, selection?: ModelSelection) {
    const selected = selection ?? (await this.defaultModel());
    const model = this.model(selected);
    if (!(await this.isConfigured(selected.provider))) {
      throw new Error("provider is not configured");
    }
    const text = await this.completeText(
      model,
      {
        systemPrompt:
          "You return strictly validated JSON for Ferry's local session organizer.",
        messages: [
          {
            role: "user",
            content: organizerPrompt(input),
            timestamp: Date.now(),
          },
        ],
      },
      {
        maxTokens: 8_000,
        timeoutMs: 120_000,
        errorMessage: "organizer request failed",
      },
    );
    return validateOrganizerResult(text, input);
  }

  async deleteCustomModel(providerId: string, modelId: string) {
    if (!this.baseProviders.has(providerId)) {
      throw new Error("provider not found");
    }
    const result = await this.store.deleteCustomModel(providerId, modelId);
    await this.applyCustomModels();
    return result;
  }

  async providers(): Promise<ProviderSummary[]> {
    const config = await this.store.snapshot();
    const credentials = new Map(
      Object.entries(config.credentials).map(([id, value]) => [id, value.type]),
    );
    const enabled = new Set(config.enabled_providers);
    const customConfigs = new Map(
      config.custom_providers.map((item) => [item.id, item]),
    );
    const output: ProviderSummary[] = [];
    for (const provider of this.models.getProviders()) {
      const credentialType = credentials.get(provider.id) ?? null;
      const authTypes: AuthType[] = [];
      if (provider.auth.apiKey) authTypes.push("api_key");
      if (provider.auth.oauth) authTypes.push("oauth");
      const models = provider.getModels();
      const visible = config.visible_models[provider.id];
      const customConfig = customConfigs.get(provider.id);
      output.push({
        ...(customConfig
          ? {
              base_url: customConfig.base_url,
              api: customConfig.api ?? "openai-completions",
            }
          : {}),
        id: provider.id,
        name: provider.name,
        configured: this.customIds.has(provider.id) || credentialType !== null,
        credential_type: credentialType,
        auth_types: authTypes,
        custom: this.customIds.has(provider.id),
        enabled: enabled.has(provider.id),
        model_count: models.length,
        visible_model_count: visible
          ? models.filter((model) => visible.includes(model.id)).length
          : models.length,
      });
    }
    return output.sort((left, right) => left.name.localeCompare(right.name));
  }

  async setProviderEnabled(providerId: string, enabled: boolean) {
    if (!this.models.getProvider(providerId)) {
      throw new Error("provider not found");
    }
    if (!enabled && this.customIds.has(providerId)) {
      throw new Error("custom providers are removed instead of disabled");
    }
    await this.store.setProviderEnabled(providerId, enabled);
    return { provider_id: providerId, enabled };
  }

  async setVisibleModels(providerId: string, modelIds: string[] | null) {
    const provider = this.models.getProvider(providerId);
    if (!provider) throw new Error("provider not found");
    if (modelIds) {
      const known = new Set(provider.getModels().map((model) => model.id));
      const unknown = modelIds.find((id) => !known.has(id));
      if (unknown) throw new Error(`model is not available: ${unknown}`);
    }
    await this.store.setVisibleModels(providerId, modelIds);
    return { provider_id: providerId, model_ids: modelIds };
  }

  // 模型选择器的数据源:已启用 + 已配置凭据的 Provider,按用户勾选的可见模型展开
  async enabledModels(): Promise<
    Array<ModelSummary & { provider_name: string }>
  > {
    return (await this.catalogModels())
      .filter((model) => model.shown)
      .map(({ shown, custom, ...model }) => model);
  }

  // 模型设置页的数据源:已添加且已配置凭据的 Provider 的全部模型,附带是否出现在选择器里
  async catalogModels(): Promise<
    Array<
      ModelSummary & { provider_name: string; shown: boolean; custom: boolean }
    >
  > {
    const config = await this.store.snapshot();
    const output: Array<
      ModelSummary & { provider_name: string; shown: boolean; custom: boolean }
    > = [];
    for (const providerId of config.enabled_providers) {
      const provider = this.models.getProvider(providerId);
      if (!provider || !(await this.isConfigured(providerId))) continue;
      const visible = config.visible_models[providerId];
      const custom = new Set(
        (config.custom_models[providerId] ?? []).map((item) => item.id),
      );
      for (const model of this.models.getModels(providerId)) {
        output.push({
          ...modelSummary(model),
          provider_name: provider.name,
          shown: !visible || visible.includes(model.id),
          custom: custom.has(model.id),
        });
      }
    }
    return output;
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
      .map(modelSummary);
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

  // 回显已保存的 API Key(设置页「眼睛」按钮):凭据库优先,自定义配置里的兜底
  async revealApiKey(providerId: string) {
    if (!this.models.getProvider(providerId)) {
      throw new Error("provider not found");
    }
    const stored = await this.store.read(providerId);
    if (stored?.type === "api_key") {
      return { provider_id: providerId, key: stored.key };
    }
    const custom = (await this.store.snapshot()).custom_providers.find(
      (item) => item.id === providerId,
    );
    return { provider_id: providerId, key: custom?.api_key ?? null };
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
    return this.models.login(providerId, type, interaction);
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

  async saveCustomProvider(config: CustomProviderConfig, clearApiKey = false) {
    if (UNSUPPORTED_PROVIDER_IDS.has(config.id)) {
      throw new Error("provider id is reserved for an unsupported provider");
    }
    if (!this.customIds.has(config.id) && this.models.getProvider(config.id)) {
      throw new Error("custom provider id conflicts with a built-in provider");
    }
    await this.store.saveCustomProvider(config, clearApiKey);
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
