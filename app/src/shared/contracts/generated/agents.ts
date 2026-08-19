// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
export const AGENT_CAPABILITIES = ["browse", "resume", "migration-source", "migration-target", "edit", "delete", "probe", "prompt", "models"] as const;
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
      "prompt",
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
      "prompt",
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
      "prompt",
      "models"
    ],
    "editOperations": [
      "rewrite"
    ]
  },
  "pi": {
    "displayName": "Pi Agent",
    "icon": "pi",
    "capabilities": [
      "browse",
      "resume",
      "migration-source",
      "migration-target",
      "edit",
      "delete",
      "probe",
      "prompt",
      "models"
    ],
    "editOperations": [
      "delete-turn",
      "rewrite",
      "replace-assistant-reply"
    ]
  },
  "grok": {
    "displayName": "Grok Build",
    "icon": "grok",
    "capabilities": [
      "browse",
      "resume",
      "migration-source",
      "migration-target",
      "delete",
      "probe",
      "prompt",
      "models"
    ],
    "editOperations": []
  },
  "cursor": {
    "displayName": "Cursor",
    "icon": "cursor",
    "capabilities": [
      "browse",
      "migration-source"
    ],
    "editOperations": []
  }
} as const;
export const AGENT_IDS = Object.keys(AGENTS) as AgentId[];
export const ALLOWED_EXECUTABLES = ["claude", "codex", "opencode", "pi", "grok", "cursor"] as const;
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
  "pi": [],
  "grok": [],
  "cursor": []
} as const;
export const SHARED_SKILL_PATHS = ["~/.agents/skills"] as const;
export type AgentId = keyof typeof AGENTS;
