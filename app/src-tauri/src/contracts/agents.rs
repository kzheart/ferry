// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
pub(crate) const AGENT_IDS: &[&str] = &["claude", "codex", "opencode", "pi", "grok"];
pub(crate) const AGENT_CAPABILITIES: &[(&str, &[&str])] = &[
    ("claude", &["browse", "resume", "migration-source", "migration-target", "edit", "delete", "probe", "models"]),
    ("codex", &["browse", "resume", "migration-source", "migration-target", "edit", "delete", "probe", "models"]),
    ("opencode", &["browse", "resume", "migration-source", "migration-target", "edit", "delete", "probe", "models"]),
    ("pi", &["browse", "resume", "migration-source", "migration-target", "edit", "delete", "probe", "models"]),
    ("grok", &["browse", "resume", "migration-source", "models"]),
];
pub(crate) const ALLOWED_EXECUTABLES: &[&str] = &["claude", "codex", "opencode", "pi", "grok"];
