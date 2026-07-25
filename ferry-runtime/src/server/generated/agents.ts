// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
export const AGENT_IDS = ["claude", "codex", "opencode"] as const;
export const AGENT_LABELS = ["Claude Code", "Codex CLI", "OpenCode"] as const;
export const AGENT_EDIT_OPERATIONS = {
  claude: ["delete-turn", "rewrite", "replace-assistant-reply"],
  codex: ["delete-turn", "rewrite", "replace-assistant-reply"],
  opencode: ["rewrite"],
} as const;
export const AGENT_SKILL_PATHS = {
  "claude": [
    "~/.claude/skills"
  ],
  "codex": [
    "~/.codex/skills"
  ],
  "opencode": [
    "~/.config/opencode/skills"
  ]
} as const;
export const SHARED_SKILL_PATHS = ["~/.agents/skills"] as const;
export type AgentId = (typeof AGENT_IDS)[number];
