// 此文件由 scripts/generate-contracts.py 生成，请勿手改。

/// Agent 能力词表；manifest 里的 capabilities 必须是它的有序子集。
pub const AGENT_CAPABILITIES: &[&str] = &[
    "browse",
    "resume",
    "migration-source",
    "migration-target",
    "edit",
    "delete",
    "prompt",
    "models",
];

/// 单个内置 Agent 的静态契约（AgentManifest 的事实源）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgentContract {
    pub id: &'static str,
    pub display_name: &'static str,
    pub icon: &'static str,
    pub source_path: &'static str,
    pub capabilities: &'static [&'static str],
    pub edit_operations: &'static [&'static str],
    pub executables: &'static [&'static str],
    pub fallback_bin_dirs: &'static [&'static str],
}

pub const AGENTS: &[AgentContract] = &[
    AgentContract {
        id: "claude",
        display_name: "Claude Code",
        icon: "claude",
        source_path: "~/.claude/projects",
        capabilities: &[
            "browse",
            "resume",
            "migration-source",
            "migration-target",
            "edit",
            "delete",
            "prompt",
            "models",
        ],
        edit_operations: &["delete-turn", "rewrite", "replace-assistant-reply"],
        executables: &["claude"],
        fallback_bin_dirs: &[],
    },
    AgentContract {
        id: "codex",
        display_name: "Codex CLI",
        icon: "codex",
        source_path: "~/.codex/sessions",
        capabilities: &[
            "browse",
            "resume",
            "migration-source",
            "migration-target",
            "edit",
            "delete",
            "prompt",
            "models",
        ],
        edit_operations: &["delete-turn", "rewrite", "replace-assistant-reply"],
        executables: &["codex"],
        fallback_bin_dirs: &[],
    },
    AgentContract {
        id: "opencode",
        display_name: "OpenCode",
        icon: "opencode",
        source_path: "~/.local/share/opencode",
        capabilities: &[
            "browse",
            "resume",
            "migration-source",
            "migration-target",
            "edit",
            "delete",
            "prompt",
            "models",
        ],
        edit_operations: &["rewrite"],
        executables: &["opencode"],
        fallback_bin_dirs: &["~/.opencode/bin"],
    },
    AgentContract {
        id: "pi",
        display_name: "Pi Agent",
        icon: "pi",
        source_path: "~/.pi/agent/sessions",
        capabilities: &[
            "browse",
            "resume",
            "migration-source",
            "migration-target",
            "edit",
            "delete",
            "prompt",
            "models",
        ],
        edit_operations: &["delete-turn", "rewrite", "replace-assistant-reply"],
        executables: &["pi"],
        fallback_bin_dirs: &[],
    },
    AgentContract {
        id: "grok",
        display_name: "Grok Build",
        icon: "grok",
        source_path: "~/.grok/sessions",
        capabilities: &[
            "browse",
            "resume",
            "migration-source",
            "migration-target",
            "delete",
            "prompt",
            "models",
        ],
        edit_operations: &[],
        executables: &["grok"],
        fallback_bin_dirs: &["~/.local/bin"],
    },
    AgentContract {
        id: "cursor",
        display_name: "Cursor",
        icon: "cursor",
        source_path: "~/Library/Application Support/Cursor/User/globalStorage",
        capabilities: &["browse", "resume", "migration-source"],
        edit_operations: &[],
        executables: &["cursor"],
        fallback_bin_dirs: &[],
    },
];

pub const AGENT_IDS: &[&str] = &["claude", "codex", "opencode", "pi", "grok", "cursor"];
pub const ALLOWED_EXECUTABLES: &[&str] = &["claude", "codex", "opencode", "pi", "grok", "cursor"];

pub fn agent(id: &str) -> Option<&'static AgentContract> {
    AGENTS.iter().find(|agent| agent.id == id)
}
