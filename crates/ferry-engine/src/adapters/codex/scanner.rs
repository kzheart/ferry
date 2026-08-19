//! Codex rollout 文件扫描。
//!
//! 语义事实源：`engine/adapters/codex/scanner.py`。

use std::fs;
use std::path::Path;

use serde_json::{Map, Value};

use crate::adapters::contracts::{ScanCache, ScanRow};
use crate::adapters::shared::scanner::{
    clip_text_default, has_tokens, iso_ms, iter_lines, path_stat_fingerprint, scan_jsonl,
    ScanOutcome, Tokens,
};
use crate::errors::{DomainError, DomainResult};
use crate::jsonutil::FileStat;
use crate::system::paths::expanduser;

use super::native::discover_closure;
use super::topology;

/// Codex 的 `total_token_usage` 是累计值；`input_tokens` 含缓存命中，拆出 cache_read。
fn tokens_from_usage(usage: &Map<String, Value>) -> Tokens {
    // `usage.get(key) or 0`：缺键、null、0 都归一成 0。
    let read = |key: &str| -> i64 { usage.get(key).and_then(Value::as_i64).unwrap_or(0) };
    let cached = read("cached_input_tokens");
    Tokens {
        input: (read("input_tokens") - cached).max(0),
        output: read("output_tokens") + read("reasoning_output_tokens"),
        cache_read: cached,
        cache_write: read("cache_write_input_tokens"),
    }
}

fn nullable(value: Option<String>) -> Value {
    value.map_or(Value::Null, Value::from)
}

fn truthy_str(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Null | Value::Bool(false) => None,
        Value::String(text) if text.is_empty() => None,
        Value::String(text) => Some(text.clone()),
        other => Some(crate::adapters::shared::dialect::python_str(other)),
    }
}

/// 解析一条 rollout 的扫描行；没有可见消息时返回空表（等价 Python 的 `{}`）。
fn meta(path: &Path, stat: &FileStat) -> DomainResult<ScanOutcome> {
    let mut sid = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut cwd = String::new();
    let mut title = String::new();
    let mut count = 0i64;
    let mut parent_id: Option<String> = None;
    let mut root_id: Option<String> = None;
    let mut agent_id: Option<String> = None;
    let mut agent_path: Option<String> = None;
    let mut agent_type: Option<String> = None;
    let mut has_meta = false;
    let mut model = String::new();
    let mut tokens: Option<Tokens> = None;
    let mut created: Option<i64> = None;

    let Ok(lines) = iter_lines(path) else {
        return Ok(ScanOutcome::Skipped);
    };
    for line in lines {
        // 读失败对齐 Python 的 `except OSError`：跳过整份文件且不写缓存。
        let Ok(line) = line else {
            return Ok(ScanOutcome::Skipped);
        };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Value>(&line) else {
            return Ok(ScanOutcome::Skipped);
        };
        if created.is_none() {
            created = iso_ms(record.get("timestamp").unwrap_or(&Value::Null));
        }
        let record_type = record.get("type").and_then(Value::as_str).unwrap_or("");
        let empty = Map::new();
        let payload = record
            .get("payload")
            .and_then(Value::as_object)
            .unwrap_or(&empty);
        if record_type == "turn_context" {
            if let Some(value) = truthy_str(payload.get("model")) {
                model = value;
            }
        } else if record_type == "event_msg"
            && payload.get("type").and_then(Value::as_str) == Some("token_count")
        {
            let usage = payload
                .get("info")
                .and_then(Value::as_object)
                .and_then(|info| info.get("total_token_usage"))
                .and_then(Value::as_object);
            if let Some(usage) = usage.filter(|usage| !usage.is_empty()) {
                tokens = Some(tokens_from_usage(usage));
            }
        }
        if record_type == "session_meta" && !has_meta {
            sid = payload
                .get("id")
                .map(crate::adapters::shared::dialect::python_str)
                .ok_or_else(|| topology::missing_identity_error(path))?;
            cwd = payload
                .get("cwd")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let source = payload
                .get("source")
                .and_then(Value::as_object)
                .unwrap_or(&empty);
            let subagent = source
                .get("subagent")
                .and_then(Value::as_object)
                .unwrap_or(&empty);
            let spawn = subagent
                .get("thread_spawn")
                .and_then(Value::as_object)
                .unwrap_or(&empty);
            root_id = truthy_str(payload.get("session_id")).or_else(|| Some(sid.clone()));
            parent_id = truthy_str(payload.get("parent_thread_id"))
                .or_else(|| truthy_str(spawn.get("parent_thread_id")))
                .or_else(|| truthy_str(subagent.get("parent_thread_id")));
            if parent_id.is_none() && root_id.as_deref() != Some(sid.as_str()) {
                parent_id = root_id.clone();
            }
            agent_id = truthy_str(subagent.get("agent_id"))
                .or_else(|| truthy_str(spawn.get("agent_id")))
                .or_else(|| truthy_str(payload.get("agent_id")));
            agent_path = truthy_str(subagent.get("agent_path"))
                .or_else(|| truthy_str(spawn.get("agent_path")))
                .or_else(|| truthy_str(payload.get("agent_path")));
            agent_type = truthy_str(subagent.get("agent_type"))
                .or_else(|| truthy_str(spawn.get("agent_type")))
                .or_else(|| truthy_str(payload.get("agent_type")));
            if model.is_empty() {
                model = truthy_str(payload.get("model")).unwrap_or_default();
            }
            has_meta = true;
        } else if record_type == "response_item"
            && payload.get("type").and_then(Value::as_str) == Some("message")
        {
            count += 1;
            let text = payload
                .get("content")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .map(|block| {
                            block
                                .get("text")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string()
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            let trimmed = text.trim();
            if title.is_empty()
                && payload.get("role").and_then(Value::as_str) == Some("user")
                && !trimmed.is_empty()
                && !trimmed.starts_with(['<', '['])
            {
                title = clip_text_default(&text);
            }
        }
    }
    if count == 0 {
        return Ok(ScanOutcome::Row(ScanRow::new()));
    }
    let mut row = ScanRow::new();
    row.insert("tool".into(), Value::from("codex"));
    row.insert("id".into(), Value::from(sid.as_str()));
    row.insert("title".into(), Value::from(title));
    row.insert("dir".into(), Value::from(cwd));
    row.insert(
        "updated".into(),
        Value::from((stat.mtime_ns / 1_000_000) as i64),
    );
    row.insert("created".into(), created.map_or(Value::Null, Value::from));
    row.insert("count".into(), Value::from(count));
    row.insert("size".into(), Value::from(stat.size as i64));
    row.insert("path".into(), Value::from(path.to_string_lossy().as_ref()));
    row.insert("parent_id".into(), nullable(parent_id));
    row.insert("root_id".into(), Value::from(root_id.unwrap_or(sid)));
    row.insert("agent_id".into(), nullable(agent_id));
    row.insert("agent_path".into(), nullable(agent_path));
    row.insert("agent_type".into(), nullable(agent_type));
    row.insert(
        "tokens".into(),
        match tokens {
            Some(tokens) if has_tokens(&tokens) => tokens.to_value(),
            _ => Value::Null,
        },
    );
    row.insert("model".into(), Value::from(model));
    Ok(ScanOutcome::Row(row))
}

/// 扫描 `~/.codex/sessions/*/*/*/rollout-*.jsonl`。
pub fn scan(cache: &dyn ScanCache) -> DomainResult<Vec<ScanRow>> {
    let pattern = expanduser("~/.codex/sessions/*/*/*/rollout-*.jsonl");
    scan_jsonl(&pattern.to_string_lossy(), cache, &meta)
}

/// 计算 Codex 可达会话 closure（含 registry）的只读指纹。
pub fn fingerprint(reference: &str) -> DomainResult<String> {
    let path = fs::canonicalize(reference)
        .map_err(|_| DomainError::session_not_found("codex", reference))?;
    let closure = discover_closure(&path, None)?;
    Ok(format!(
        "{}:{}",
        closure.revision,
        closure.registry_revision.as_deref().unwrap_or("none")
    ))
}

/// Agent 检索阶段的 O(1) 修订标记。
pub fn agent_fingerprint(reference: &str) -> DomainResult<String> {
    path_stat_fingerprint(reference).map_err(|_| DomainError::session_not_found("codex", reference))
}

/// 供测试直接构造扫描行。
#[cfg(test)]
fn meta_of(path: &Path) -> ScanRow {
    let metadata = fs::metadata(path).unwrap();
    match meta(path, &FileStat::from_metadata(&metadata)).unwrap() {
        ScanOutcome::Row(row) => row,
        ScanOutcome::Skipped => ScanRow::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write(dir: &Path, name: &str, records: &[Value]) -> std::path::PathBuf {
        let path = dir.join(name);
        let payload: String = records
            .iter()
            .map(|record| serde_json::to_string(record).unwrap() + "\n")
            .collect();
        fs::write(&path, payload).unwrap();
        path
    }

    #[test]
    fn rows_without_messages_are_dropped() {
        let temp = tempfile::tempdir().unwrap();
        let path = write(
            temp.path(),
            "rollout-a.jsonl",
            &[json!({"type": "session_meta", "payload": {"id": "a", "cwd": "/w"}})],
        );
        assert!(meta_of(&path).is_empty());
    }

    #[test]
    fn the_first_plain_user_message_becomes_the_title() {
        let temp = tempfile::tempdir().unwrap();
        let path = write(
            temp.path(),
            "rollout-a.jsonl",
            &[
                json!({"type": "session_meta", "payload": {"id": "a", "cwd": "/w"}}),
                // 尖括号/方括号开头的正文是环境上下文，不做标题。
                json!({"type": "response_item", "payload": {"type": "message", "role": "user",
                       "content": [{"type": "input_text", "text": "<env>"}]}}),
                json!({"type": "response_item", "payload": {"type": "message", "role": "user",
                       "content": [{"type": "input_text", "text": "  hello   world  "}]}}),
                json!({"type": "response_item", "payload": {"type": "message", "role": "user",
                       "content": [{"type": "input_text", "text": "later"}]}}),
            ],
        );
        let row = meta_of(&path);
        assert_eq!(row["title"], json!("hello world"));
        assert_eq!(row["count"], json!(3));
        assert_eq!(row["id"], json!("a"));
        assert_eq!(row["root_id"], json!("a"));
        assert_eq!(row["parent_id"], json!(null));
    }

    #[test]
    fn cumulative_token_usage_splits_out_cache_reads() {
        let temp = tempfile::tempdir().unwrap();
        let path = write(
            temp.path(),
            "rollout-a.jsonl",
            &[
                json!({"type": "session_meta", "payload": {"id": "a", "cwd": "/w"}}),
                json!({"type": "turn_context", "payload": {"model": "gpt-5.4"}}),
                json!({"type": "event_msg", "payload": {"type": "token_count", "info": {
                    "total_token_usage": {
                        "input_tokens": 100, "cached_input_tokens": 30,
                        "output_tokens": 10, "reasoning_output_tokens": 5,
                        "cache_write_input_tokens": 7}}}}),
                json!({"type": "response_item", "payload": {"type": "message", "role": "user",
                       "content": [{"type": "input_text", "text": "go"}]}}),
            ],
        );
        let row = meta_of(&path);
        assert_eq!(
            row["tokens"],
            json!({"input": 70, "output": 15, "cache_read": 30, "cache_write": 7})
        );
        assert_eq!(row["model"], json!("gpt-5.4"));
    }

    #[test]
    fn subagent_metadata_populates_the_parent_columns() {
        let temp = tempfile::tempdir().unwrap();
        let path = write(
            temp.path(),
            "rollout-a.jsonl",
            &[
                json!({"type": "session_meta", "payload": {
                    "id": "child", "session_id": "root", "cwd": "/w",
                    "source": {"subagent": {"agent_id": "ag", "thread_spawn": {
                        "agent_path": "/root/docs", "agent_type": "docs"}}}}}),
                json!({"type": "response_item", "payload": {"type": "message", "role": "user",
                       "content": [{"type": "input_text", "text": "go"}]}}),
            ],
        );
        let row = meta_of(&path);
        assert_eq!(row["root_id"], json!("root"));
        // parent_thread_id 缺席且 root != sid → 用 root 兜底。
        assert_eq!(row["parent_id"], json!("root"));
        assert_eq!(row["agent_id"], json!("ag"));
        assert_eq!(row["agent_path"], json!("/root/docs"));
        assert_eq!(row["agent_type"], json!("docs"));
    }

    #[test]
    fn zero_token_buckets_are_reported_as_null() {
        let temp = tempfile::tempdir().unwrap();
        let path = write(
            temp.path(),
            "rollout-a.jsonl",
            &[
                json!({"type": "session_meta", "payload": {"id": "a", "cwd": "/w"}}),
                json!({"type": "event_msg", "payload": {"type": "token_count", "info": {
                    "total_token_usage": {"input_tokens": 0}}}}),
                json!({"type": "response_item", "payload": {"type": "message", "role": "user",
                       "content": [{"type": "input_text", "text": "go"}]}}),
            ],
        );
        assert_eq!(meta_of(&path)["tokens"], json!(null));
    }
}
