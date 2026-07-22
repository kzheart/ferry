import {
  createModels,
  type Model,
  type StreamFunction,
} from "@earendil-works/pi-ai";
import { deepseekProvider } from "@earendil-works/pi-ai/providers/deepseek";

export const DEEPSEEK_PROVIDER_ID = "deepseek";
export const DEEPSEEK_MODEL_ID = "deepseek-v4-flash";
export const DEEPSEEK_API_KEY_ENV = "DEEPSEEK_API_KEY";

export interface DeepSeekBackend {
  model: Model<string>;
  streamFn: StreamFunction;
  provider: typeof DEEPSEEK_PROVIDER_ID;
  modelId: typeof DEEPSEEK_MODEL_ID;
  credentialAvailable(): boolean;
}

export function createDeepSeekBackend(): DeepSeekBackend {
  const models = createModels();
  models.setProvider(deepseekProvider());
  const model = models.getModel(DEEPSEEK_PROVIDER_ID, DEEPSEEK_MODEL_ID);
  if (!model) {
    throw new Error(
      `Pi model catalog does not contain ${DEEPSEEK_PROVIDER_ID}/${DEEPSEEK_MODEL_ID}`,
    );
  }
  return {
    model,
    streamFn: models.streamSimple.bind(models),
    provider: DEEPSEEK_PROVIDER_ID,
    modelId: DEEPSEEK_MODEL_ID,
    credentialAvailable: () =>
      Boolean(process.env[DEEPSEEK_API_KEY_ENV]?.trim()),
  };
}
