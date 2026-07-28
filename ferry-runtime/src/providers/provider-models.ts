import {
  createProvider,
  type Api,
  type AuthType,
  type CredentialInfo,
  type Model,
  type Provider,
} from "@earendil-works/pi-ai";
import { anthropicMessagesApi } from "@earendil-works/pi-ai/api/anthropic-messages.lazy";
import { openAICompletionsApi } from "@earendil-works/pi-ai/api/openai-completions.lazy";
import { builtinProviders } from "@earendil-works/pi-ai/providers/all";

import type {
  CustomModelConfig,
  CustomProviderApi,
  CustomProviderConfig,
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
  enabled: boolean;
  model_count: number;
  visible_model_count: number;
  // 仅自定义提供商携带,用于设置页就地编辑
  base_url?: string;
  api?: CustomProviderApi;
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

// 按模型 ID 匹配 pi-ai 内置目录,补全自定义端点模型的能力字段
let builtinModelIndex: Map<string, Model<Api>> | undefined;
function builtinModelById(id: string): Model<Api> | undefined {
  if (!builtinModelIndex) {
    builtinModelIndex = new Map();
    for (const provider of builtinProviders()) {
      for (const model of provider.getModels()) {
        const key = model.id.toLowerCase();
        if (!builtinModelIndex.has(key)) builtinModelIndex.set(key, model);
      }
    }
  }
  // OpenRouter 风格的 "vendor/model" ID 也尝试用斜杠后的部分匹配
  const key = id.toLowerCase();
  return (
    builtinModelIndex.get(key) ??
    builtinModelIndex.get(key.slice(key.lastIndexOf("/") + 1))
  );
}

interface ModelsDevEntry {
  reasoning: boolean;
  image: boolean;
  context?: number;
  output?: number;
  cost: {
    input: number;
    output: number;
    cacheRead: number;
    cacheWrite: number;
  };
}

// pi-ai 没收录的模型,退而查 models.dev 公共目录;整份目录按天缓存在内存里
let modelsDevCache:
  | { at: number; index: Map<string, ModelsDevEntry> }
  | undefined;
async function modelsDevById(
  id: string,
  signal?: AbortSignal,
): Promise<ModelsDevEntry | undefined> {
  if (!modelsDevCache || Date.now() - modelsDevCache.at > 24 * 60 * 60 * 1000) {
    const index = new Map<string, ModelsDevEntry>();
    try {
      const response = await fetch("https://models.dev/api.json", {
        signal: signal ?? null,
      });
      if (response.ok) {
        const payload = (await response.json()) as Record<
          string,
          { models?: Record<string, Record<string, unknown>> }
        >;
        for (const provider of Object.values(payload)) {
          for (const [modelId, raw] of Object.entries(provider.models ?? {})) {
            const key = modelId.toLowerCase();
            if (index.has(key)) continue;
            const limit = raw.limit as
              | { context?: number; output?: number }
              | undefined;
            const cost = raw.cost as
              | Record<string, number | undefined>
              | undefined;
            const modalities = raw.modalities as
              | { input?: string[] }
              | undefined;
            index.set(key, {
              reasoning: raw.reasoning === true,
              image: modalities?.input?.includes("image") === true,
              ...(typeof limit?.context === "number"
                ? { context: limit.context }
                : {}),
              ...(typeof limit?.output === "number"
                ? { output: limit.output }
                : {}),
              cost: {
                input: cost?.input ?? 0,
                output: cost?.output ?? 0,
                cacheRead: cost?.cache_read ?? 0,
                cacheWrite: cost?.cache_write ?? 0,
              },
            });
          }
        }
      }
    } catch {
      // 目录拉不到就退回模板默认值,不阻塞模型发现
    }
    modelsDevCache = { at: Date.now(), index };
  }
  const key = id.toLowerCase();
  return (
    modelsDevCache.index.get(key) ??
    modelsDevCache.index.get(key.slice(key.lastIndexOf("/") + 1))
  );
}

export function customProvider(config: CustomProviderConfig): Provider {
  // 旧配置没有 api 字段,一律按 OpenAI 兼容处理
  const api: CustomProviderApi = config.api ?? "openai-completions";
  const template = config.models[0];
  const models: Model<CustomProviderApi>[] = config.models.map((item) => ({
    id: item.id,
    name: item.name ?? item.id,
    api,
    provider: config.id,
    baseUrl: config.base_url,
    reasoning: item.reasoning,
    input: item.input,
    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    contextWindow: item.context_window,
    maxTokens: item.max_tokens,
  }));

  // 从端点的模型列表接口自动发现模型;能力字段依次查 pi-ai 内置目录、
  // models.dev,最后落到手填模板/保守默认值
  const fetchModels = async (context: {
    credential?: { type: string; key?: string };
    signal?: AbortSignal;
  }): Promise<Model<CustomProviderApi>[]> => {
    const key =
      (context.credential?.type === "api_key"
        ? context.credential.key
        : undefined) ?? config.api_key;
    const url =
      api === "anthropic-messages"
        ? `${config.base_url}/v1/models`
        : `${config.base_url}/models`;
    const headers: Record<string, string> =
      api === "anthropic-messages"
        ? {
            "anthropic-version": "2023-06-01",
            ...(key ? { "x-api-key": key } : {}),
          }
        : key
          ? { Authorization: `Bearer ${key}` }
          : {};
    const response = await fetch(url, {
      headers,
      signal: context.signal ?? null,
    });
    if (!response.ok) {
      throw new Error(`model list request failed: HTTP ${response.status}`);
    }
    const payload = (await response.json()) as {
      data?: Array<Record<string, unknown>>;
    };
    const rows = (Array.isArray(payload.data) ? payload.data : [])
      .filter(
        (row): row is Record<string, unknown> & { id: string } =>
          typeof row?.id === "string" && row.id.length > 0,
      )
      .slice(0, 500);
    const output: Model<CustomProviderApi>[] = [];
    for (const row of rows) {
      const listedName =
        typeof row.display_name === "string"
          ? row.display_name
          : typeof row.name === "string"
            ? row.name
            : undefined;
      const known = builtinModelById(row.id);
      const dev = known
        ? undefined
        : await modelsDevById(row.id, context.signal);
      output.push({
        id: row.id,
        name: listedName ?? known?.name ?? row.id,
        api,
        provider: config.id,
        baseUrl: config.base_url,
        reasoning:
          known?.reasoning ?? dev?.reasoning ?? template?.reasoning ?? false,
        input: known
          ? [...known.input]
          : dev
            ? dev.image
              ? ["text", "image"]
              : ["text"]
            : template
              ? [...template.input]
              : ["text"],
        cost: known
          ? { ...known.cost }
          : (dev?.cost ?? { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 }),
        contextWindow:
          known?.contextWindow ??
          dev?.context ??
          template?.context_window ??
          128_000,
        maxTokens:
          known?.maxTokens ?? dev?.output ?? template?.max_tokens ?? 8_192,
      });
    }
    return output;
  };
  return createProvider({
    id: config.id,
    name: config.name,
    baseUrl: config.base_url,
    auth: {
      apiKey: {
        name: `${config.name} API key`,
        // 凭据库里保存的 Key 优先,配置文件里的 api_key 兜底
        async resolve(input) {
          const stored =
            input?.credential?.type === "api_key"
              ? input.credential.key
              : undefined;
          const key = stored ?? config.api_key;
          return {
            auth: {
              ...(key ? { apiKey: key } : {}),
              baseUrl: config.base_url,
            },
            source: "Ferry provider config",
          };
        },
      },
    },
    models,
    fetchModels,
    api:
      api === "anthropic-messages"
        ? anthropicMessagesApi()
        : openAICompletionsApi(),
  });
}

function overlayModel(
  template: Model<Api>,
  config: CustomModelConfig,
): Model<Api> {
  return {
    ...template,
    id: config.id,
    name: config.name ?? config.id,
    reasoning: config.reasoning,
    input: config.input,
    contextWindow: config.context_window,
    maxTokens: config.max_tokens,
    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
  };
}

export function withCustomModels(
  provider: Provider,
  configs: CustomModelConfig[],
): Provider {
  const base = provider.getModels.bind(provider);
  return {
    ...provider,
    getModels: () => {
      const merged = [...base()];
      const template = merged[0];
      if (!template) return merged;
      for (const config of configs) {
        const model = overlayModel(template, config);
        const index = merged.findIndex((item) => item.id === model.id);
        if (index >= 0) merged[index] = model;
        else merged.push(model);
      }
      return merged;
    },
  };
}

export function modelSummary(model: Model<string>): ModelSummary {
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
