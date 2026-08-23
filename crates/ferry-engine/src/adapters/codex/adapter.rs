//! `adapters::codex::adapter` 的组装入口。

use std::path::Path;
use std::sync::Arc;

use serde_json::Value;

use crate::adapters::contracts::{
    filesystem_reference, AgentAdapter, AgentManifest, Fingerprint, NativeSessionReference,
    ScanCache, ScanRow, SessionBrowser, StorageKind,
};
use crate::adapters::shared::dialect::register_dialect;
use crate::adapters::shared::migration::TreeMigrationSource;
use crate::contracts::agents::agent;
use crate::errors::{DomainError, DomainResult};
use crate::loss::declare as declare_loss;
use crate::model::Session;
use crate::system::paths::{is_within, realpath_strict};

use super::dialect::DIALECT;
use super::editor::{resolve, CodexBackend};
use super::lifecycle::CodexLifecycle;
use super::migration::CodexMigrationTarget;
use super::models::CodexModels;
use super::probe::CodexVerifier;
use super::topology::rollout_files;
use super::{reader, scanner, tool_calls};

/// Codex 扫描与读取实现，不复用跨 Agent 的函数适配器。
pub struct CodexBrowser {
    source_path: String,
}

impl SessionBrowser for CodexBrowser {
    fn scan(&self, cache: &dyn ScanCache) -> DomainResult<Vec<ScanRow>> {
        scanner::scan(cache)
    }

    fn read(&self, reference: &str) -> DomainResult<Session> {
        reader::read(reference, None)
    }

    fn read_agent(&self, reference: &str) -> DomainResult<Session> {
        self.read(reference)
    }

    fn resolve_ref(&self, reference: &str) -> DomainResult<String> {
        Ok(resolve(reference)?.to_string_lossy().into_owned())
    }

    fn fingerprint(&self, reference: &str) -> DomainResult<Fingerprint> {
        Ok(Value::from(scanner::fingerprint(reference)?))
    }

    fn agent_fingerprint(&self, reference: &str) -> DomainResult<Fingerprint> {
        Ok(Value::from(scanner::agent_fingerprint(reference)?))
    }

    fn canonicalize(&self, row: &ScanRow) -> Option<NativeSessionReference> {
        let resolver = |path: &Path| {
            resolve(&path.to_string_lossy())
                .ok()
                .map(|path| path.to_string_lossy().into_owned())
        };
        let reference =
            filesystem_reference(row, &self.source_path, &resolver, StorageKind::File, None)?;
        let is_jsonl = Path::new(reference.canonical_ref())
            .extension()
            .is_some_and(|extension| extension == "jsonl");
        is_jsonl.then_some(reference)
    }

    fn validate_read_scope(&self, reference: &NativeSessionReference) -> DomainResult<()> {
        if reference.storage_kind() != StorageKind::File || reference.root().is_none() {
            return Err(DomainError::agent_reference_invalid(
                "Codex 会话必须使用路径引用",
            ));
        }
        let root = realpath_strict(Path::new(reference.root().unwrap_or_default()))
            .map_err(|_| DomainError::agent_reference_invalid("Codex 会话读取范围包含失效文件"))?;
        let path = realpath_strict(Path::new(reference.canonical_ref()))
            .map_err(|_| DomainError::agent_reference_invalid("Codex 会话读取范围包含失效文件"))?;
        let root_text = root.to_string_lossy().into_owned();
        if !path.is_file()
            || path
                .extension()
                .is_none_or(|extension| extension != "jsonl")
            || !is_within(&path.to_string_lossy(), &root_text)
        {
            return Err(DomainError::agent_reference_invalid(
                "Codex 会话读取范围超出会话根目录",
            ));
        }
        for candidate in rollout_files(&root) {
            let resolved = realpath_strict(&candidate)
                .map_err(|_| DomainError::agent_reference_invalid("Codex 会话子树包含失效文件"))?;
            if !resolved.is_file() || !is_within(&resolved.to_string_lossy(), &root_text) {
                return Err(DomainError::agent_reference_invalid(
                    "Codex 会话子树超出 Agent 会话根目录",
                ));
            }
        }
        Ok(())
    }
}

/// 装配 codex adapter。
pub fn build() -> Result<AgentAdapter, String> {
    let contract = agent("codex").ok_or_else(|| "codex 不在生成契约里".to_string())?;
    let manifest = AgentManifest::from_contract(contract);
    // Rust 没有 import 副作用：方言与损耗目录都在这里显式登记。
    register_dialect("codex", &DIALECT);
    declare_loss(tool_calls::LOSS_OUTCOMES);

    let browser = Arc::new(CodexBrowser {
        source_path: manifest.source_path.clone(),
    });
    let executable = manifest
        .executables
        .first()
        .cloned()
        .unwrap_or_else(|| "codex".to_string());
    AgentAdapter::builder()
        .browser(browser.clone())
        .migration_source(Arc::new(TreeMigrationSource::new(browser)))
        .migration_target(Arc::new(CodexMigrationTarget))
        .editor(Arc::new(CodexBackend))
        .verifier(Arc::new(CodexVerifier))
        .lifecycle(Arc::new(CodexLifecycle::new(executable)))
        .models(Arc::new(CodexModels))
        .build(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::contracts::Component;
    use crate::adapters::shared::dialect::get_dialect;
    use crate::loss::{outcome_for_code, Outcome};

    #[test]
    fn the_adapter_declares_every_codex_capability() {
        let adapter = build().expect("codex adapter 必须可装配");
        assert_eq!(adapter.id(), "codex");
        for capability in [
            "browse",
            "resume",
            "migration-source",
            "migration-target",
            "edit",
            "delete",
            "prompt",
            "models",
        ] {
            assert!(adapter.supports(capability), "缺少能力 {capability}");
        }
        assert!(adapter.has_component(Component::Editor));
        assert_eq!(
            adapter.manifest.edit_operations,
            ["delete-turn", "rewrite", "replace-assistant-reply"]
        );
    }

    #[test]
    fn build_registers_the_dialect_and_loss_catalogue() {
        build().expect("codex adapter 必须可装配");
        assert_eq!(
            get_dialect("codex").map(|dialect| dialect.adapter()),
            Some("codex")
        );
        assert_eq!(
            outcome_for_code("migration.apply_patch_unparsed"),
            Some(Outcome::Degraded)
        );
    }

    #[test]
    fn read_scope_rejects_non_jsonl_and_out_of_root_paths() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("sessions");
        std::fs::create_dir_all(&root).unwrap();
        let session = root.join("rollout-a.jsonl");
        std::fs::write(&session, "{}\n").unwrap();
        let outside = temp.path().join("stray.jsonl");
        std::fs::write(&outside, "{}\n").unwrap();

        let browser = CodexBrowser {
            source_path: root.to_string_lossy().into_owned(),
        };
        let root_text = std::fs::canonicalize(&root)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let inside = NativeSessionReference::new(
            std::fs::canonicalize(&session)
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            Some(root_text.clone()),
            StorageKind::File,
        )
        .unwrap();
        assert!(browser.validate_read_scope(&inside).is_ok());

        let escaping = NativeSessionReference::new(
            std::fs::canonicalize(&outside)
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            Some(root_text),
            StorageKind::File,
        )
        .unwrap();
        let error = browser.validate_read_scope(&escaping).unwrap_err();
        assert_eq!(error.code, "agent.reference_invalid");
    }
}
