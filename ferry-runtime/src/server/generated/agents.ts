// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
export const AGENT_IDS = [
  "claude",
  "codex",
  "opencode",
  "pi",
  "grok",
  "cursor",
] as const;
export const AGENT_LABELS = [
  "Claude Code",
  "Codex CLI",
  "OpenCode",
  "Pi Agent",
  "Grok Build",
  "Cursor",
] as const;
export const AGENT_CAPABILITIES = {
  claude: [
    "browse",
    "resume",
    "migration-source",
    "migration-target",
    "edit",
    "delete",
    "prompt",
    "models",
  ],
  codex: [
    "browse",
    "resume",
    "migration-source",
    "migration-target",
    "edit",
    "delete",
    "prompt",
    "models",
  ],
  opencode: [
    "browse",
    "resume",
    "migration-source",
    "migration-target",
    "edit",
    "delete",
    "prompt",
    "models",
  ],
  pi: [
    "browse",
    "resume",
    "migration-source",
    "migration-target",
    "edit",
    "delete",
    "prompt",
    "models",
  ],
  grok: [
    "browse",
    "resume",
    "migration-source",
    "migration-target",
    "delete",
    "prompt",
    "models",
  ],
  cursor: ["browse", "resume", "migration-source", "migration-target"],
} as const;
export const AGENT_EDIT_OPERATIONS = {
  claude: ["delete-turn", "rewrite", "replace-assistant-reply"],
  codex: ["delete-turn", "rewrite", "replace-assistant-reply"],
  opencode: ["rewrite"],
  pi: ["delete-turn", "rewrite", "replace-assistant-reply"],
  grok: [],
  cursor: [],
} as const;
export const AGENT_SKILL_PATHS = {
  claude: ["~/.claude/skills"],
  codex: ["~/.codex/skills"],
  opencode: ["~/.config/opencode/skills"],
  pi: [],
  grok: [],
  cursor: [],
} as const;
export const SHARED_SKILL_PATHS = ["~/.agents/skills"] as const;
export type AgentId = (typeof AGENT_IDS)[number];
