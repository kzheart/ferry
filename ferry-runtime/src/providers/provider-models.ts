import {
  createProvider,
  type Api,
  type AuthType,
  type CredentialInfo,
  type Model,
  type Provider,
} from "@earendil-works/pi-ai";
import { openAICompletionsApi } from "@earendil-works/pi-ai/api/openai-completions.lazy";

import type {
  CustomModelConfig,
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

export function customProvider(config: CustomProviderConfig): Provider {
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
