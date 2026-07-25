// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
export const AGENT_IDS = ["claude", "codex", "opencode", "pi"] as const;
export const AGENT_LABELS = ["Claude Code", "Codex CLI", "OpenCode", "Pi Agent"] as const;
export const AGENT_CAPABILITIES = {
  "claude": [
    "browse",
    "resume",
    "migration-source",
    "migration-target",
    "edit",
    "delete",
    "probe",
    "models"
  ],
  "codex": [
    "browse",
    "resume",
    "migration-source",
    "migration-target",
    "edit",
    "delete",
    "probe",
    "models"
  ],
  "opencode": [
    "browse",
    "resume",
    "migration-source",
    "migration-target",
    "edit",
    "delete",
    "probe",
    "models"
  ],
  "pi": [
    "browse",
    "resume",
    "migration-source",
    "migration-target",
    "edit",
    "delete",
    "probe",
    "models"
  ]
} as const;
export const AGENT_EDIT_OPERATIONS = {
  claude: ["delete-turn", "rewrite", "replace-assistant-reply"],
  codex: ["delete-turn", "rewrite", "replace-assistant-reply"],
  opencode: ["rewrite"],
  pi: ["delete-turn", "rewrite", "replace-assistant-reply"],
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
  ],
  "pi": []
} as const;
export const SHARED_SKILL_PATHS = ["~/.agents/skills"] as const;
export type AgentId = (typeof AGENT_IDS)[number];
