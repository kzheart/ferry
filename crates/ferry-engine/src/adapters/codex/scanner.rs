//! Codex rollout 文件扫描。

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use serde_json::{Map, Value};

use crate::adapters::contracts::{ScanCache, ScanRow};
use crate::adapters::shared::scanner::{
    add_tokens, clip_text_default, dominant_model, empty_tokens, has_tokens, iso_ms, iter_lines,
    path_stat_fingerprint, scan_jsonl, ScanOutcome, Tokens,
};
use crate::errors::{DomainError, DomainResult};
use crate::jsonutil::FileStat;
use crate::system::paths::expanduser;

use super::native::discover_closure;
use super::topology;

/// Codex 的累计/单次 usage 原始桶。`output_tokens` 已包含 reasoning，不能再把
/// `reasoning_output_tokens` 加一次；Ferry 的四桶没有独立 reasoning 桶，因此
/// output 直接保留上游总 output。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct CodexTotals {
    input: i64,
    output: i64,
    cached: i64,
    cache_write: i64,
}

impl CodexTotals {
    fn from_usage(usage: &Map<String, Value>) -> Self {
        let read =
            |key: &str| -> i64 { usage.get(key).and_then(Value::as_i64).unwrap_or(0).max(0) };
        Self {
            input: read("input_tokens"),
            output: read("output_tokens"),
            cached: read("cached_input_tokens").max(read("cache_read_input_tokens")),
            cache_write: read("cache_write_input_tokens"),
        }
    }

    fn delta_from(self, previous: Self) -> Option<Self> {
        if self.input < previous.input
            || self.output < previous.output
            || self.cached < previous.cached
            || self.cache_write < previous.cache_write
        {
            return None;
        }
        Some(Self {
            input: self.input - previous.input,
            output: self.output - previous.output,
            cached: self.cached - previous.cached,
            cache_write: self.cache_write - previous.cache_write,
        })
    }

    fn total(self) -> i64 {
        self.input
            .saturating_add(self.output)
            .saturating_add(self.cached)
            .saturating_add(self.cache_write)
    }

    fn looks_like_stale_regression(self, previous: Self, last: Self) -> bool {
        let previous_total = previous.total();
        let current_total = self.total();
        let last_total = last.total();
        previous_total > 0
            && current_total > 0
            && last_total > 0
            && (current_total.saturating_mul(100) >= previous_total.saturating_mul(98)
                || current_total.saturating_add(last_total.saturating_mul(2)) >= previous_total)
    }

    fn into_tokens(self) -> Tokens {
        let cached = self.cached.min(self.input).max(0);
        Tokens {
            input: (self.input - cached).max(0),
            output: self.output.max(0),
            cache_read: cached,
            cache_write: self.cache_write.max(0),
        }
    }
}

fn add_model_tokens(by_model: &mut Vec<(String, Tokens)>, model: &str, tokens: &Tokens) {
    let model = if model.is_empty() { "unknown" } else { model };
    match by_model.iter_mut().find(|(name, _)| name == model) {
        Some((_, current)) => add_tokens(current, tokens),
        None => by_model.push((model.to_string(), *tokens)),
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

fn is_forked_session(payload: &Map<String, Value>) -> bool {
    if payload
        .get("forked_from_id")
        .and_then(Value::as_str)
        .is_some_and(|id| !id.is_empty())
    {
        return true;
    }
    payload
        .get("source")
        .and_then(Value::as_object)
        .and_then(|source| source.get("subagent"))
        .and_then(Value::as_object)
        .and_then(|subagent| subagent.get("thread_spawn"))
        .and_then(Value::as_object)
        .and_then(|spawn| spawn.get("parent_thread_id"))
        .and_then(Value::as_str)
        .is_some_and(|id| !id.is_empty())
}

fn codex_uuid_v7_order_key(id: &str) -> Option<String> {
    let mut parts = id.split('-');
    let parts = [
        parts.next()?,
        parts.next()?,
        parts.next()?,
        parts.next()?,
        parts.next()?,
    ];
    if parts.iter().map(|part| part.len()).collect::<Vec<_>>() != [8, 4, 4, 4, 12]
        || !parts[2].starts_with('7')
        || parts
            .iter()
            .any(|part| !part.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        return None;
    }
    Some(parts.concat().to_ascii_lowercase())
}

fn fork_turn_is_child_local(
    child_id: &str,
    replay_session_id: Option<&str>,
    task_started_turns: &HashSet<String>,
    user_fork: bool,
    turn_id: Option<&str>,
) -> bool {
    if replay_session_id.is_none() {
        return true;
    }
    let (Some(turn_id), Some(child_key)) = (turn_id, codex_uuid_v7_order_key(child_id)) else {
        return true;
    };
    let Some(turn_key) = codex_uuid_v7_order_key(turn_id) else {
        return user_fork || task_started_turns.contains(turn_id);
    };
    match turn_key[..12].cmp(&child_key[..12]) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => user_fork || task_started_turns.contains(turn_id),
    }
}

fn task_starts_child_turn(child_id: &str, turn_id: Option<&str>, started_at: Option<i64>) -> bool {
    let Some(child_key) = codex_uuid_v7_order_key(child_id) else {
        return turn_id.is_some();
    };
    if let Some(turn_key) = turn_id.and_then(codex_uuid_v7_order_key) {
        return turn_key[..12] >= child_key[..12];
    }
    let Some(started_at) = started_at else {
        return false;
    };
    i64::from_str_radix(&child_key[..12], 16).is_ok_and(|child_ms| started_at >= child_ms / 1000)
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
    let mut by_model: Vec<(String, Tokens)> = Vec::new();
    let mut previous_totals: Option<CodexTotals> = None;
    let mut forked = false;
    let mut waiting_for_own_turn = false;
    let mut forked_child_id: Option<String> = None;
    let mut replay_session_id: Option<String> = None;
    let mut task_started_turns: HashSet<String> = HashSet::new();
    let mut user_fork = false;
    let mut inherited_baseline: Option<CodexTotals> = None;
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
        let subtype = payload.get("type").and_then(Value::as_str);
        if waiting_for_own_turn {
            if record_type == "turn_context"
                && forked_child_id.as_deref().is_some_and(|child_id| {
                    fork_turn_is_child_local(
                        child_id,
                        replay_session_id.as_deref(),
                        &task_started_turns,
                        user_fork,
                        payload.get("turn_id").and_then(Value::as_str),
                    )
                })
            {
                waiting_for_own_turn = false;
                replay_session_id = None;
                task_started_turns.clear();
                user_fork = false;
                if let Some(value) = truthy_str(payload.get("model")) {
                    model = value;
                }
            } else {
                if record_type == "session_meta" {
                    if let Some(replayed_id) = payload.get("id").and_then(Value::as_str) {
                        if forked_child_id.as_deref() != Some(replayed_id) {
                            replay_session_id = Some(replayed_id.to_string());
                        }
                    }
                } else if record_type == "event_msg" && subtype == Some("task_started") {
                    if forked_child_id.as_deref().is_some_and(|child_id| {
                        task_starts_child_turn(
                            child_id,
                            payload.get("turn_id").and_then(Value::as_str),
                            payload.get("started_at").and_then(Value::as_i64),
                        )
                    }) {
                        if let Some(turn_id) = payload.get("turn_id").and_then(Value::as_str) {
                            task_started_turns.insert(turn_id.to_string());
                        }
                    }
                } else if record_type == "event_msg" && subtype == Some("token_count") {
                    if let Some(total) = payload
                        .get("info")
                        .and_then(Value::as_object)
                        .and_then(|info| info.get("total_token_usage"))
                        .and_then(Value::as_object)
                        .filter(|usage| !usage.is_empty())
                        .map(CodexTotals::from_usage)
                    {
                        previous_totals = Some(total);
                        inherited_baseline = Some(total);
                    }
                }
                continue;
            }
        }
        if record_type == "turn_context" {
            if let Some(value) = truthy_str(payload.get("model")) {
                model = value;
            }
        } else if record_type == "event_msg" && subtype == Some("token_count") {
            let info = payload.get("info").and_then(Value::as_object);
            let total = info
                .and_then(|info| info.get("total_token_usage"))
                .and_then(Value::as_object)
                .filter(|usage| !usage.is_empty())
                .map(CodexTotals::from_usage);
            let last = info
                .and_then(|info| info.get("last_token_usage"))
                .and_then(Value::as_object)
                .filter(|usage| !usage.is_empty())
                .map(CodexTotals::from_usage);

            if forked
                && inherited_baseline.is_some_and(|baseline| {
                    total.is_some_and(|current| {
                        current
                            .delta_from(baseline)
                            .is_some_and(|delta| delta == CodexTotals::default())
                    })
                })
            {
                continue;
            }
            if let (Some(total), Some(baseline)) = (total, inherited_baseline) {
                if total.input <= baseline.input
                    && total.output <= baseline.output
                    && total.cached <= baseline.cached
                    && total.cache_write <= baseline.cache_write
                {
                    continue;
                }
                inherited_baseline = None;
            }

            let (tokens, next_totals) = match (total, last, previous_totals) {
                // Tokscale 的主路径：`last_token_usage` 是本次增量；累计值只做去重/
                // 单调性基线，避免 compaction 或 resumed session 把历史再算一遍。
                (Some(total), Some(last), Some(previous)) => {
                    if total == previous {
                        continue;
                    }
                    if total.delta_from(previous).is_none()
                        && total.looks_like_stale_regression(previous, last)
                    {
                        continue;
                    }
                    (last.into_tokens(), Some(total))
                }
                (Some(total), Some(last), None) => (last.into_tokens(), Some(total)),
                (Some(total), None, Some(previous)) => match total.delta_from(previous) {
                    Some(delta) => (delta.into_tokens(), Some(total)),
                    None => {
                        previous_totals = Some(total);
                        continue;
                    }
                },
                (Some(total), None, None) => (total.into_tokens(), Some(total)),
                (None, Some(last), Some(previous)) => (last.into_tokens(), Some(previous)),
                (None, Some(last), None) => (last.into_tokens(), None),
                (None, None, _) => continue,
            };
            if !has_tokens(&tokens) {
                continue;
            }
            previous_totals = next_totals;
            add_model_tokens(&mut by_model, &model, &tokens);
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
            forked = is_forked_session(payload);
            waiting_for_own_turn = forked;
            if forked {
                forked_child_id = Some(sid.clone());
                user_fork = payload.get("thread_source").and_then(Value::as_str) == Some("user");
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
    let mut tokens = empty_tokens();
    for (_, model_tokens) in &by_model {
        add_tokens(&mut tokens, model_tokens);
    }
    let dominant = dominant_model(&by_model);
    if !dominant.is_empty() {
        model = dominant;
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
        if has_tokens(&tokens) {
            tokens.to_value()
        } else {
            Value::Null
        },
    );
    row.insert("usage_by_model".into(), usage_by_model_value(&by_model));
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
                        "cache_write_input_tokens": 7},
                    "last_token_usage": {
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
            // reasoning 是 output 的子集，不再重复相加。
            json!({"input": 70, "output": 10, "cache_read": 30, "cache_write": 7})
        );
        assert_eq!(row["model"], json!("gpt-5.4"));
    }

    #[test]
    fn token_count_uses_last_usage_increments_instead_of_the_final_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let path = write(
            temp.path(),
            "rollout-a.jsonl",
            &[
                json!({"type": "session_meta", "payload": {"id": "a", "cwd": "/w"}}),
                json!({"type": "turn_context", "payload": {"model": "gpt-5.6-sol"}}),
                json!({"type": "event_msg", "payload": {"type": "token_count", "info": {
                    "total_token_usage": {"input_tokens": 100, "cached_input_tokens": 40,
                                          "output_tokens": 10},
                    "last_token_usage": {"input_tokens": 100, "cached_input_tokens": 40,
                                         "output_tokens": 10}}}}),
                json!({"type": "event_msg", "payload": {"type": "token_count", "info": {
                    "total_token_usage": {"input_tokens": 250, "cached_input_tokens": 140,
                                          "output_tokens": 30},
                    "last_token_usage": {"input_tokens": 150, "cached_input_tokens": 100,
                                         "output_tokens": 20}}}}),
                json!({"type": "response_item", "payload": {"type": "message", "role": "user",
                       "content": [{"type": "input_text", "text": "go"}]}}),
            ],
        );
        assert_eq!(
            meta_of(&path)["tokens"],
            json!({"input": 110, "output": 30, "cache_read": 140, "cache_write": 0})
        );
    }

    #[test]
    fn forked_child_skips_inherited_history_before_its_own_turn() {
        let temp = tempfile::tempdir().unwrap();
        let path = write(
            temp.path(),
            "rollout-child.jsonl",
            &[
                json!({"type": "session_meta", "payload": {
                    "id": "01a02f36-4ad8-7273-b72b-d92a48cda34b", "cwd": "/w",
                    "forked_from_id": "01a02f35-aee3-71d0-b628-9d442ec2ac8a",
                    "parent_thread_id": "01a02f35-aee3-71d0-b628-9d442ec2ac8a"}}),
                // 真实 fork 会完整重放父 session_meta + task_started + turn_context。
                json!({"type": "session_meta", "payload": {
                    "id": "01a02f35-aee3-71d0-b628-9d442ec2ac8a", "cwd": "/w"}}),
                json!({"type": "event_msg", "payload": {"type": "task_started",
                    "turn_id": "01a02f35-b53c-77b0-a6ec-86703ed2ada8",
                    "started_at": 1_787_498_444}}),
                json!({"type": "turn_context", "payload": {
                    "turn_id": "01a02f35-b53c-77b0-a6ec-86703ed2ada8",
                    "model": "gpt-5.6-sol"}}),
                json!({"type": "event_msg", "payload": {"type": "token_count", "info": {
                    "total_token_usage": {"input_tokens": 1_000, "cached_input_tokens": 800,
                                          "output_tokens": 100},
                    "last_token_usage": {"input_tokens": 200, "cached_input_tokens": 100,
                                         "output_tokens": 20}}}}),
                json!({"type": "event_msg", "payload": {"type": "task_started",
                    "turn_id": "01a02f36-4b67-77c1-bf6b-076f9f500b01",
                    "started_at": 1_787_498_482}}),
                json!({"type": "turn_context", "payload": {
                    "turn_id": "01a02f36-4b67-77c1-bf6b-076f9f500b01",
                    "model": "gpt-5.6-sol"}}),
                json!({"type": "event_msg", "payload": {"type": "token_count", "info": {
                    "total_token_usage": {"input_tokens": 1_000, "cached_input_tokens": 800,
                                          "output_tokens": 100},
                    "last_token_usage": {"input_tokens": 200, "cached_input_tokens": 100,
                                         "output_tokens": 20}}}}),
                json!({"type": "event_msg", "payload": {"type": "token_count", "info": {
                    "total_token_usage": {"input_tokens": 1_300, "cached_input_tokens": 1_000,
                                          "output_tokens": 140},
                    "last_token_usage": {"input_tokens": 300, "cached_input_tokens": 200,
                                         "output_tokens": 40}}}}),
                json!({"type": "response_item", "payload": {"type": "message", "role": "user",
                       "content": [{"type": "input_text", "text": "child work"}]}}),
            ],
        );
        assert_eq!(
            meta_of(&path)["tokens"],
            json!({"input": 100, "output": 40, "cache_read": 200, "cache_write": 0})
        );
    }

    #[test]
    fn token_total_epoch_reset_keeps_the_first_new_increment() {
        let temp = tempfile::tempdir().unwrap();
        let path = write(
            temp.path(),
            "rollout-reset.jsonl",
            &[
                json!({"type": "session_meta", "payload": {"id": "a", "cwd": "/w"}}),
                json!({"type": "turn_context", "payload": {"model": "gpt-5.6-sol"}}),
                json!({"type": "event_msg", "payload": {"type": "token_count", "info": {
                    "total_token_usage": {"input_tokens": 3_000, "cached_input_tokens": 2_000,
                                          "output_tokens": 500},
                    "last_token_usage": {"input_tokens": 1_000, "cached_input_tokens": 700,
                                         "output_tokens": 200}}}}),
                // 累计值大幅回落代表新 epoch；last 仍是第一条真实调用，必须计入。
                json!({"type": "event_msg", "payload": {"type": "token_count", "info": {
                    "total_token_usage": {"input_tokens": 100, "cached_input_tokens": 60,
                                          "output_tokens": 20},
                    "last_token_usage": {"input_tokens": 100, "cached_input_tokens": 60,
                                         "output_tokens": 20}}}}),
                json!({"type": "response_item", "payload": {"type": "message", "role": "user",
                       "content": [{"type": "input_text", "text": "go"}]}}),
            ],
        );
        assert_eq!(
            meta_of(&path)["tokens"],
            json!({"input": 340, "output": 220, "cache_read": 760, "cache_write": 0})
        );
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
