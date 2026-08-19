//! Pi v3 JSONL 会话发现。
//!
//! 语义事实源：`engine/adapters/pi/scanner.py`。
//!
//! 与 claude/codex 不同，pi 的 scanner **不走** `shared::scanner::scan_jsonl`：
//! 它有多个扫描根（环境变量 / settings.json / 默认目录），需要跨根按 realpath
//! 去重，且返回的是**扁平行**（不做 `session_roots` 树装配——pi 没有父子会话，
//! 装配由上层索引统一负责）。

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::adapters::contracts::{ScanCache, ScanRow};
use crate::adapters::shared::dialect::python_str;
use crate::adapters::shared::scanner::{
    add_tokens, clip_text_default, empty_tokens, has_tokens, iso_ms, iter_lines,
    path_stat_fingerprint, report_scan_advance, report_scan_total, Tokens,
};
use crate::errors::{DomainError, DomainResult};
use crate::jsonutil::{hash_bytes, FileStat};
use crate::system::paths::{home_dir, pi_session_roots, process_environ};

use super::reader::is_v3_header;
use super::tool_calls::truthy;

/// user 消息标题的文本投影：字符串原样，列表只取 `type == "text"` 的段。
fn text_of(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
            .map(|part| match part.get("text") {
                Some(value) if truthy(value) => python_str(value),
                _ => String::new(),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// 解析单个文件的扫描行；返回空 map 表示「不是会话」。
fn meta(path: &Path, stat: &FileStat) -> ScanRow {
    let mut records: Vec<Value> = Vec::new();
    // 只容忍「文件最后一行写了一半」：坏行后面还有任何一行都按整个文件不可解析
    // 处理。逐行读时读到下一行才知道坏行不是末行，所以先记下再判。
    let mut broken = false;
    let Ok(lines) = iter_lines(path) else {
        return ScanRow::new();
    };
    for line in lines {
        let Ok(line) = line else {
            return ScanRow::new();
        };
        if broken {
            return ScanRow::new();
        }
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(&line) {
            Ok(value) => records.push(value),
            Err(_) => broken = true,
        }
    }
    if records.is_empty() {
        return ScanRow::new();
    }
    let header = records[0].clone();
    if !is_v3_header(&header) {
        return ScanRow::new();
    }
    let entries = &records[1..];

    // 活动分支投影：与 reader::active_branch / codec::active_indexes 同一算法。
    let valid: Vec<&Value> = entries
        .iter()
        .filter(|entry| {
            entry.get("id").and_then(Value::as_str).is_some() && entry.get("parentId").is_some()
        })
        .collect();
    let mut branch: Vec<&Value> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut current = valid.last().copied();
    while let Some(entry) = current {
        let id = entry.get("id").and_then(Value::as_str).unwrap_or_default();
        if seen.contains(id) {
            break;
        }
        branch.push(entry);
        seen.insert(id);
        current = entry
            .get("parentId")
            .and_then(Value::as_str)
            .and_then(|parent| {
                valid
                    .iter()
                    .find(|candidate| candidate.get("id").and_then(Value::as_str) == Some(parent))
                    .copied()
            });
    }
    branch.reverse();

    let mut title = String::new();
    let mut count = 0i64;
    let mut model = String::new();
    let mut tokens = empty_tokens();
    for record in &branch {
        if record.get("type").and_then(Value::as_str) == Some("session_info") {
            if let Some(name) = record.get("name").filter(|value| truthy(value)) {
                title = python_str(name);
            }
        }
        if record.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        let empty = Value::Object(Map::new());
        let message = match record.get("message") {
            Some(value) if truthy(value) => value,
            _ => &empty,
        };
        let role = message.get("role").and_then(Value::as_str);
        if matches!(
            role,
            Some("user") | Some("assistant") | Some("bashExecution")
        ) {
            count += 1;
        }
        if role == Some("user") && title.is_empty() {
            let candidate = text_of(message.get("content"));
            if !candidate.trim().is_empty() {
                title = clip_text_default(&candidate);
            }
        }
        if role == Some("assistant") {
            if let Some(value) = message.get("model").filter(|value| truthy(value)) {
                model = python_str(value);
            }
            let usage = message.get("usage").filter(|value| truthy(value));
            let read = |key: &str| -> Value {
                usage
                    .and_then(|usage| usage.get(key))
                    .filter(|value| truthy(value))
                    .cloned()
                    .unwrap_or(Value::from(0))
            };
            let mut bucket = Map::new();
            bucket.insert("input".into(), read("input"));
            bucket.insert("output".into(), read("output"));
            bucket.insert("cache_read".into(), read("cacheRead"));
            bucket.insert("cache_write".into(), read("cacheWrite"));
            add_tokens(&mut tokens, &Tokens::from_value(&Value::Object(bucket)));
        }
    }
    if count == 0 {
        return ScanRow::new();
    }

    let mut row = ScanRow::new();
    row.insert("tool".into(), Value::from("pi"));
    row.insert("id".into(), header["id"].clone());
    row.insert("title".into(), Value::from(title));
    row.insert("dir".into(), header["cwd"].clone());
    row.insert(
        "updated".into(),
        Value::from((stat.mtime_ns / 1_000_000) as i64),
    );
    row.insert(
        "created".into(),
        iso_ms(&header["timestamp"]).map_or(Value::Null, Value::from),
    );
    row.insert("count".into(), Value::from(count));
    row.insert("size".into(), Value::from(stat.size));
    row.insert("path".into(), Value::from(path.to_string_lossy().as_ref()));
    row.insert("parent_id".into(), Value::Null);
    row.insert("root_id".into(), header["id"].clone());
    row.insert(
        "tokens".into(),
        if has_tokens(&tokens) {
            tokens.to_value()
        } else {
            Value::Null
        },
    );
    row.insert("model".into(), Value::from(model));
    row
}

/// 扫描全部 pi 会话根。
pub fn scan(cache: &dyn ScanCache) -> DomainResult<Vec<ScanRow>> {
    scan_roots(cache, &pi_session_roots(&process_environ(), &home_dir()))
}

/// [`scan`] 的显式根版本；测试与黄金对照直接用它，避免改动进程环境变量。
pub fn scan_roots(cache: &dyn ScanCache, roots: &[PathBuf]) -> DomainResult<Vec<ScanRow>> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    for root in roots {
        if !root.exists() {
            continue;
        }
        let pattern = format!("{}/**/*.jsonl", root.to_string_lossy());
        if let Ok(paths) = glob::glob(&pattern) {
            candidates.extend(paths.filter_map(Result::ok));
        }
    }
    report_scan_total(candidates.len());

    let mut rows: Vec<ScanRow> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    for path in candidates {
        report_scan_advance(1);
        let Ok(resolved) = fs::canonicalize(&path) else {
            continue;
        };
        let Ok(metadata) = fs::metadata(&resolved) else {
            continue;
        };
        if seen.contains(&resolved) || !metadata.is_file() {
            continue;
        }
        seen.insert(resolved.clone());
        let stat = FileStat::from_metadata(&metadata);
        let cached = match cache.get(&resolved, &stat) {
            // 外层 Some = 命中缓存；内层 None = 已知不是会话。
            Some(hit) => hit.unwrap_or_default(),
            None => {
                let parsed = meta(&resolved, &stat);
                cache.put(
                    &resolved,
                    &stat,
                    if parsed.is_empty() {
                        None
                    } else {
                        Some(parsed.clone())
                    },
                );
                parsed
            }
        };
        if !cached.is_empty() {
            rows.push(cached);
        }
    }
    Ok(rows)
}

/// 会话内容指纹：整文件 sha256（编辑事务的 revision 用的是同一套）。
pub fn fingerprint(reference: &str) -> DomainResult<Value> {
    let path = fs::canonicalize(reference)
        .map_err(|error| DomainError::internal(format!("Pi 会话不可读: {error}")))?;
    let bytes = fs::read(&path)
        .map_err(|error| DomainError::internal(format!("Pi 会话不可读: {error}")))?;
    Ok(Value::from(hash_bytes(&bytes)))
}

/// Agent 检索阶段的 O(1) 指纹（stat 摘要）。
pub fn agent_fingerprint(reference: &str) -> DomainResult<Value> {
    path_stat_fingerprint(reference)
        .map(Value::from)
        .map_err(|error| DomainError::internal(format!("Pi 会话不可读: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jsonutil::FileStat;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Mutex;

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

    fn write(path: &Path, records: &[Value]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let body = records
            .iter()
            .map(|record| serde_json::to_string(record).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(path, body).unwrap();
    }

    fn header() -> Value {
        json!({"type": "session", "version": 3, "id": "valid",
               "timestamp": "2026-07-25T00:00:00Z", "cwd": "/raw/project"})
    }

    #[test]
    fn accepts_only_v3_and_aggregates_usage() {
        let root = tempfile::tempdir().unwrap();
        write(
            &root.path().join("bucket").join("valid.jsonl"),
            &[
                header(),
                json!({"type": "message", "id": "u", "parentId": null,
                       "timestamp": "2026-07-25T00:00:01Z",
                       "message": {"role": "user", "content": "sk-test-title",
                                   "timestamp": 1}}),
                json!({"type": "message", "id": "a", "parentId": "u",
                       "timestamp": "2026-07-25T00:00:02Z",
                       "message": {"role": "assistant", "content": [],
                                   "model": "pi-model",
                                   "usage": {"input": 10, "output": 4,
                                             "cacheRead": 3, "cacheWrite": 2},
                                   "timestamp": 2}}),
            ],
        );
        let mut old = header();
        old["version"] = json!(2);
        old["id"] = json!("old");
        write(&root.path().join("old.jsonl"), &[old]);

        let rows = scan_roots(&MemoryCache::default(), &[root.path().to_path_buf()]).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["id"], json!("valid"));
        assert_eq!(rows[0]["dir"], json!("/raw/project"));
        assert_eq!(rows[0]["title"], json!("sk-test-title"));
        assert_eq!(
            rows[0]["tokens"],
            json!({"input": 10, "output": 4, "cache_read": 3, "cache_write": 2})
        );
        assert_eq!(rows[0]["model"], json!("pi-model"));
        assert_eq!(rows[0]["count"], json!(2));
    }

    #[test]
    fn tolerates_a_malformed_final_line_only() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("session.jsonl");
        let head = serde_json::to_string(&header()).unwrap();
        let user = serde_json::to_string(&json!({
            "type": "message", "id": "u", "parentId": null,
            "message": {"role": "user", "content": "kept"}
        }))
        .unwrap();
        fs::write(&path, format!("{head}\n{user}\n{{broken")).unwrap();
        let rows = scan_roots(&MemoryCache::default(), &[root.path().to_path_buf()]).unwrap();
        assert_eq!(rows[0]["id"], json!("valid"));

        // 坏行在中间 -> 整个文件不可解析。
        fs::write(&path, format!("{head}\n{{broken\n{user}")).unwrap();
        let rows = scan_roots(&MemoryCache::default(), &[root.path().to_path_buf()]).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn only_the_active_branch_is_counted() {
        let root = tempfile::tempdir().unwrap();
        write(
            &root.path().join("branch.jsonl"),
            &[
                header(),
                json!({"type": "message", "id": "u", "parentId": null,
                       "message": {"role": "user", "content": "root"}}),
                json!({"type": "message", "id": "dead", "parentId": "u",
                       "message": {"role": "assistant", "content": [],
                                   "usage": {"input": 99}}}),
                json!({"type": "message", "id": "live", "parentId": "u",
                       "message": {"role": "assistant", "content": [],
                                   "usage": {"input": 1}}}),
            ],
        );
        let rows = scan_roots(&MemoryCache::default(), &[root.path().to_path_buf()]).unwrap();
        assert_eq!(rows[0]["count"], json!(2));
        // 死分支的 99 token 不计入。
        assert_eq!(rows[0]["tokens"]["input"], json!(1));
    }

    #[test]
    fn cached_non_sessions_are_not_reparsed() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("junk.jsonl");
        fs::write(&path, "{}").unwrap();
        let cache = MemoryCache::default();
        assert!(scan_roots(&cache, &[root.path().to_path_buf()])
            .unwrap()
            .is_empty());
        assert_eq!(cache.entries.lock().unwrap().len(), 1);
        assert!(scan_roots(&cache, &[root.path().to_path_buf()])
            .unwrap()
            .is_empty());
    }

    #[test]
    fn duplicate_roots_are_deduplicated_by_realpath() {
        let root = tempfile::tempdir().unwrap();
        write(
            &root.path().join("a.jsonl"),
            &[
                header(),
                json!({"type": "message", "id": "u", "parentId": null,
                       "message": {"role": "user", "content": "hi"}}),
            ],
        );
        let rows = scan_roots(
            &MemoryCache::default(),
            &[root.path().to_path_buf(), root.path().to_path_buf()],
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn fingerprints_use_the_two_documented_shapes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("a.jsonl");
        fs::write(&path, b"ferry").unwrap();
        let reference = path.to_string_lossy().into_owned();
        assert_eq!(
            fingerprint(&reference).unwrap(),
            Value::from(hash_bytes(b"ferry"))
        );
        assert!(agent_fingerprint(&reference)
            .unwrap()
            .as_str()
            .unwrap()
            .starts_with("stat:"));
    }
}
