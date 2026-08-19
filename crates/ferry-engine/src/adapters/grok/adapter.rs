//! Grok Build 当前格式的 adapter 组装。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
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
use crate::model::Session;
use crate::system::paths::{expanduser, is_within};

use super::dialect::DIALECT;
use super::lifecycle::GrokLifecycle;
use super::migration::GrokMigrationTarget;
use super::models::GrokModels;
use super::probe::GrokVerifier;
use super::reader::read as read_bundle;
use super::scanner::{agent_fingerprint, fingerprint, scan, sessions_root};
use super::store::{authoritative_history, read_text};

fn manifest() -> AgentManifest {
    AgentManifest::from_contract(agent("grok").expect("grok 必须在生成契约里"))
}

/// `~/.grok/sessions` 的规范化形态；目录不存在时退回展开后的路径
/// （Python 的 `Path.resolve()` 非严格，不要求目标存在）。
fn root_directory() -> PathBuf {
    let root = sessions_root();
    std::fs::canonicalize(&root).unwrap_or(root)
}

/// 遍历会话根下全部 `summary.json`。
fn summaries(root: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file() && entry.file_name() == "summary.json")
        .map(|entry| entry.into_path())
        .collect();
    found.sort();
    found
}

fn parse_summary(path: &Path) -> Option<Value> {
    read_text(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
}

/// 把引用解析成 bundle 目录：既接受目录路径，也接受原生 session id。
pub fn resolve(reference: &str) -> DomainResult<PathBuf> {
    let root = root_directory();
    let path = expanduser(reference);
    if path.is_dir() {
        let resolved = std::fs::canonicalize(&path)
            .map_err(|_| DomainError::session_not_found("grok", reference))?;
        if is_within(&resolved.to_string_lossy(), &root.to_string_lossy()) {
            return Ok(resolved);
        }
        return Err(DomainError::session_not_found("grok", reference));
    }
    let mut hits: Vec<PathBuf> = Vec::new();
    if root.exists() {
        for summary_path in summaries(&root) {
            let Some(summary) = parse_summary(&summary_path) else {
                continue;
            };
            let matches = summary
                .get("info")
                .and_then(|info| info.get("id"))
                .and_then(Value::as_str)
                == Some(reference);
            if matches {
                if let Some(parent) = summary_path.parent() {
                    if let Ok(resolved) = std::fs::canonicalize(parent) {
                        hits.push(resolved);
                    }
                }
            }
        }
    }
    match hits.len() {
        1 => Ok(hits.remove(0)),
        _ => Err(DomainError::session_not_found("grok", reference)),
    }
}

/// 全局 `parent_session_id → 子 bundle 目录` 索引。
fn child_index(root: &Path) -> HashMap<String, Vec<PathBuf>> {
    let mut children: HashMap<String, Vec<PathBuf>> = HashMap::new();
    if !root.exists() {
        return children;
    }
    let root_text = root.to_string_lossy().into_owned();
    for summary_path in summaries(root) {
        let Some(summary) = parse_summary(&summary_path) else {
            continue;
        };
        let Some(parent_id) = summary
            .get("parent_session_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let Some(child_path) = summary_path
            .parent()
            .and_then(|parent| std::fs::canonicalize(parent).ok())
        else {
            continue;
        };
        if !is_within(&child_path.to_string_lossy(), &root_text) {
            continue;
        }
        children
            .entry(parent_id.to_string())
            .or_default()
            .push(child_path);
    }
    children
}

/// 递归挂上子会话，并给整棵树打上同一个 `root_id`。
fn attach(
    node: &mut Session,
    children: &HashMap<String, Vec<PathBuf>>,
    root_session_id: &str,
    seen: &mut HashSet<String>,
) -> DomainResult<()> {
    node.root_id = Some(root_session_id.to_string());
    node.children = Vec::new();
    let Some(paths) = children.get(&node.source_id).cloned() else {
        return Ok(());
    };
    for child_path in paths {
        let mut child = read_bundle(&child_path)?;
        if !seen.insert(child.source_id.clone()) {
            continue;
        }
        attach(&mut child, children, root_session_id, seen)?;
        node.children.push(child);
    }
    Ok(())
}

pub struct GrokBrowser;

impl SessionBrowser for GrokBrowser {
    fn scan(&self, cache: &dyn ScanCache) -> DomainResult<Vec<ScanRow>> {
        scan(cache)
    }

    fn read(&self, reference: &str) -> DomainResult<Session> {
        let path = resolve(reference)?;
        let mut session = read_bundle(&path)?;
        let summary = parse_summary(&path.join("summary.json")).unwrap_or(Value::Null);
        let root_session_id = summary
            .get("root_session_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or(session.source_id.as_str())
            .to_string();
        let children = child_index(&root_directory());
        let mut seen = HashSet::from([session.source_id.clone()]);
        attach(&mut session, &children, &root_session_id, &mut seen)?;
        Ok(session)
    }

    fn read_agent(&self, reference: &str) -> DomainResult<Session> {
        self.read(reference)
    }

    fn resolve_ref(&self, reference: &str) -> DomainResult<String> {
        Ok(resolve(reference)?.to_string_lossy().into_owned())
    }

    fn fingerprint(&self, reference: &str) -> DomainResult<Fingerprint> {
        fingerprint(&resolve(reference)?.to_string_lossy())
    }

    fn agent_fingerprint(&self, reference: &str) -> DomainResult<Fingerprint> {
        agent_fingerprint(&resolve(reference)?.to_string_lossy())
    }

    /// bundle 的权威成员，**相对 bundle 目录**：索引层按 `(名字, 摘要)` 组装
    /// revision，绝对路径会让同一个会话在不同 HOME 下算出不同修订。
    fn authoritative_members(&self, reference: &str) -> Option<DomainResult<Vec<String>>> {
        Some(resolve(reference).map(|path| {
            let history = authoritative_history(&path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "chat_history.jsonl".to_string());
            vec!["summary.json".to_string(), history]
        }))
    }

    fn canonicalize(&self, row: &ScanRow) -> Option<NativeSessionReference> {
        let resolve_ref = |path: &Path| -> Option<String> {
            resolve(&path.to_string_lossy())
                .ok()
                .map(|path| path.to_string_lossy().into_owned())
        };
        filesystem_reference(
            row,
            "~/.grok/sessions",
            &resolve_ref,
            StorageKind::Directory,
            Some("summary.json"),
        )
    }

    fn validate_read_scope(&self, reference: &NativeSessionReference) -> DomainResult<()> {
        if reference.storage_kind() != StorageKind::Directory || reference.root().is_none() {
            return Err(DomainError::agent_reference_invalid(
                "Grok 会话必须使用目录引用",
            ));
        }
        let scope_error =
            || DomainError::agent_reference_invalid("Grok 会话读取范围超出会话根目录");
        let path = std::fs::canonicalize(reference.canonical_ref()).map_err(|_| scope_error())?;
        let root = std::fs::canonicalize(reference.root().unwrap_or_default())
            .map_err(|_| scope_error())?;
        if !path.is_dir()
            || !is_within(&path.to_string_lossy(), &root.to_string_lossy())
            || !path.join("summary.json").is_file()
        {
            return Err(scope_error());
        }
        Ok(())
    }
}

/// 装配 grok adapter。
pub fn build() -> Result<AgentAdapter, String> {
    // 方言注册是进程级的静态表；registry 装配时一次性登记。
    register_dialect("grok", &DIALECT);
    // grok 不产生私有损耗 code：它只用 shared 目录里的
    // `migration.unknown_block_dropped`，`session.malformed_record` 在 Python 侧
    // 同样未声明（不计入迁移差异），此处保持一致，不额外 declare。
    let manifest = manifest();
    let executable = manifest
        .executables
        .first()
        .cloned()
        .unwrap_or_else(|| "grok".to_string());
    let browser: Arc<GrokBrowser> = Arc::new(GrokBrowser);
    AgentAdapter::builder()
        .browser(browser.clone())
        .migration_source(Arc::new(TreeMigrationSource::new(browser)))
        .migration_target(Arc::new(GrokMigrationTarget))
        .verifier(Arc::new(GrokVerifier))
        .lifecycle(Arc::new(GrokLifecycle::new(executable)))
        .models(Arc::new(GrokModels))
        .build(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::contracts::Component;
    use serde_json::json;
    use std::fs;

    /// 环境变量是进程级的，改 `GROK_HOME` 的测试必须串行——而且要与 claude 的
    /// `HOME`、opencode 的 `FERRY_DATA_DIR` 共用**同一把** crate 级锁：
    /// `grok_home()` 在 `GROK_HOME` 缺失时回落 `HOME`，各自造锁挡不住交叉污染。
    use crate::system::paths::testing::EnvGuard;

    /// 在作用域内独占进程环境并把 `GROK_HOME` 指向沙箱。
    fn set_home(path: &Path) -> EnvGuard {
        EnvGuard::acquire().set("GROK_HOME", path)
    }

    fn bundle(home: &Path, id: &str, summary: Value) -> PathBuf {
        let path = home.join("sessions").join("project").join(id);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("summary.json"), summary.to_string()).unwrap();
        fs::write(
            path.join("updates.jsonl"),
            json!({"method": "session/update", "params": {
                "update": {"sessionUpdate": "agent_message_chunk",
                           "content": {"type": "text", "text": id}},
                "_meta": {"promptId": "p1"}}})
            .to_string()
                + "\n",
        )
        .unwrap();
        path
    }

    fn summary(id: &str, extra: Value) -> Value {
        let mut value = json!({
            "info": {"id": id, "cwd": "/w"}, "chat_format_version": 1,
            "generated_title": id, "current_model_id": "grok-code-fast-1",
        });
        for (key, item) in extra.as_object().unwrap() {
            value[key] = item.clone();
        }
        value
    }

    #[test]
    fn the_adapter_declares_exactly_the_contract_capabilities() {
        let adapter = build().expect("grok adapter 必须可装配");
        assert_eq!(adapter.id(), "grok");
        assert_eq!(
            adapter.manifest.capabilities,
            [
                "browse",
                "resume",
                "migration-source",
                "migration-target",
                "delete",
                "probe",
                "prompt",
                "models"
            ]
        );
        // 无 edit 能力 → 没有 editor 组件。
        assert!(adapter.manifest.edit_operations.is_empty());
        assert!(!adapter.has_component(Component::Editor));
        assert!(adapter.require_editor().is_err());
        for capability in ["browse", "migration-source", "migration-target", "models"] {
            assert!(adapter.supports(capability));
        }
        assert!(crate::adapters::shared::dialect::get_dialect("grok").is_some());
    }

    #[test]
    fn a_session_id_resolves_to_its_bundle_directory() {
        let home = tempfile::tempdir().unwrap();
        let _guard = set_home(home.path());
        let path = bundle(home.path(), "wanted", summary("wanted", json!({})));
        bundle(home.path(), "other", summary("other", json!({})));

        assert_eq!(resolve("wanted").unwrap(), fs::canonicalize(&path).unwrap());
        // 目录引用同样接受。
        assert_eq!(
            resolve(&path.to_string_lossy()).unwrap(),
            fs::canonicalize(&path).unwrap()
        );
        // 根之外的目录一律拒绝。
        let outside = tempfile::tempdir().unwrap();
        assert_eq!(
            resolve(&outside.path().to_string_lossy()).unwrap_err().code,
            "session.not_found"
        );
        assert_eq!(resolve("missing").unwrap_err().code, "session.not_found");
    }

    #[test]
    fn read_assembles_the_parent_child_tree() {
        let home = tempfile::tempdir().unwrap();
        let _guard = set_home(home.path());
        bundle(
            home.path(),
            "root",
            summary("root", json!({"root_session_id": "root"})),
        );
        bundle(
            home.path(),
            "kid",
            summary(
                "kid",
                json!({"root_session_id": "root", "parent_session_id": "root"}),
            ),
        );
        bundle(
            home.path(),
            "grandkid",
            summary(
                "grandkid",
                json!({"root_session_id": "root", "parent_session_id": "kid"}),
            ),
        );

        let session = GrokBrowser.read("root").unwrap();
        assert_eq!(session.root_id.as_deref(), Some("root"));
        assert_eq!(session.children.len(), 1);
        assert_eq!(session.children[0].source_id, "kid");
        assert_eq!(session.children[0].root_id.as_deref(), Some("root"));
        assert_eq!(session.children[0].children[0].source_id, "grandkid");
    }

    #[test]
    fn a_parent_cycle_cannot_make_read_recurse_forever() {
        let home = tempfile::tempdir().unwrap();
        let _guard = set_home(home.path());
        bundle(
            home.path(),
            "a",
            summary("a", json!({"parent_session_id": "b"})),
        );
        bundle(
            home.path(),
            "b",
            summary("b", json!({"parent_session_id": "a"})),
        );
        let session = GrokBrowser.read("a").unwrap();
        assert_eq!(session.children.len(), 1);
        assert!(session.children[0].children.is_empty());
    }

    #[test]
    fn authoritative_members_are_relative_names() {
        let home = tempfile::tempdir().unwrap();
        let _guard = set_home(home.path());
        let path = bundle(home.path(), "s", summary("s", json!({})));
        assert_eq!(
            GrokBrowser.authoritative_members("s").unwrap().unwrap(),
            ["summary.json", "updates.jsonl"]
        );
        fs::remove_file(path.join("updates.jsonl")).unwrap();
        fs::write(path.join("chat_history.jsonl"), "{}\n").unwrap();
        assert_eq!(
            GrokBrowser.authoritative_members("s").unwrap().unwrap(),
            ["summary.json", "chat_history.jsonl"]
        );
    }

    #[test]
    fn read_scope_requires_a_directory_reference_with_a_summary() {
        let home = tempfile::tempdir().unwrap();
        let _guard = set_home(home.path());
        let path = bundle(home.path(), "s", summary("s", json!({})));
        let root = fs::canonicalize(home.path().join("sessions")).unwrap();
        let reference = NativeSessionReference::new(
            fs::canonicalize(&path).unwrap().to_string_lossy(),
            Some(root.to_string_lossy().into_owned()),
            StorageKind::Directory,
        )
        .unwrap();
        assert!(GrokBrowser.validate_read_scope(&reference).is_ok());

        // id 型引用直接拒绝。
        let id_reference = NativeSessionReference::new("s", None, StorageKind::Id).unwrap();
        let error = GrokBrowser.validate_read_scope(&id_reference).unwrap_err();
        assert_eq!(error.code, "agent.reference_invalid");

        // 根外目录拒绝。
        let outside = tempfile::tempdir().unwrap();
        let escaped = NativeSessionReference::new(
            fs::canonicalize(outside.path()).unwrap().to_string_lossy(),
            Some(root.to_string_lossy().into_owned()),
            StorageKind::Directory,
        )
        .unwrap();
        assert!(GrokBrowser.validate_read_scope(&escaped).is_err());
    }

    #[test]
    fn fingerprints_track_bundle_content_and_summary_stat() {
        let home = tempfile::tempdir().unwrap();
        let _guard = set_home(home.path());
        let path = bundle(home.path(), "s", summary("s", json!({})));
        let before = GrokBrowser.fingerprint("s").unwrap();
        assert!(GrokBrowser
            .agent_fingerprint("s")
            .unwrap()
            .as_str()
            .unwrap()
            .starts_with("stat:"));
        fs::write(path.join("updates.jsonl"), "{\"method\": \"x\"}\n").unwrap();
        assert_ne!(GrokBrowser.fingerprint("s").unwrap(), before);
    }
}
