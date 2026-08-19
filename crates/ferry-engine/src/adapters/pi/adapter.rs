//! Pi 适配器组装。
//!
//! 语义事实源：`engine/adapters/pi/adapter.py`。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;

use crate::adapters::contracts::{
    filesystem_reference, AgentAdapter, AgentManifest, Fingerprint, NativeSessionReference,
    ScanCache, ScanRow, SessionBrowser, StorageKind,
};
use crate::adapters::shared::dialect::register_dialect;
use crate::adapters::shared::migration::TreeMigrationSource;
use crate::adapters::shared::scanner::iter_lines;
use crate::contracts::agents::agent;
use crate::errors::{DomainError, DomainResult};
use crate::model::Session;
use crate::system::paths::{expanduser, home_dir, pi_session_roots, process_environ};

use super::dialect::DIALECT;
use super::editor::PiBackend;
use super::lifecycle::PiLifecycle;
use super::migration::PiMigrationTarget;
use super::models::PiModels;
use super::probe::PiVerifier;
use super::reader;
use super::scanner;

/// 运行期的 pi 会话根（顺序即查找优先级）。
fn roots() -> Vec<PathBuf> {
    pi_session_roots(&process_environ(), &home_dir())
}

/// 把「文件路径或会话 id」解析成绝对文件路径。
///
/// id 形态要遍历所有根、读每个 `*.jsonl` 的首行 header 比对 `version == 3 &&
/// id == ref`，**命中必须唯一**；重名或零命中都按会话不存在处理。
pub fn resolve(reference: &str) -> DomainResult<PathBuf> {
    resolve_in(reference, &roots())
}

fn resolve_in(reference: &str, roots: &[PathBuf]) -> DomainResult<PathBuf> {
    let not_found = || DomainError::session_not_found("pi", reference);
    let resolved_roots: Vec<PathBuf> = roots
        .iter()
        .filter(|root| root.exists())
        .filter_map(|root| std::fs::canonicalize(root).ok())
        .collect();
    let path = expanduser(reference);
    if path.is_file() {
        let resolved = std::fs::canonicalize(&path).map_err(|_| not_found())?;
        if resolved_roots.iter().any(|root| resolved.starts_with(root)) {
            return Ok(resolved);
        }
        return Err(not_found());
    }
    let mut hits: Vec<PathBuf> = Vec::new();
    for root in &resolved_roots {
        let pattern = format!("{}/**/*.jsonl", root.to_string_lossy());
        let Ok(candidates) = glob::glob(&pattern) else {
            continue;
        };
        for candidate in candidates.filter_map(Result::ok) {
            let Some(header) = first_header(&candidate) else {
                continue;
            };
            if header.get("type").and_then(Value::as_str) == Some("session")
                && header.get("version").and_then(Value::as_i64) == Some(3)
                && header.get("id").and_then(Value::as_str) == Some(reference)
            {
                if let Ok(resolved) = std::fs::canonicalize(&candidate) {
                    hits.push(resolved);
                }
            }
        }
    }
    if hits.len() == 1 {
        return Ok(hits.remove(0));
    }
    Err(not_found())
}

fn first_header(path: &Path) -> Option<Value> {
    let line = iter_lines(path).ok()?.next()?.ok()?;
    serde_json::from_str::<Value>(&line).ok()
}

pub struct PiBrowser;

impl PiBrowser {
    fn resolved_string(&self, reference: &str) -> DomainResult<String> {
        Ok(resolve(reference)?.to_string_lossy().into_owned())
    }
}

impl SessionBrowser for PiBrowser {
    fn scan(&self, cache: &dyn ScanCache) -> DomainResult<Vec<ScanRow>> {
        scanner::scan(cache)
    }

    fn read(&self, reference: &str) -> DomainResult<Session> {
        reader::read(&self.resolved_string(reference)?)
    }

    fn read_agent(&self, reference: &str) -> DomainResult<Session> {
        self.read(reference)
    }

    fn resolve_ref(&self, reference: &str) -> DomainResult<String> {
        self.resolved_string(reference)
    }

    fn fingerprint(&self, reference: &str) -> DomainResult<Fingerprint> {
        scanner::fingerprint(&self.resolved_string(reference)?)
    }

    fn agent_fingerprint(&self, reference: &str) -> DomainResult<Fingerprint> {
        scanner::agent_fingerprint(&self.resolved_string(reference)?)
    }

    fn canonicalize(&self, row: &ScanRow) -> Option<NativeSessionReference> {
        let resolver = |path: &Path| {
            resolve(&path.to_string_lossy())
                .ok()
                .map(|resolved| resolved.to_string_lossy().into_owned())
        };
        for root in roots() {
            let reference = filesystem_reference(
                row,
                &root.to_string_lossy(),
                &resolver,
                StorageKind::File,
                None,
            );
            if let Some(reference) = reference {
                if Path::new(reference.canonical_ref()).extension() == Some("jsonl".as_ref()) {
                    return Some(reference);
                }
            }
        }
        None
    }

    fn validate_read_scope(&self, reference: &NativeSessionReference) -> DomainResult<()> {
        let scope = || DomainError::agent_reference_invalid("Pi 会话读取范围超出会话根目录");
        if reference.storage_kind() != StorageKind::File {
            return Err(DomainError::agent_reference_invalid(
                "Pi 会话必须使用文件引用",
            ));
        }
        let Some(root) = reference.root().filter(|value| !value.is_empty()) else {
            return Err(DomainError::agent_reference_invalid(
                "Pi 会话必须使用文件引用",
            ));
        };
        let path = std::fs::canonicalize(reference.canonical_ref()).map_err(|_| scope())?;
        let root = std::fs::canonicalize(root).map_err(|_| scope())?;
        if !path.is_file() || path.extension() != Some("jsonl".as_ref()) || !path.starts_with(&root)
        {
            return Err(scope());
        }
        Ok(())
    }
}

/// 装配 pi adapter。
pub fn build() -> Result<AgentAdapter, String> {
    let contract = agent("pi").ok_or_else(|| "pi 未在生成契约里".to_string())?;
    let manifest = AgentManifest::from_contract(contract);
    // pi 没有私有 loss code：reader 产出的 `session.malformed_record` /
    // `session.orphan_tool_result` 在 Python 侧同样未声明（不计入迁移差异），
    // `migration.unknown_block_dropped` / `session.unpaired_tool_use` 由共享目录
    // 声明，因此这里不需要 `loss::declare`。
    register_dialect("pi", &DIALECT);
    let executable = manifest
        .executables
        .first()
        .cloned()
        .ok_or_else(|| "pi manifest 缺少可执行文件白名单".to_string())?;
    let browser = Arc::new(PiBrowser);
    AgentAdapter::builder()
        .browser(browser.clone())
        .migration_source(Arc::new(TreeMigrationSource::new(browser)))
        .migration_target(Arc::new(PiMigrationTarget))
        .editor(Arc::new(PiBackend))
        .verifier(Arc::new(PiVerifier))
        .lifecycle(Arc::new(PiLifecycle::new(executable)))
        .models(Arc::new(PiModels))
        .build(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::contracts::Component;
    use crate::adapters::shared::dialect::get_dialect;
    use crate::jsonutil::FileStat;
    use serde_json::json;
    use std::collections::HashMap;
    use std::fs;
    use std::sync::Mutex;

    const FIXTURES: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/agent_formats/pi"
    );
    const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/golden");
    /// 黄金基线把所有物化 fixture 的 mtime 钉在 2026-07-25T00:00:00Z。
    const FIXED_MTIME: u64 = 1_784_937_600;
    const CASES: [&str; 3] = [
        "case-01-plain",
        "case-02-tools",
        "case-03-branch-compaction",
    ];

    /// 改动进程环境变量的测试必须串行（`PI_CODING_AGENT_SESSION_DIR` 是全局的），
    /// 且必须与其他模块共用 crate 级的那把锁。
    use crate::system::paths::testing::EnvGuard;

    #[derive(Default)]
    struct MemoryCache {
        entries: Mutex<HashMap<PathBuf, Option<ScanRow>>>,
    }

    impl ScanCache for MemoryCache {
        fn get(&self, path: &Path, _stat: &FileStat) -> Option<Option<ScanRow>> {
            self.entries.lock().unwrap().get(path).cloned()
        }
        fn put(&self, path: &Path, _stat: &FileStat, meta: Option<ScanRow>) {
            self.entries
                .lock()
                .unwrap()
                .insert(path.to_path_buf(), meta);
        }
        fn get_digest(&self, _path: &Path, _stat: &FileStat) -> Option<String> {
            None
        }
        fn put_digest(&self, _path: &Path, _stat: &FileStat, _digest: &str) {}
        fn flush(&self) {}
    }

    /// 按 `scripts/dump-canonical-fixtures.py` 的 `prepare_pi` 物化一个 case：
    /// `<root>/<case>/<v3 头部 id>.jsonl`，mtime 钉死。
    fn materialize(root: &Path, case: &str) -> PathBuf {
        let source = PathBuf::from(FIXTURES).join(case).join("session.jsonl");
        let header = first_header(&source).expect("fixture 必须有 v3 头部");
        // fixture 没有 manifest，会话 id 只能从 v3 头部记录里取。
        let stem = header
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or(case)
            .to_string();
        let target = root.join(case).join(format!("{stem}.jsonl"));
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::copy(&source, &target).unwrap();
        let handle = fs::File::options().write(true).open(&target).unwrap();
        let pinned = std::time::UNIX_EPOCH + std::time::Duration::from_secs(FIXED_MTIME);
        handle
            .set_times(fs::FileTimes::new().set_modified(pinned))
            .unwrap();
        target
    }

    fn golden(kind: &str, case: &str) -> Value {
        let path = PathBuf::from(GOLDEN)
            .join(kind)
            .join("pi")
            .join(format!("{case}.json"));
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap()
    }

    #[test]
    fn reader_matches_the_canonical_golden_baseline() {
        for case in CASES {
            let root = tempfile::tempdir().unwrap();
            let path = materialize(root.path(), case);
            let session = reader::read(&path.to_string_lossy()).unwrap();
            let actual = serde_json::to_value(&session).unwrap();
            assert_eq!(actual, golden("canonical", case), "case={case}");
        }
    }

    #[test]
    fn scanner_matches_the_scan_golden_baseline() {
        for case in CASES {
            let root = tempfile::tempdir().unwrap();
            let path = materialize(root.path(), case);
            let rows =
                scanner::scan_roots(&MemoryCache::default(), &[root.path().to_path_buf()]).unwrap();
            let expected = golden("scan", case);
            let expected_rows = expected["rows"].as_array().unwrap();
            assert_eq!(rows.len(), expected_rows.len(), "case={case}");
            for (actual, expected) in rows.iter().zip(expected_rows) {
                // `_normalized.environment_dependent_fields` 里的 path 由沙箱决定，
                // 单独核对后缀；updated/size 因为钉了 mtime、按字节拷贝，可逐字段比。
                let mut trimmed = actual.clone();
                let actual_path = trimmed.remove("path").unwrap();
                let mut wanted = expected.as_object().unwrap().clone();
                wanted.remove("path");
                assert_eq!(Value::Object(trimmed), Value::Object(wanted), "case={case}");
                assert!(actual_path.as_str().unwrap().ends_with(&format!(
                    "{case}/{}",
                    path.file_name().unwrap().to_string_lossy()
                )));
            }
        }
    }

    #[test]
    fn scan_and_resolve_follow_the_session_dir_environment_variable() {
        let root = tempfile::tempdir().unwrap();
        for case in CASES {
            materialize(root.path(), case);
        }
        let _guard = EnvGuard::acquire().set("PI_CODING_AGENT_SESSION_DIR", root.path());
        let rows = PiBrowser.scan(&MemoryCache::default()).unwrap();
        let mut ids: Vec<&str> = rows.iter().map(|row| row["id"].as_str().unwrap()).collect();
        ids.sort_unstable();
        assert_eq!(
            ids,
            ["fixture-pi-branch", "fixture-pi-plain", "fixture-pi-tools"]
        );

        // 会话 id → 唯一文件。
        let path = resolve("fixture-pi-tools").unwrap();
        assert!(path.ends_with("case-02-tools/fixture-pi-tools.jsonl"));
        // 绝对路径原样接受，且必须落在扫描根内。
        assert_eq!(resolve(&path.to_string_lossy()).unwrap(), path);
        assert_eq!(
            resolve("fixture-pi-missing").unwrap_err().code,
            "session.not_found"
        );
        // canonicalize 走 filesystem_reference 的五道门。
        let row = rows
            .iter()
            .find(|row| row["id"] == json!("fixture-pi-tools"))
            .unwrap();
        let reference = PiBrowser.canonicalize(row).expect("扫描行必须可规范化");
        assert_eq!(reference.storage_kind(), StorageKind::File);
        PiBrowser.validate_read_scope(&reference).unwrap();
        // 扫描根就位后，resume 用的是解析出来的绝对文件路径。
        let args = crate::adapters::shared::lifecycle::BaseLifecycle::resume_args(
            &PiLifecycle::new("pi"),
            "fixture-pi-tools",
        )
        .unwrap();
        assert_eq!(
            args,
            ["--session".to_string(), path.to_string_lossy().into_owned()]
        );
        // 变量由 `_guard` 析构时统一恢复，这里不再手动改进程环境。
    }

    #[test]
    fn references_outside_the_scan_roots_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let stray = materialize(outside.path(), "case-01-plain");
        let _guard = EnvGuard::acquire().set("PI_CODING_AGENT_SESSION_DIR", root.path());
        let error = resolve(&stray.to_string_lossy()).unwrap_err();
        assert_eq!(error.code, "session.not_found");
    }

    #[test]
    fn duplicate_ids_across_roots_are_ambiguous() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        materialize(first.path(), "case-01-plain");
        materialize(second.path(), "case-01-plain");
        let error = resolve_in(
            "fixture-pi-plain",
            &[first.path().to_path_buf(), second.path().to_path_buf()],
        )
        .unwrap_err();
        assert_eq!(error.code, "session.not_found");
    }

    #[test]
    fn build_wires_every_declared_capability() {
        let adapter = build().unwrap();
        assert_eq!(adapter.id(), "pi");
        for component in [
            Component::Browser,
            Component::MigrationSource,
            Component::MigrationTarget,
            Component::Editor,
            Component::Verifier,
            Component::Lifecycle,
            Component::Models,
        ] {
            assert!(adapter.has_component(component), "缺组件 {component:?}");
        }
        assert!(adapter.require_browser().is_ok());
        assert!(adapter.require_editor().is_ok());
        assert!(adapter.require_migration_target().is_ok());
        assert!(adapter.require_verifier("probe").is_ok());
        assert!(adapter.require_lifecycle("delete").is_ok());
        assert_eq!(
            adapter.manifest.edit_operations,
            ["delete-turn", "rewrite", "replace-assistant-reply"]
        );
        // build() 必须把方言登记进静态注册表。
        assert_eq!(
            get_dialect("pi").map(|dialect| dialect.adapter()),
            Some("pi")
        );
    }
}
