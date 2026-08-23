// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
export const FEATURE_STAGES = ["experimental", "preferences"] as const;
export type FeatureStage = (typeof FEATURE_STAGES)[number];
export const FEATURES = [
  { id: "builtin-agent", stage: "experimental", default: false },
  { id: "handoff", stage: "preferences", default: true },
] as const;
export type FeatureId = (typeof FEATURES)[number]["id"];
export const isFeatureId = (id: unknown): id is FeatureId =>
  FEATURES.some(feature => feature.id === id);
