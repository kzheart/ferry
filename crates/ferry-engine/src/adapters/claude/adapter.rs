//! Claude 当前原生结构的静态 Adapter 装配。

use std::path::Path;
use std::sync::Arc;

use crate::adapters::contracts::{
    filesystem_reference, AgentAdapter, AgentManifest, Fingerprint, NativeSessionReference,
    ScanCache, ScanRow, SessionBrowser, StorageKind,
};
use crate::adapters::shared::dialect::register_dialect;
use crate::adapters::shared::migration::TreeMigrationSource;
use crate::contracts::agents::agent;
use crate::errors::{DomainError, DomainResult};
use crate::loss::{self, Outcome};
use crate::model::Session;
use crate::system::paths::{is_within, realpath_strict};

use super::dialect::DIALECT;
use super::editing as claude_edit;
use super::editor::ClaudeBackend;
use super::lifecycle::ClaudeLifecycle;
use super::migration::ClaudeMigrationTarget;
use super::models::ClaudeModels;
use super::probe::ClaudeVerifier;
use super::reader;
use super::scanner;

/// claude 没有私有的损耗 code：reader 产生的 `session.malformed_record` /
/// `session.orphan_tool_result` / `session.subagent_unlinked` 与 writer 的
/// `migration.tool_degraded` 都是读取期告警，**刻意不声明后果**，因此不计入
/// 迁移差异。这里保留显式声明点，新增私有 code 时往表里加即可。
pub const LOSS_OUTCOMES: &[(&str, Outcome)] = &[];

/// Claude 扫描与读取实现。
pub struct ClaudeBrowser;

impl ClaudeBrowser {
    fn source_path() -> &'static str {
        "~/.claude/projects"
    }
}

impl SessionBrowser for ClaudeBrowser {
    fn scan(&self, cache: &dyn ScanCache) -> DomainResult<Vec<ScanRow>> {
        scanner::scan(cache)
    }

    fn read(&self, reference: &str) -> DomainResult<Session> {
        reader::read(reference)
    }

    fn read_agent(&self, reference: &str) -> DomainResult<Session> {
        reader::read(reference)
    }

    fn read_browser(&self, reference: &str) -> DomainResult<Session> {
        reader::read_preview(reference)
    }

    fn resolve_ref(&self, reference: &str) -> DomainResult<String> {
        Ok(claude_edit::resolve(reference)?
            .to_string_lossy()
            .into_owned())
    }

    fn fingerprint(&self, reference: &str) -> DomainResult<Fingerprint> {
        scanner::fingerprint(reference)
    }

    fn agent_fingerprint(&self, reference: &str) -> DomainResult<Fingerprint> {
        scanner::agent_fingerprint(reference)
    }

    fn canonicalize(&self, row: &ScanRow) -> Option<NativeSessionReference> {
        let resolve = |path: &Path| {
            claude_edit::resolve(&path.to_string_lossy())
                .ok()
                .map(|path| path.to_string_lossy().into_owned())
        };
        let reference =
            filesystem_reference(row, Self::source_path(), &resolve, StorageKind::File, None)?;
        let is_jsonl = Path::new(reference.canonical_ref())
            .extension()
            .is_some_and(|suffix| suffix == "jsonl");
        is_jsonl.then_some(reference)
    }

    fn validate_read_scope(&self, reference: &NativeSessionReference) -> DomainResult<()> {
        let invalid = DomainError::agent_reference_invalid;
        if reference.storage_kind() != StorageKind::File {
            return Err(invalid("Claude 会话必须使用路径引用"));
        }
        let Some(raw_root) = reference.root().filter(|root| !root.is_empty()) else {
            return Err(invalid("Claude 会话必须使用路径引用"));
        };
        let root = realpath_strict(Path::new(raw_root))
            .map_err(|_| invalid("Claude 会话读取范围包含失效文件"))?;
        let path = realpath_strict(Path::new(reference.canonical_ref()))
            .map_err(|_| invalid("Claude 会话读取范围包含失效文件"))?;
        let root_text = root.to_string_lossy().into_owned();
        if !path.is_file()
            || path.extension().is_none_or(|suffix| suffix != "jsonl")
            || !is_within(&path.to_string_lossy(), &root_text)
        {
            return Err(invalid("Claude 会话读取范围超出会话根目录"));
        }

        let child_root = path.with_extension("").join("subagents");
        if !child_root.exists() {
            return Ok(());
        }
        let resolved_child_root =
            realpath_strict(&child_root).map_err(|_| invalid("Claude 会话子树包含失效目录"))?;
        if !resolved_child_root.is_dir()
            || !is_within(&resolved_child_root.to_string_lossy(), &root_text)
        {
            return Err(invalid("Claude 会话子树超出 Agent 会话根目录"));
        }
        for entry in walkdir::WalkDir::new(&child_root)
            .into_iter()
            .filter_map(Result::ok)
        {
            let candidate = entry.path();
            if candidate.extension().is_none_or(|suffix| suffix != "jsonl") {
                continue;
            }
            let resolved =
                realpath_strict(candidate).map_err(|_| invalid("Claude 会话子树包含失效文件"))?;
            if !resolved.is_file() || !is_within(&resolved.to_string_lossy(), &root_text) {
                return Err(invalid("Claude 会话子树超出 Agent 会话根目录"));
            }
        }
        Ok(())
    }
}

/// 装配 claude adapter。
pub fn build() -> Result<AgentAdapter, String> {
    register_dialect("claude", &DIALECT);
    loss::declare(LOSS_OUTCOMES);

    let contract = agent("claude").ok_or_else(|| "claude 不在生成契约里".to_string())?;
    let manifest = AgentManifest::from_contract(contract);
    let executable = manifest
        .executables
        .first()
        .cloned()
        .ok_or_else(|| "claude manifest 缺少可执行文件白名单".to_string())?;
    let browser: Arc<dyn SessionBrowser> = Arc::new(ClaudeBrowser);
    AgentAdapter::builder()
        .browser(browser.clone())
        .migration_source(Arc::new(TreeMigrationSource::new(browser)))
        .migration_target(Arc::new(ClaudeMigrationTarget))
        .editor(Arc::new(ClaudeBackend))
        .verifier(Arc::new(ClaudeVerifier))
        .lifecycle(Arc::new(ClaudeLifecycle::new(executable)))
        .models(Arc::new(ClaudeModels))
        .build(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::claude::editing::testing::home_guard;
    use crate::adapters::contracts::Component;
    use crate::adapters::shared::dialect::get_dialect;
    use crate::tool_ops::CanonicalOp;
    use serde_json::Value;

    #[test]
    fn build_registers_the_dialect_and_every_declared_component() {
        let adapter = build().expect("claude 必须可装配");
        assert_eq!(adapter.id(), "claude");
        for component in [
            Component::Browser,
            Component::MigrationSource,
            Component::MigrationTarget,
            Component::Editor,
            Component::Verifier,
            Component::Lifecycle,
            Component::Models,
        ] {
            assert!(adapter.has_component(component), "{component:?}");
        }
        assert_eq!(
            adapter.manifest.edit_operations,
            ["delete-turn", "rewrite", "replace-assistant-reply"]
        );
        assert_eq!(
            get_dialect("claude").map(|dialect| dialect.op_for("Bash")),
            Some(Some(CanonicalOp::SHELL_EXEC))
        );
    }

    fn reference(path: &Path, root: &Path) -> NativeSessionReference {
        NativeSessionReference::new(
            path.to_string_lossy().into_owned(),
            Some(root.to_string_lossy().into_owned()),
            StorageKind::File,
        )
        .unwrap()
    }

    #[test]
    fn read_scope_accepts_a_session_with_its_subagents() {
        let temp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(temp.path()).unwrap();
        let session = root.join("s.jsonl");
        std::fs::write(&session, "{}\n").unwrap();
        let subagents = root.join("s/subagents");
        std::fs::create_dir_all(&subagents).unwrap();
        std::fs::write(subagents.join("agent-a.jsonl"), "{}\n").unwrap();
        assert!(ClaudeBrowser
            .validate_read_scope(&reference(&session, &root))
            .is_ok());
    }

    #[test]
    fn read_scope_rejects_non_jsonl_and_escaping_paths() {
        let temp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(temp.path()).unwrap();
        let other = root.join("other.txt");
        std::fs::write(&other, "x").unwrap();
        let error = ClaudeBrowser
            .validate_read_scope(&reference(&other, &root))
            .unwrap_err();
        assert_eq!(error.code, "agent.reference_invalid");
        assert_eq!(error.message(), "Claude 会话读取范围超出会话根目录");

        let outside = tempfile::tempdir().unwrap();
        let stray = std::fs::canonicalize(outside.path())
            .unwrap()
            .join("s.jsonl");
        std::fs::write(&stray, "{}\n").unwrap();
        assert_eq!(
            ClaudeBrowser
                .validate_read_scope(&reference(&stray, &root))
                .unwrap_err()
                .message(),
            "Claude 会话读取范围超出会话根目录"
        );

        // 引用不存在 -> 失效文件。
        assert_eq!(
            ClaudeBrowser
                .validate_read_scope(&reference(&root.join("gone.jsonl"), &root))
                .unwrap_err()
                .message(),
            "Claude 会话读取范围包含失效文件"
        );
    }

    #[cfg(unix)]
    #[test]
    fn read_scope_rejects_subtrees_that_symlink_out() {
        let temp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(temp.path()).unwrap();
        let outside = tempfile::tempdir().unwrap();
        let stray = std::fs::canonicalize(outside.path())
            .unwrap()
            .join("x.jsonl");
        std::fs::write(&stray, "{}\n").unwrap();

        let session = root.join("s.jsonl");
        std::fs::write(&session, "{}\n").unwrap();
        let subagents = root.join("s/subagents");
        std::fs::create_dir_all(&subagents).unwrap();
        std::os::unix::fs::symlink(&stray, subagents.join("agent-a.jsonl")).unwrap();
        assert_eq!(
            ClaudeBrowser
                .validate_read_scope(&reference(&session, &root))
                .unwrap_err()
                .message(),
            "Claude 会话子树超出 Agent 会话根目录"
        );
    }

    #[test]
    fn canonicalize_only_accepts_jsonl_files() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let projects = home.join(".claude/projects/slug");
        std::fs::create_dir_all(&projects).unwrap();
        let session = projects.join("sid.jsonl");
        std::fs::write(&session, "{}\n").unwrap();
        let plain = projects.join("notes.txt");
        std::fs::write(&plain, "x").unwrap();

        let _home = home_guard(home);
        crate::adapters::contracts::clear_resolved_root_cache();
        let row = |path: &Path| {
            let mut row = ScanRow::new();
            row.insert(
                "path".into(),
                Value::from(path.to_string_lossy().into_owned()),
            );
            row
        };
        let accepted = ClaudeBrowser.canonicalize(&row(&session));
        let rejected = ClaudeBrowser.canonicalize(&row(&plain));
        drop(_home);
        crate::adapters::contracts::clear_resolved_root_cache();
        assert!(accepted.is_some());
        assert!(rejected.is_none());
    }
}
