// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
pub(crate) const AGENT_IDS: &[&str] = &["claude", "codex", "opencode", "pi", "grok", "cursor"];
pub(crate) const AGENT_CAPABILITIES: &[(&str, &[&str])] = &[
    (
        "claude",
        &[
            "browse",
            "resume",
            "migration-source",
            "migration-target",
            "edit",
            "delete",
            "probe",
            "prompt",
            "models",
        ],
    ),
    (
        "codex",
        &[
            "browse",
            "resume",
            "migration-source",
            "migration-target",
            "edit",
            "delete",
            "probe",
            "prompt",
            "models",
        ],
    ),
    (
        "opencode",
        &[
            "browse",
            "resume",
            "migration-source",
            "migration-target",
            "edit",
            "delete",
            "probe",
            "prompt",
            "models",
        ],
    ),
    (
        "pi",
        &[
            "browse",
            "resume",
            "migration-source",
            "migration-target",
            "edit",
            "delete",
            "probe",
            "prompt",
            "models",
        ],
    ),
    (
        "grok",
        &[
            "browse",
            "resume",
            "migration-source",
            "migration-target",
            "delete",
            "probe",
            "prompt",
            "models",
        ],
    ),
    (
        "cursor",
        &["browse", "resume", "migration-source", "migration-target"],
    ),
];
pub(crate) const ALLOWED_EXECUTABLES: &[&str] =
    &["claude", "codex", "opencode", "pi", "grok", "cursor"];
pub(crate) const AGENT_SKILL_TARGETS: &[(&str, &str, &str)] = &[
    ("claude", "Claude Code", "~/.claude/skills"),
    ("codex", "Codex CLI", "~/.codex/skills"),
    ("opencode", "OpenCode", "~/.config/opencode/skills"),
];
pub(crate) const SHARED_SKILL_PATHS: &[&str] = &["~/.agents/skills"];
