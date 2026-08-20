//! Cursor 当前原生结构的静态 Adapter 装配。
//!
//! Cursor 提供 `browse` + `migration-source` + `migration-target` + `resume`：
//! 浏览与迁出对它的存储严格只读；迁入是唯一的写路径，且只新增本次迁移生成的键
//! （见 `store::open_writable` 与 `writer`）。仍然没有 edit / delete / probe：
//! 就地编辑与永久删除会改写 Cursor 自己的记录，探针需要一个跑得起来的 CLI Agent。
//!
//! 它与 opencode 一样是 `storage_kind == "id"`——会话不落文件，引用就是
//! `state.vscdb` 里的原生 composerId，因此 `canonicalize` 走 `id_reference`
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
use crate::model::Session;
use serde_json::Value;

use super::dialect::DIALECT;
use super::lifecycle::CursorLifecycle;
use super::migration::CursorMigrationTarget;
use super::{reader, scanner};

/// Cursor 的读侧组件。
pub struct CursorBrowser;

impl SessionBrowser for CursorBrowser {
    fn scan(&self, cache: &dyn ScanCache) -> DomainResult<Vec<ScanRow>> {
        scanner::scan(cache)
    }

    fn read(&self, reference: &str) -> DomainResult<Session> {
        reader::read(reference)
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

    /// 活索引轮询探针：Cursor 运行时一直在写 `-wal`，只能从会话内容派生令牌。
    fn watch_stamp(&self) -> Option<DomainResult<Value>> {
        Some(Ok(scanner::watch_stamp()))
    }

    fn canonicalize(&self, row: &ScanRow) -> Option<NativeSessionReference> {
        id_reference(row)
    }

    fn validate_read_scope(&self, reference: &NativeSessionReference) -> DomainResult<()> {
        if reference.storage_kind() != StorageKind::Id || reference.root().is_some() {
            return Err(DomainError::agent_reference_invalid(
                "Cursor 会话引用必须由原生 id 支持",
            ));
        }
        Ok(())
    }
}

/// 装配 cursor adapter。
pub fn build() -> Result<AgentAdapter, String> {
    // Rust 没有 import 副作用：方言必须在装配入口显式登记。
    register_dialect("cursor", &DIALECT);

    let contract = agent("cursor").ok_or("cursor 不在生成契约里")?;
    let manifest = AgentManifest::from_contract(contract);
    let executable = manifest
        .executables
        .first()
        .cloned()
        .unwrap_or_else(|| "cursor".to_string());
    let browser: Arc<dyn SessionBrowser> = Arc::new(CursorBrowser);
    AgentAdapter::builder()
        .browser(browser.clone())
        .migration_source(Arc::new(TreeMigrationSource::new(browser)))
        .migration_target(Arc::new(CursorMigrationTarget))
        .lifecycle(Arc::new(CursorLifecycle::new(executable)))
        .build(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::contracts::Component;
    use crate::adapters::shared::dialect::get_dialect;

    #[test]
    fn build_registers_the_dialect_and_the_declared_components() {
        let adapter = build().expect("cursor adapter 可装配");
        assert_eq!(adapter.id(), "cursor");
        assert!(get_dialect("cursor").is_some());
        assert_eq!(
            adapter.manifest.capabilities,
            ["browse", "resume", "migration-source", "migration-target"]
        );
        assert!(adapter.manifest.edit_operations.is_empty());
        for component in [
            Component::Browser,
            Component::MigrationSource,
            Component::MigrationTarget,
            Component::Lifecycle,
        ] {
            assert!(adapter.has_component(component), "缺少组件 {component:?}");
        }
        // 就地编辑、永久删除与探针都会改写或拉起 Cursor 自己的东西，一律不装。
        for component in [Component::Editor, Component::Verifier, Component::Models] {
            assert!(!adapter.has_component(component), "多出组件 {component:?}");
        }
        assert!(adapter.require_editor().is_err());
        assert!(adapter.require_lifecycle("resume").is_ok());
        assert!(adapter.require_lifecycle("delete").is_err());
        assert!(adapter.require_migration_target().is_ok());
    }

    #[test]
    fn references_must_be_native_ids_without_a_root() {
        let browser = CursorBrowser;
        let id = NativeSessionReference::new("c-1", None, StorageKind::Id).unwrap();
        assert!(browser.validate_read_scope(&id).is_ok());
        let file =
            NativeSessionReference::new("/root/a.jsonl", Some("/root".into()), StorageKind::File)
                .unwrap();
        let error = browser.validate_read_scope(&file).unwrap_err();
        assert_eq!(error.code, "agent.reference_invalid");
        assert_eq!(error.message(), "Cursor 会话引用必须由原生 id 支持");

        let mut row = ScanRow::new();
        row.insert("id".into(), Value::from("c-1"));
        let canonical = browser.canonicalize(&row).unwrap();
        assert_eq!(canonical.storage_kind(), StorageKind::Id);
        assert_eq!(canonical.root(), None);
        // resolve_ref 是恒等映射：cursor 没有文件路径。
        assert_eq!(browser.resolve_ref("c-1").unwrap(), "c-1");
    }

    #[test]
    fn the_watch_stamp_is_content_derived_and_survives_a_missing_database() {
        // 库路径必须先隔离，否则会去读开发机上真实的 Cursor 库。
        let _guard = super::super::store::tests::exclusive();
        let root = tempfile::tempdir().unwrap();
        super::super::store::set_database_path_override(Some(root.path().join("absent.vscdb")));
        let browser = CursorBrowser;
        // 库缺失不是探测失败：给一个稳定令牌，而不是 Err。
        let stamp = browser.watch_stamp().unwrap().unwrap();
        assert_eq!(stamp.as_array().unwrap().len(), 3);
        assert_eq!(stamp[1], Value::Null);
        // 库缺失时指纹为 null，该行不入索引。
        assert_eq!(browser.fingerprint("nope").unwrap(), Value::Null);
        super::super::store::set_database_path_override(None);
        // 目录型会话独有的钩子在 cursor 上不适用。
        assert!(browser.authoritative_members("nope").is_none());
        assert!(browser.scan_fingerprint("nope").is_none());
        assert!(browser.post_scan_maintenance().is_none());
    }
}
