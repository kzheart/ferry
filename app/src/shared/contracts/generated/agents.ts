// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
export const AGENTS = {
  "claude": {
    "displayName": "Claude Code",
    "icon": "claude",
    "editOperations": [
      "delete-turn",
      "rewrite",
      "replace-assistant-reply"
    ]
  },
  "codex": {
    "displayName": "Codex CLI",
    "icon": "codex",
    "editOperations": [
      "delete-turn",
      "rewrite",
      "replace-assistant-reply"
    ]
  },
  "opencode": {
    "displayName": "OpenCode",
    "icon": "opencode",
    "editOperations": [
      "rewrite"
    ]
  }
} as const;
export const AGENT_IDS = Object.keys(AGENTS) as AgentId[];
export const ALLOWED_EXECUTABLES = ["claude", "codex", "opencode"] as const;
export const AGENT_SKILL_PATHS = {
  "claude": [
    "~/.claude/skills"
  ],
  "codex": [],
  "opencode": []
} as const;
export type AgentId = keyof typeof AGENTS;
