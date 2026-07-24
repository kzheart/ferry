import type { CredentialInfo, ProviderEnv } from "@earendil-works/pi-ai";

export const PROVIDER_CONFIG_VERSION = 2 as const;

export const THINKING_LEVELS = [
  "off",
  "minimal",
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
] as const;
export type ThinkingLevel = (typeof THINKING_LEVELS)[number];

export const DEFAULT_ENABLED_PROVIDERS = ["anthropic", "openai", "deepseek"];

export interface ModelSelection {
  provider: string;
  model: string;
  thinking?: ThinkingLevel;
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

export type StoredCredential =
  | {
      type: "api_key";
      key?: string;
      fields?: ProviderEnv;
    }
  | (Record<string, unknown> & {
      type: "oauth";
      access: string;
      refresh: string;
      expires: number;
    });

export interface ProviderConfigDocument {
  schema_version: typeof PROVIDER_CONFIG_VERSION;
  default_model: ModelSelection;
  credentials: Record<string, StoredCredential>;
  custom_providers: CustomProviderConfig[];
  enabled_providers: string[];
  visible_models: Record<string, string[]>;
  custom_models: Record<string, CustomModelConfig[]>;
}

export interface PublicProviderConfig {
  schema_version: typeof PROVIDER_CONFIG_VERSION;
  default_model: ModelSelection;
  credentials: CredentialInfo[];
  custom_providers: Array<
    Omit<CustomProviderConfig, "api_key"> & { configured: boolean }
  >;
  enabled_providers: string[];
  visible_models: Record<string, string[]>;
  custom_models: Record<string, CustomModelConfig[]>;
}

export function createInitialProviderConfig(): ProviderConfigDocument {
  return {
    schema_version: PROVIDER_CONFIG_VERSION,
    default_model: { provider: "deepseek", model: "deepseek-v4-flash" },
    credentials: {},
    custom_providers: [],
    enabled_providers: [...DEFAULT_ENABLED_PROVIDERS],
    visible_models: {},
    custom_models: {},
  };
}
