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
            "prompt",
            "models",
        ],
    ),
    ("cursor", &["browse", "resume", "migration-source"]),
];
pub(crate) const ALLOWED_EXECUTABLES: &[&str] =
    &["claude", "codex", "opencode", "pi", "grok", "cursor"];
pub(crate) const AGENT_SKILL_PATHS: &[(&str, &[&str])] = &[
    ("claude", &["~/.claude/skills"]),
    ("codex", &["~/.codex/skills"]),
    ("opencode", &["~/.config/opencode/skills"]),
    ("pi", &[]),
    ("grok", &[]),
    ("cursor", &[]),
];
pub(crate) const SHARED_SKILL_PATHS: &[&str] = &["~/.agents/skills"];
