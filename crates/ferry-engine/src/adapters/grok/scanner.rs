//! 递归扫描 Grok 的 summary.json。
//!
//! grok 是目录型存储，扫描单元是 `~/.grok/sessions/**/summary.json`，缓存键与
//! stat 都取 summary.json 自己（bundle 的其余文件不参与扫描判定）。

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::adapters::contracts::{ScanCache, ScanRow};
use crate::adapters::shared::scanner::{
    has_tokens, iso_ms, report_scan_advance, report_scan_total, session_roots, stat_digest, Tokens,
};
use crate::errors::{DomainError, DomainResult};
use crate::jsonutil::FileStat;
use crate::system::paths::{grok_home, home_dir, process_environ};

use super::store::{fingerprint as bundle_fingerprint, read_text};

fn non_negative(value: Option<&Value>) -> i64 {
    value.and_then(Value::as_i64).unwrap_or(0).max(0)
}

fn get_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter()
        .try_fold(value, |current, key| current.get(*key))
}

fn extract_total_tokens(value: &Value) -> Option<i64> {
    [
        &["params", "_meta", "totalTokens"][..],
        &["params", "update", "_meta", "totalTokens"][..],
        &["params", "update", "totalTokens"][..],
        &["params", "totalTokens"][..],
        &["usage", "totalTokens"][..],
        &["totalTokens"][..],
    ]
    .into_iter()
    .find_map(|path| get_path(value, path).and_then(Value::as_i64))
}

/// 对齐 Tokscale 的 Grok legacy 口径：updates 里的 `totalTokens` 是累计计数，
/// 只接受单调增加的最高水位；signals.json 在 compaction 后保存被折叠的历史，
/// 用它补齐 updates 已不可见的部分。旧格式没有稳定 input/output 拆分，因此归入
/// input，至少保证总量与原始账本一致。
fn grok_tokens(path: &Path, summary: &Value) -> Tokens {
    let history = if path.join("updates.jsonl").is_file() {
        path.join("updates.jsonl")
    } else {
        path.join("chat_history.jsonl")
    };
    let mut maximum = 0i64;
    if let Ok(text) = read_text(&history) {
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            let Ok(record) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if let Some(total) = extract_total_tokens(&record).filter(|total| *total >= maximum) {
                maximum = total;
            }
        }
    }
    if let Ok(text) = read_text(&path.join("signals.json")) {
        if let Ok(signals) = serde_json::from_str::<Value>(&text) {
            let before = non_negative(signals.get("totalTokensBeforeCompaction"));
            let total = non_negative(signals.get("totalTokens"));
            let effective = match signals.get("contextTokensUsed") {
                None => before.saturating_add(total),
                Some(context) => total.max(before.saturating_add(non_negative(Some(context)))),
            };
            maximum = maximum.max(effective);
        }
    }
    // 少数版本把 rollup 直接写回 summary；作为最后回退。
    maximum = maximum.max(non_negative(summary.get("totalTokens")));
    Tokens {
        input: maximum,
        ..Tokens::default()
    }
}

/// `~/.grok/sessions`（受 `GROK_HOME` 覆盖）。每次调用都重读环境变量，
/// 对齐 Python 侧 `grok_home()` 的运行期求值。
pub fn sessions_root() -> PathBuf {
    grok_home(&process_environ(), &home_dir()).join("sessions")
}

/// 一个 bundle 目录的扫描行；结构不匹配当前格式时返回空 map。
fn meta(path: &Path) -> Option<ScanRow> {
    let summary_path = path.join("summary.json");
    let summary: Value = serde_json::from_str(&read_text(&summary_path).ok()?).ok()?;
    let info = summary.get("info").cloned().unwrap_or(Value::Null);
    let id = info.get("id").filter(|value| truthy(value));
    if summary.get("chat_format_version") != Some(&Value::from(1))
        || id.is_none()
        || !info.get("cwd").is_some_and(Value::is_string)
    {
        return Some(ScanRow::new());
    }
    let stat = fs::metadata(&summary_path).ok()?;
    let updated = summary
        .get("updated_at")
        .and_then(iso_ms)
        .filter(|value| *value != 0)
        .unwrap_or_else(|| mtime_millis(&stat));
    let title = summary
        .get("generated_title")
        .filter(|value| truthy(value))
        .or_else(|| summary.get("session_summary").filter(|value| truthy(value)))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let count = summary
        .get("num_chat_messages")
        .filter(|value| truthy(value))
        .or_else(|| summary.get("num_messages").filter(|value| truthy(value)))
        .cloned()
        .unwrap_or_else(|| Value::from(0));
    let root_id = summary
        .get("root_session_id")
        .filter(|value| truthy(value))
        .or(id)
        .cloned()
        .unwrap_or(Value::Null);
    let tokens = grok_tokens(path, &summary);

    let mut row = ScanRow::new();
    row.insert("tool".into(), Value::from("grok"));
    row.insert("id".into(), id.cloned().unwrap_or(Value::Null));
    row.insert("title".into(), Value::from(title));
    row.insert(
        "dir".into(),
        info.get("cwd").cloned().unwrap_or(Value::Null),
    );
    row.insert("updated".into(), Value::from(updated));
    row.insert(
        "created".into(),
        summary
            .get("created_at")
            .and_then(iso_ms)
            .map_or(Value::Null, Value::from),
    );
    row.insert("count".into(), count);
    row.insert("size".into(), Value::from(stat.len() as i64));
    row.insert("path".into(), Value::from(path.to_string_lossy().as_ref()));
    row.insert(
        "parent_id".into(),
        summary
            .get("parent_session_id")
            .cloned()
            .unwrap_or(Value::Null),
    );
    row.insert("root_id".into(), root_id);
    row.insert(
        "tokens".into(),
        if has_tokens(&tokens) {
            tokens.to_value()
        } else {
            Value::Null
        },
    );
    row.insert(
        "model".into(),
        summary
            .get("current_model_id")
            .filter(|value| truthy(value))
            .cloned()
            .unwrap_or_else(|| Value::from("")),
    );
    row.insert(
        "authoritative_members".into(),
        Value::Array(vec![
            Value::from("summary.json"),
            Value::from(if path.join("updates.jsonl").is_file() {
                "updates.jsonl"
            } else {
                "chat_history.jsonl"
            }),
        ]),
    );
    if has_tokens(&tokens) {
        let model = row
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("grok-unknown")
            .to_string();
        let mut by_model = serde_json::Map::new();
        by_model.insert(
            if model.is_empty() {
                "grok-unknown".to_string()
            } else {
                model
            },
            tokens.to_value(),
        );
        row.insert("usage_by_model".into(), Value::Object(by_model));
    }
    Some(row)
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|float| float != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(entries) => !entries.is_empty(),
    }
}

/// 等价 `int(stat.st_mtime * 1000)`（向零截断）。
fn mtime_millis(metadata: &fs::Metadata) -> i64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        metadata.mtime() * 1000 + metadata.mtime_nsec() / 1_000_000
    }
    #[cfg(not(unix))]
    {
        metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|delta| delta.as_millis() as i64)
            .unwrap_or(0)
    }
}

/// 递归收集 `<root>/**/summary.json`。按路径排序，让同一批 fixture 的扫描顺序
/// 稳定（Python 走 `rglob`，顺序由 readdir 决定，语义上不承诺顺序）。
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

/// 扫描全部 bundle，返回装配好的会话树。
pub fn scan(cache: &dyn ScanCache) -> DomainResult<Vec<ScanRow>> {
    let root = sessions_root();
    if !root.exists() {
        return Ok(Vec::new());
    }
    let summaries = summaries(&root);
    report_scan_total(summaries.len());
    let mut rows = Vec::new();
    for summary in summaries {
        report_scan_advance(1);
        let Some(path) = summary.parent().map(Path::to_path_buf) else {
            continue;
        };
        let Ok(metadata) = fs::metadata(&summary) else {
            continue;
        };
        // Grok 的 token 还依赖 updates/signals，不能只拿 summary stat 当缓存键；
        // bundle 较少且本就需要读历史，直接重算以免运行中的会话显示陈旧用量。
        let stat = FileStat::from_metadata(&metadata);
        let row = match meta(&path).unwrap_or_default() {
            parsed if parsed.is_empty() => {
                cache.put(&summary, &stat, None);
                parsed
            }
            parsed => {
                cache.put(&summary, &stat, Some(parsed.clone()));
                parsed
            }
        };
        if !row.is_empty() {
            rows.push(row);
        }
    }
    session_roots(rows)
}

/// Agent 检索阶段的 O(1) 修订标记：只 stat summary.json。
pub fn agent_fingerprint(reference: &str) -> DomainResult<Value> {
    let path = fs::canonicalize(reference)
        .map_err(|_| DomainError::session_not_found("grok", reference))?;
    let metadata = fs::metadata(path.join("summary.json"))
        .map_err(|_| DomainError::session_not_found("grok", reference))?;
    let stat = FileStat::from_metadata(&metadata);
    Ok(Value::from(stat_digest(&path, &stat)))
}

/// 完整内容指纹：读整个 bundle。
pub fn fingerprint(reference: &str) -> DomainResult<Value> {
    Ok(Value::from(bundle_fingerprint(Path::new(reference))?))
}

/// 单元测试与集成测试共用的空缓存（每次都 miss，写入即丢弃）。
#[derive(Clone, Copy, Debug, Default)]
pub struct NullScanCache;

impl ScanCache for NullScanCache {
    fn get(&self, _path: &Path, _stat: &FileStat) -> Option<Option<ScanRow>> {
        None
    }
    fn put(&self, _path: &Path, _stat: &FileStat, _meta: Option<ScanRow>) {}
    fn get_digest(&self, _path: &Path, _stat: &FileStat) -> Option<String> {
        None
    }
    fn put_digest(&self, _path: &Path, _stat: &FileStat, _digest: &str) {}
    fn flush(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write_bundle(root: &Path, name: &str, summary: Value, with_updates: bool) -> PathBuf {
        let path = root.join(name);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("summary.json"), summary.to_string()).unwrap();
        if with_updates {
            fs::write(path.join("updates.jsonl"), "{}\n").unwrap();
        } else {
            fs::write(path.join("chat_history.jsonl"), "{}\n").unwrap();
        }
        path
    }

    #[test]
    fn meta_prefers_the_iso_timestamp_over_mtime() {
        let root = tempfile::tempdir().unwrap();
        let path = write_bundle(
            root.path(),
            "b1",
            json!({"info": {"id": "s1", "cwd": "/w"}, "chat_format_version": 1,
                   "updated_at": "2026-07-25T12:00:02Z", "created_at": "2026-07-25T12:00:00Z",
                   "session_summary": "T", "num_messages": 4,
                   "current_model_id": "grok-code-fast-1"}),
            true,
        );
        let row = meta(&path).unwrap();
        assert_eq!(row["updated"], json!(1784980802000i64));
        assert_eq!(row["created"], json!(1784980800000i64));
        assert_eq!(row["title"], json!("T"));
        assert_eq!(row["count"], json!(4));
        assert_eq!(row["root_id"], json!("s1"));
        assert_eq!(row["tokens"], Value::Null);
        assert_eq!(
            row["authoritative_members"],
            json!(["summary.json", "updates.jsonl"])
        );
    }

    #[test]
    fn meta_reads_monotonic_grok_totals_and_reconciles_signals() {
        let root = tempfile::tempdir().unwrap();
        let path = write_bundle(
            root.path(),
            "usage",
            json!({"info": {"id": "s1", "cwd": "/w"}, "chat_format_version": 1,
                   "current_model_id": "grok-4.6"}),
            true,
        );
        fs::write(
            path.join("updates.jsonl"),
            concat!(
                "{\"params\":{\"_meta\":{\"totalTokens\":100}}}\n",
                "{\"params\":{\"_meta\":{\"totalTokens\":250}}}\n",
                "{\"params\":{\"_meta\":{\"totalTokens\":200}}}\n"
            ),
        )
        .unwrap();
        fs::write(
            path.join("signals.json"),
            json!({"totalTokensBeforeCompaction": 300, "contextTokensUsed": 50}).to_string(),
        )
        .unwrap();

        let row = meta(&path).unwrap();
        assert_eq!(
            row["tokens"],
            json!({"input": 350, "output": 0, "cache_read": 0, "cache_write": 0})
        );
        assert_eq!(row["usage_by_model"]["grok-4.6"], row["tokens"],);
    }

    #[test]
    fn meta_without_a_timestamp_falls_back_to_mtime() {
        let root = tempfile::tempdir().unwrap();
        let path = write_bundle(
            root.path(),
            "b2",
            json!({"info": {"id": "s2", "cwd": "/w"}, "chat_format_version": 1}),
            false,
        );
        let row = meta(&path).unwrap();
        assert!(row["updated"].as_i64().unwrap() > 0);
        assert_eq!(row["created"], Value::Null);
        assert_eq!(row["model"], json!(""));
        assert_eq!(
            row["authoritative_members"],
            json!(["summary.json", "chat_history.jsonl"])
        );
    }

    #[test]
    fn structural_drift_yields_an_empty_row() {
        let root = tempfile::tempdir().unwrap();
        // 版本不是 1。
        let path = write_bundle(
            root.path(),
            "b3",
            json!({"info": {"id": "s", "cwd": "/w"}, "chat_format_version": 2}),
            true,
        );
        assert!(meta(&path).unwrap().is_empty());
        // cwd 不是字符串。
        fs::write(
            path.join("summary.json"),
            json!({"info": {"id": "s", "cwd": 1}, "chat_format_version": 1}).to_string(),
        )
        .unwrap();
        assert!(meta(&path).unwrap().is_empty());
        // 损坏 JSON → None（不写缓存）。
        fs::write(path.join("summary.json"), "{").unwrap();
        assert!(meta(&path).is_none());
    }
}
