//! Claude 文件存储扫描。
//!
//! token 归一化与 `iso_ms` 在 Python 侧来自 `sessions.usage`；Rust 禁止
//! `adapters → sessions`（见 `adapters/mod.rs`），这几个纯函数由
//! `adapters::shared::scanner` 提供，`sessions::usage` 反过来复用它们。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use sha2::{Digest as _, Sha256};

use crate::adapters::contracts::{Fingerprint, ScanCache, ScanRow};
use crate::adapters::shared::scanner::{
    add_tokens, clip_text_default, dominant_model, empty_tokens, has_tokens, iso_ms, iter_lines,
    scan_jsonl, ScanOutcome, Tokens,
};
use crate::errors::{DomainError, DomainResult};
use crate::jsonutil::FileStat;
use crate::system::paths::{expanduser, is_within, realpath_strict};

/// Claude 的四个原生 usage 字段 → 归一化 token 桶。
fn usage_tokens(usage: &Value) -> Tokens {
    let count_of = |key: &str| -> i64 {
        match usage.get(key) {
            Some(Value::Number(number)) => number
                .as_i64()
                .or_else(|| number.as_f64().map(|float| float.trunc() as i64))
                .unwrap_or(0),
            _ => 0,
        }
    };
    Tokens {
        input: count_of("input_tokens"),
        output: count_of("output_tokens"),
        cache_read: count_of("cache_read_input_tokens"),
        cache_write: count_of("cache_creation_input_tokens"),
    }
}

fn usage_by_model_value(by_model: &[(String, Tokens)]) -> Value {
    let mut result = Map::new();
    for (model, tokens) in by_model {
        if has_tokens(tokens) {
            result.insert(model.clone(), tokens.to_value());
        }
    }
    Value::Object(result)
}

// ---------------------------------------------------------------------------
// 扫描
// ---------------------------------------------------------------------------

fn text_of(value: Option<&Value>) -> &str {
    value.and_then(Value::as_str).unwrap_or("")
}

/// 单个 JSONL 会话文件 → 扫描行；不是会话（无 user/assistant 记录）时返回空 map。
fn meta(path: &Path, stat: &FileStat, base: &Path) -> DomainResult<ScanOutcome> {
    let Ok(lines) = iter_lines(path) else {
        return Ok(ScanOutcome::Skipped);
    };
    let mut cwd = String::new();
    let mut title = String::new();
    let mut count = 0i64;
    let mut by_model: Vec<(String, Tokens)> = Vec::new();
    // Claude Code 的流式落盘会把同一 API 回复（message.id + requestId）写多行；
    // 后续行通常只是 content block 更完整，usage 仍是同一次请求。Tokscale 对
    // 复合键去重并按字段 max 合并，Ferry 也采用同一口径，避免逐行累加。
    let mut seen_usage: HashMap<String, (String, Tokens)> = HashMap::new();
    let mut created: Option<i64> = None;

    for line in lines {
        let Ok(line) = line else {
            return Ok(ScanOutcome::Skipped);
        };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Value>(&line) else {
            return Ok(ScanOutcome::Skipped);
        };
        let kind = record.get("type").and_then(Value::as_str);
        match kind {
            Some(kind @ ("user" | "assistant")) => {
                count += 1;
                if cwd.is_empty() {
                    cwd = text_of(record.get("cwd")).to_string();
                }
                let stamp = record
                    .get("timestamp")
                    .and_then(iso_ms)
                    .filter(|stamp| *stamp != 0);
                if let Some(stamp) = stamp {
                    if created.is_none_or(|current| stamp < current) {
                        created = Some(stamp);
                    }
                }
                let message = record.get("message").filter(|message| message.is_object());
                let model = message
                    .and_then(|message| message.get("model"))
                    .and_then(Value::as_str)
                    .filter(|model| !model.is_empty() && *model != "<synthetic>");
                if let (true, Some(model)) = (kind == "assistant", model) {
                    let usage = message
                        .and_then(|message| message.get("usage"))
                        .filter(|usage| usage.is_object())
                        .cloned()
                        .unwrap_or_else(|| Value::Object(Map::new()));
                    let tokens = usage_tokens(&usage);
                    let message_id = message
                        .and_then(|message| message.get("id"))
                        .and_then(Value::as_str);
                    let request_id = record.get("requestId").and_then(Value::as_str);
                    let dedup = match (message_id, request_id) {
                        (Some(message_id), Some(request_id)) => {
                            Some(format!("{message_id}:{request_id}"))
                        }
                        (Some(message_id), None) => Some(format!("message:{message_id}")),
                        _ => None,
                    };
                    if let Some(dedup) = dedup {
                        seen_usage
                            .entry(dedup)
                            .and_modify(|(seen_model, current)| {
                                if seen_model.is_empty() {
                                    *seen_model = model.to_string();
                                }
                                current.input = current.input.max(tokens.input);
                                current.output = current.output.max(tokens.output);
                                current.cache_read = current.cache_read.max(tokens.cache_read);
                                current.cache_write = current.cache_write.max(tokens.cache_write);
                            })
                            .or_insert_with(|| (model.to_string(), tokens));
                    } else {
                        if by_model.iter().all(|(name, _)| name != model) {
                            by_model.push((model.to_string(), empty_tokens()));
                        }
                        if let Some(slot) = by_model.iter_mut().find(|(name, _)| name == model) {
                            add_tokens(&mut slot.1, &tokens);
                        }
                    }
                }
                let content = message.and_then(|message| message.get("content"));
                if title.is_empty() && kind == "user" {
                    if let Some(Value::String(text)) = content {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() && !trimmed.starts_with('<') {
                            title = clip_text_default(text);
                        }
                    }
                }
            }
            Some("ai-title") => {
                let candidate = text_of(record.get("title"));
                if !candidate.is_empty() {
                    title = candidate.to_string();
                }
            }
            _ => {}
        }
    }

    if count == 0 {
        return Ok(ScanOutcome::Row(ScanRow::new()));
    }
    for (_, (model, tokens)) in seen_usage {
        if by_model.iter().all(|(name, _)| *name != model) {
            by_model.push((model.clone(), empty_tokens()));
        }
        if let Some(slot) = by_model.iter_mut().find(|(name, _)| *name == model) {
            add_tokens(&mut slot.1, &tokens);
        }
    }

    let relative = path.strip_prefix(base).unwrap_or(path);
    let parts: Vec<_> = relative.components().collect();
    let child = parts.len() > 2;
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let root_id = if child {
        parts[1].as_os_str().to_string_lossy().into_owned()
    } else {
        stem.clone()
    };
    let mut tokens = empty_tokens();
    for (_, model_tokens) in &by_model {
        add_tokens(&mut tokens, model_tokens);
    }

    let mut row = ScanRow::new();
    row.insert("tool".into(), Value::from("claude"));
    row.insert("id".into(), Value::from(stem));
    row.insert("title".into(), Value::from(title));
    row.insert("dir".into(), Value::from(cwd));
    row.insert("updated".into(), Value::from(mtime_ms(stat)));
    row.insert("created".into(), created.map_or(Value::Null, Value::from));
    row.insert("count".into(), Value::from(count));
    row.insert("size".into(), Value::from(stat.size as i64));
    row.insert(
        "path".into(),
        Value::from(path.to_string_lossy().into_owned()),
    );
    row.insert(
        "tokens".into(),
        if has_tokens(&tokens) {
            tokens.to_value()
        } else {
            Value::Null
        },
    );
    row.insert("model".into(), Value::from(dominant_model(&by_model)));
    row.insert("usage_by_model".into(), usage_by_model_value(&by_model));
    row.insert(
        "parent_id".into(),
        if child {
            Value::from(root_id.as_str())
        } else {
            Value::Null
        },
    );
    row.insert("root_id".into(), Value::from(root_id));
    Ok(ScanOutcome::Row(row))
}

/// `int(stat.st_mtime * 1000)`：先折成 Python 的 float 秒再乘 1000 截断。
fn mtime_ms(stat: &FileStat) -> i64 {
    let seconds = stat.mtime_ns as f64 / 1_000_000_000.0;
    (seconds * 1000.0) as i64
}

/// 扫描 `~/.claude/projects/**/*.jsonl`。
pub fn scan(cache: &dyn ScanCache) -> DomainResult<Vec<ScanRow>> {
    let base = expanduser("~/.claude/projects");
    let pattern = base.join("**/*.jsonl").to_string_lossy().into_owned();
    let parse = move |path: &Path, stat: &FileStat| meta(path, stat, &base);
    scan_jsonl(&pattern, cache, &parse)
}

/// 按顺序产出根会话及其 subagents 的 `(相对路径, 绝对路径)`，并校验读作用域。
fn tree(reference: &str) -> DomainResult<Vec<(String, PathBuf)>> {
    let out_of_scope = || DomainError::internal("Claude 会话树超出存储根目录");
    let path = realpath_strict(Path::new(reference))
        .map_err(|error| DomainError::internal(format!("claude 会话不可达: {error}")))?;
    let root = realpath_strict(&expanduser("~/.claude/projects"))
        .map_err(|error| DomainError::internal(format!("claude 存储根不可达: {error}")))?;

    let mut candidates = vec![path.clone()];
    let child_root = path.with_extension("").join("subagents");
    if child_root.exists() {
        let mut found: Vec<PathBuf> = walkdir::WalkDir::new(&child_root)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .map(walkdir::DirEntry::into_path)
            .filter(|path| path.extension().is_some_and(|suffix| suffix == "jsonl"))
            .collect();
        found.sort_by_key(|path| {
            path.components()
                .map(|component| component.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        });
        candidates.extend(found);
    }

    let mut out = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let resolved = realpath_strict(&candidate).map_err(|_| out_of_scope())?;
        if !resolved.is_file() || !is_within(&resolved.to_string_lossy(), &root.to_string_lossy()) {
            return Err(out_of_scope());
        }
        let relative = resolved
            .strip_prefix(&root)
            .map_err(|_| out_of_scope())?
            .to_string_lossy()
            .into_owned();
        out.push((relative, resolved));
    }
    Ok(out)
}

/// Claude 根会话及其 subagents 的只读树指纹（内容级）。
pub fn fingerprint(reference: &str) -> DomainResult<Fingerprint> {
    let mut digest = Sha256::new();
    for (relative, resolved) in tree(reference)? {
        digest.update(relative.as_bytes());
        digest.update(b"\0");
        let bytes = std::fs::read(&resolved)
            .map_err(|error| DomainError::internal(format!("claude 会话读取失败: {error}")))?;
        digest.update(&bytes);
        digest.update(b"\0");
    }
    Ok(Value::from(format!(
        "sha256:{}",
        super::editing::hex_lower(&digest.finalize())
    )))
}

/// Agent 检索阶段的 stat 级指纹。
pub fn agent_fingerprint(reference: &str) -> DomainResult<Fingerprint> {
    let mut digest = Sha256::new();
    for (relative, resolved) in tree(reference)? {
        let metadata = std::fs::metadata(&resolved)
            .map_err(|error| DomainError::internal(format!("claude 会话 stat 失败: {error}")))?;
        let stat = FileStat::from_metadata(&metadata);
        digest.update(relative.as_bytes());
        digest.update(
            format!(
                "\0{}:{}:{}:{}\0",
                stat.dev, stat.ino, stat.mtime_ns, stat.size
            )
            .as_bytes(),
        );
    }
    Ok(Value::from(format!(
        "stat:{}",
        super::editing::hex_lower(&digest.finalize())
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn usage_tokens_normalize_the_four_claude_fields() {
        let tokens = usage_tokens(&json!({
            "input_tokens": 10, "output_tokens": 2,
            "cache_read_input_tokens": 3, "cache_creation_input_tokens": 4
        }));
        assert_eq!(
            tokens,
            Tokens {
                input: 10,
                output: 2,
                cache_read: 3,
                cache_write: 4
            }
        );
        assert_eq!(usage_tokens(&json!({})), Tokens::default());
    }

    fn write(path: &Path, records: &[Value]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let payload: String = records
            .iter()
            .map(|record| format!("{}\n", serde_json::to_string(record).unwrap()))
            .collect();
        std::fs::write(path, payload).unwrap();
    }

    fn stat_of(path: &Path) -> FileStat {
        FileStat::from_metadata(&std::fs::metadata(path).unwrap())
    }

    #[test]
    fn meta_extracts_title_tokens_and_tree_identity() {
        let root = tempfile::tempdir().unwrap();
        let base = root.path().join("projects");
        let path = base.join("slug/sess.jsonl");
        write(
            &path,
            &[
                json!({"type": "user", "cwd": "/w", "timestamp": "2026-07-25T00:00:00Z",
                       "message": {"role": "user", "content": "  Hello   there  "}}),
                json!({"type": "assistant", "timestamp": "2026-07-25T00:00:01Z",
                       "message": {"role": "assistant", "model": "opus",
                                   "usage": {"input_tokens": 7, "output_tokens": 1}}}),
                json!({"type": "summary", "summary": "ignored"}),
            ],
        );
        let ScanOutcome::Row(row) = meta(&path, &stat_of(&path), &base).unwrap() else {
            panic!("应当解析出扫描行");
        };
        assert_eq!(row["tool"], json!("claude"));
        assert_eq!(row["id"], json!("sess"));
        assert_eq!(row["title"], json!("Hello there"));
        assert_eq!(row["dir"], json!("/w"));
        assert_eq!(row["count"], json!(2));
        assert_eq!(row["created"], json!(1_784_937_600_000_i64));
        assert_eq!(row["model"], json!("opus"));
        assert_eq!(row["root_id"], json!("sess"));
        assert_eq!(row["parent_id"], json!(null));
        assert_eq!(
            row["tokens"],
            json!({"input": 7, "output": 1, "cache_read": 0, "cache_write": 0})
        );
    }

    #[test]
    fn subagent_rows_point_at_the_root_session() {
        let root = tempfile::tempdir().unwrap();
        let base = root.path().join("projects");
        let path = base.join("slug/sess/subagents/agent-x.jsonl");
        write(
            &path,
            &[json!({"type": "assistant", "message": {"role": "assistant"}})],
        );
        let ScanOutcome::Row(row) = meta(&path, &stat_of(&path), &base).unwrap() else {
            panic!("应当解析出扫描行");
        };
        assert_eq!(row["id"], json!("agent-x"));
        assert_eq!(row["root_id"], json!("sess"));
        assert_eq!(row["parent_id"], json!("sess"));
        assert_eq!(row["tokens"], json!(null));
        assert_eq!(row["model"], json!(""));
    }

    #[test]
    fn ai_title_overrides_the_first_user_line_and_xml_prompts_are_skipped() {
        let root = tempfile::tempdir().unwrap();
        let base = root.path().join("projects");
        let path = base.join("slug/sess.jsonl");
        write(
            &path,
            &[
                json!({"type": "user", "message": {"role": "user",
                       "content": "<command-name>skip</command-name>"}}),
                json!({"type": "ai-title", "title": "Real Title"}),
            ],
        );
        let ScanOutcome::Row(row) = meta(&path, &stat_of(&path), &base).unwrap() else {
            panic!("应当解析出扫描行");
        };
        assert_eq!(row["title"], json!("Real Title"));
    }

    #[test]
    fn files_without_conversation_records_yield_no_row() {
        let root = tempfile::tempdir().unwrap();
        let base = root.path().join("projects");
        let path = base.join("slug/sess.jsonl");
        write(&path, &[json!({"type": "summary", "summary": "x"})]);
        let ScanOutcome::Row(row) = meta(&path, &stat_of(&path), &base).unwrap() else {
            panic!("空 meta 也是 Row");
        };
        assert!(row.is_empty());
    }

    #[test]
    fn malformed_json_skips_the_file_without_caching() {
        let root = tempfile::tempdir().unwrap();
        let base = root.path().join("projects");
        let path = base.join("slug/sess.jsonl");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{oops}\n").unwrap();
        assert!(matches!(
            meta(&path, &stat_of(&path), &base).unwrap(),
            ScanOutcome::Skipped
        ));
    }
}
