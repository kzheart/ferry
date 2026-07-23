// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
export const AGENTS = Object.freeze({
  "claude": {
    "displayName": "Claude Code",
    "icon": "claude",
    "referenceKind": "path"
  },
  "codex": {
    "displayName": "Codex CLI",
    "icon": "codex",
    "referenceKind": "path"
  },
  "opencode": {
    "displayName": "OpenCode",
    "icon": "opencode",
    "referenceKind": "id"
  }
});
export const AGENT_IDS = Object.freeze(Object.keys(AGENTS));
export const ALLOWED_EXECUTABLES = Object.freeze(["claude", "codex", "opencode"]);
