// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
export const UI_ENGINE_METHODS = [
  "health",
  "version",
  "scan",
  "scan_progress",
  "env",
  "resume",
  "models",
  "history",
  "pricing",
  "show",
  "session_asset",
  "session_meta_list",
  "session_search",
] as const;
export type UiEngineMethod = (typeof UI_ENGINE_METHODS)[number];
