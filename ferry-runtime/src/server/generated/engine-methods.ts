// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
export const ORGANIZATION_ENGINE_METHODS = [
  "session_backbone",
  "session_summaries_set",
  "organization_digest_context",
  "organization_propose",
  "organization_proposals_list",
] as const;
export type OrganizationEngineMethod =
  (typeof ORGANIZATION_ENGINE_METHODS)[number];
