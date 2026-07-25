// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
export const AGENT_CAPABILITIES = ["browse", "resume", "migration-source", "migration-target", "edit", "delete", "probe", "models"] as const;
export type AgentCapability = (typeof AGENT_CAPABILITIES)[number];
export const AGENTS = {
  "claude": {
    "displayName": "Claude Code",
    "icon": "claude",
    "capabilities": [
      "browse",
      "resume",
      "migration-source",
      "migration-target",
      "edit",
      "delete",
      "probe",
      "models"
    ],
    "editOperations": [
      "delete-turn",
      "rewrite",
      "replace-assistant-reply"
    ]
  },
  "codex": {
    "displayName": "Codex CLI",
    "icon": "codex",
    "capabilities": [
      "browse",
      "resume",
      "migration-source",
      "migration-target",
      "edit",
      "delete",
      "probe",
      "models"
    ],
    "editOperations": [
      "delete-turn",
      "rewrite",
      "replace-assistant-reply"
    ]
  },
  "opencode": {
    "displayName": "OpenCode",
    "icon": "opencode",
    "capabilities": [
      "browse",
      "resume",
      "migration-source",
      "migration-target",
      "edit",
      "delete",
      "probe",
      "models"
    ],
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
  "codex": [
    "~/.codex/skills"
  ],
  "opencode": [
    "~/.config/opencode/skills"
  ]
} as const;
export const SHARED_SKILL_PATHS = ["~/.agents/skills"] as const;
export type AgentId = keyof typeof AGENTS;
