// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
export const AGENTS = {
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
} as const;
export const AGENT_IDS = Object.keys(AGENTS) as AgentId[];
export const ALLOWED_EXECUTABLES = ["claude", "codex", "opencode"] as const;
export type AgentId = keyof typeof AGENTS;
