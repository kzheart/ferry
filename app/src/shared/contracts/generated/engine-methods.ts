// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
export const PUBLIC_ENGINE_METHODS = [
  "health",
  "version",
  "scan",
  "env",
  "resume",
  "models",
  "history",
  "history_delete",
  "pricing",
  "show",
  "session_asset",
  "session_meta_list",
] as const;
export const TRUSTED_UI_ENGINE_METHODS = [
  "organization_proposals_list",
  "organization_proposal_modify",
  "organization_proposal_decide",
] as const;
export type PublicEngineMethod = (typeof PUBLIC_ENGINE_METHODS)[number];
export type TrustedUiEngineMethod =
  (typeof TRUSTED_UI_ENGINE_METHODS)[number];
