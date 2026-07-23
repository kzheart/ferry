// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
export const AGENTS = Object.freeze({
  "claude": {
    "displayName": "Claude Code",
    "icon": "claude"
  },
  "codex": {
    "displayName": "Codex CLI",
    "icon": "codex"
  },
  "opencode": {
    "displayName": "OpenCode",
    "icon": "opencode"
  }
});
export const AGENT_IDS = Object.freeze(Object.keys(AGENTS));
export const ALLOWED_EXECUTABLES = Object.freeze(["claude", "codex", "opencode"]);
