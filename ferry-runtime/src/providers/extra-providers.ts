import {
  createProvider,
  envApiKeyAuth,
  type Model,
  type Provider,
} from "@earendil-works/pi-ai";
import { anthropicMessagesApi } from "@earendil-works/pi-ai/api/anthropic-messages.lazy";

// Xiaomi Token Plan 每个区域端点同时提供 /v1(OpenAI)与 /anthropic(Anthropic)两种协议,
// pi-ai 内置的只有 OpenAI 端点,这里补一份 Anthropic 格式的变体
const XIAOMI_TOKEN_PLAN_REGIONS = [
  { region: "ams", label: "AMS" },
  { region: "cn", label: "CN" },
  { region: "sgp", label: "SGP" },
] as const;

const XIAOMI_MIMO_MODELS: Array<{
  id: string;
  name: string;
  input: Array<"text" | "image">;
}> = [
  { id: "mimo-v2.5-pro", name: "MiMo-V2.5-Pro", input: ["text"] },
  { id: "mimo-v2.5", name: "MiMo-V2.5", input: ["text", "image"] },
  { id: "mimo-v2-pro", name: "MiMo-V2-Pro", input: ["text"] },
];

function xiaomiTokenPlanAnthropicProvider(
  region: string,
  label: string,
): Provider {
  const id = `xiaomi-token-plan-${region}-anthropic`;
  const baseUrl = `https://token-plan-${region}.xiaomimimo.com/anthropic`;
  const models: Model<"anthropic-messages">[] = XIAOMI_MIMO_MODELS.map(
    (item) => ({
      id: item.id,
      name: item.name,
      api: "anthropic-messages",
      provider: id,
      baseUrl,
      reasoning: true,
      input: item.input,
      cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
      contextWindow: 1_048_576,
      maxTokens: 131_072,
    }),
  );
  return createProvider({
    id,
    name: `Xiaomi Token Plan ${label} (Anthropic)`,
    baseUrl,
    auth: {
      apiKey: envApiKeyAuth(`Xiaomi Token Plan ${label} API key`, [
        `XIAOMI_TOKEN_PLAN_${label}_API_KEY`,
      ]),
    },
    models,
    api: anthropicMessagesApi(),
  });
}

export function extraProviders(): Provider[] {
  return XIAOMI_TOKEN_PLAN_REGIONS.map(({ region, label }) =>
    xiaomiTokenPlanAnthropicProvider(region, label),
  );
}

// 内置的 xiaomi-token-plan-* 走 OpenAI 端点,与上面的 Anthropic 变体并存时在名字上标明协议
export const PROVIDER_RENAMES: ReadonlyMap<string, string> = new Map(
  XIAOMI_TOKEN_PLAN_REGIONS.map(({ region, label }) => [
    `xiaomi-token-plan-${region}`,
    `Xiaomi Token Plan ${label} (OpenAI)`,
  ]),
);
