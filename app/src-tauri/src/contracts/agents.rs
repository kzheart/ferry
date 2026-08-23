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
pub(crate) const SHARED_SKILL_PATHS: &[&str] = &["~/.agents/skills"];
