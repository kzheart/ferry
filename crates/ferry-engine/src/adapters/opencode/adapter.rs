//! OpenCode 当前原生结构的静态 Adapter 装配。
//!
//! 语义事实源：`engine/adapters/opencode/adapter.py`。
//!
//! OpenCode 是五个 adapter 里**唯一** `storage_kind == "id"` 的：会话不落文件，
//! 引用就是 SQLite 里的原生 id，因此 `canonicalize` 走 `id_reference`
//! （`root` 恒为 `None`）。

use std::sync::Arc;

use crate::adapters::contracts::{
    id_reference, AgentAdapter, AgentManifest, Fingerprint, NativeSessionReference, ScanCache,
    ScanRow, SessionBrowser, StorageKind,
};
use crate::adapters::shared::dialect::register_dialect;
use crate::adapters::shared::migration::TreeMigrationSource;
use crate::contracts::agents::agent;
use crate::errors::{DomainError, DomainResult};
use crate::loss::{declare, Outcome};
use crate::model::Session;
use serde_json::Value;

use super::dialect::DIALECT;
use super::editor::OpenCodeBackend;
use super::lifecycle::OpenCodeLifecycle;
use super::migration::OpenCodeMigrationTarget;
use super::models::OpenCodeModels;
use super::probe::OpenCodeVerifier;
use super::{reader, scanner};

/// OpenCode 私有的损耗语义（共享目录里没有的 code 在这里声明）。
const OPENCODE_LOSSES: &[(&str, Outcome)] = &[("migration.tool_degraded", Outcome::Degraded)];

/// OpenCode 的读侧组件。
pub struct OpenCodeBrowser;

impl SessionBrowser for OpenCodeBrowser {
    fn scan(&self, cache: &dyn ScanCache) -> DomainResult<Vec<ScanRow>> {
        scanner::scan(cache)
    }

    fn read(&self, reference: &str) -> DomainResult<Session> {
        reader::read(reference)
    }

    fn read_agent(&self, reference: &str) -> DomainResult<Session> {
        reader::read_preview(reference)
    }

    /// 引用即原生 id，没有可解析的路径。
    fn resolve_ref(&self, reference: &str) -> DomainResult<String> {
        Ok(reference.to_string())
    }

    fn fingerprint(&self, reference: &str) -> DomainResult<Fingerprint> {
        Ok(scanner::fingerprint(reference).map_or(Value::Null, Value::from))
    }

    fn agent_fingerprint(&self, reference: &str) -> DomainResult<Fingerprint> {
        self.fingerprint(reference)
    }

    /// 扫描路径容忍落后一轮的快照，库频繁写入时不把全量刷新拖住。
    fn scan_fingerprint(&self, reference: &str) -> Option<DomainResult<Fingerprint>> {
        Some(Ok(
            scanner::scan_fingerprint(reference).map_or(Value::Null, Value::from)
        ))
    }

    /// 扫描完成后再补重建，避免重建与扫描并行互相拖慢。
    fn post_scan_maintenance(&self) -> Option<DomainResult<()>> {
        scanner::ensure_fingerprint_index_fresh();
        Some(Ok(()))
    }

    /// 活索引轮询探针：整库变更只需 stat sqlite 的 db 与 -wal。
    fn watch_stamp(&self) -> Option<DomainResult<Value>> {
        Some(Ok(Value::Array(scanner::database_stamp())))
    }

    fn canonicalize(&self, row: &ScanRow) -> Option<NativeSessionReference> {
        id_reference(row)
    }

    fn validate_read_scope(&self, reference: &NativeSessionReference) -> DomainResult<()> {
        if reference.storage_kind() != StorageKind::Id || reference.root().is_some() {
            return Err(DomainError::agent_reference_invalid(
                "OpenCode 会话引用必须由原生 id 支持",
            ));
        }
        Ok(())
    }
}

/// 装配 opencode adapter。
pub fn build() -> Result<AgentAdapter, String> {
    // Rust 没有 import 副作用：方言与损耗目录都必须在装配入口显式登记。
    register_dialect("opencode", &DIALECT);
    declare(OPENCODE_LOSSES);

    let contract = agent("opencode").ok_or("opencode 不在生成契约里")?;
    let manifest = AgentManifest::from_contract(contract);
    let executable = manifest
        .executables
        .first()
        .cloned()
        .ok_or("opencode manifest 缺少可执行文件白名单")?;
    let browser: Arc<dyn SessionBrowser> = Arc::new(OpenCodeBrowser);
    AgentAdapter::builder()
        .browser(browser.clone())
        .migration_source(Arc::new(TreeMigrationSource::new(browser)))
        .migration_target(Arc::new(OpenCodeMigrationTarget))
        .editor(Arc::new(OpenCodeBackend::new()))
        .verifier(Arc::new(OpenCodeVerifier))
        .lifecycle(Arc::new(OpenCodeLifecycle::new(executable)))
        .models(Arc::new(OpenCodeModels))
        .build(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::contracts::Component;
    use crate::adapters::shared::dialect::get_dialect;
    use crate::loss::outcome_for_code;

    #[test]
    fn build_registers_the_dialect_and_the_loss_catalog() {
        let adapter = build().expect("opencode adapter 可装配");
        assert_eq!(adapter.id(), "opencode");
        assert!(get_dialect("opencode").is_some());
        assert_eq!(
            outcome_for_code("migration.tool_degraded"),
            Some(Outcome::Degraded)
        );
    }

    #[test]
    fn every_declared_capability_has_its_component() {
        let adapter = build().unwrap();
        for component in [
            Component::Browser,
            Component::MigrationSource,
            Component::MigrationTarget,
            Component::Editor,
            Component::Verifier,
            Component::Lifecycle,
            Component::Models,
        ] {
            assert!(adapter.has_component(component), "缺少组件 {component:?}");
        }
        // opencode 只支持 rewrite 一种编辑操作。
        assert_eq!(adapter.manifest.edit_operations, ["rewrite"]);
        assert!(adapter.require_editor().is_ok());
        assert!(adapter.require_lifecycle("delete").is_ok());
        assert!(adapter.require_verifier("prompt").is_ok());
    }

    #[test]
    fn references_must_be_native_ids_without_a_root() {
        let browser = OpenCodeBrowser;
        let id = NativeSessionReference::new("ses_1", None, StorageKind::Id).unwrap();
        assert!(browser.validate_read_scope(&id).is_ok());
        let file =
            NativeSessionReference::new("/root/a.jsonl", Some("/root".into()), StorageKind::File)
                .unwrap();
        let error = browser.validate_read_scope(&file).unwrap_err();
        assert_eq!(error.code, "agent.reference_invalid");
        assert_eq!(error.message(), "OpenCode 会话引用必须由原生 id 支持");

        let mut row = ScanRow::new();
        row.insert("id".into(), Value::from("ses_1"));
        let canonical = browser.canonicalize(&row).unwrap();
        assert_eq!(canonical.storage_kind(), StorageKind::Id);
        assert_eq!(canonical.root(), None);
        // resolve_ref 是恒等映射：opencode 没有文件路径。
        assert_eq!(browser.resolve_ref("ses_1").unwrap(), "ses_1");
    }

    #[test]
    fn all_four_optional_browser_hooks_are_implemented() {
        // 库路径必须先隔离：否则指纹钩子会去读开发机上真实的 opencode 库。
        let _guard = super::super::store::tests::exclusive();
        let root = tempfile::tempdir().unwrap();
        super::super::store::set_database_path_override(Some(root.path().join("absent.db")));
        let browser = OpenCodeBrowser;
        assert_eq!(
            browser.scan_fingerprint("nope").transpose().unwrap(),
            Some(Value::Null)
        );
        assert!(browser.post_scan_maintenance().is_some());
        let stamp = browser.watch_stamp().unwrap().unwrap();
        // 只 stat db 与 -wal 两条路径。
        assert_eq!(stamp.as_array().unwrap().len(), 2);
        super::super::store::set_database_path_override(None);
        // 目录型会话独有的钩子在 opencode 上不适用。
        assert!(browser.authoritative_members("nope").is_none());
    }
}
