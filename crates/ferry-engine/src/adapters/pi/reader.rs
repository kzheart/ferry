//! Pi v3 活动分支投影到 canonical 会话模型。
//!
//! pi 的会话文件是 append-only 树：每条 entry 用 `parentId` 指向上一条，重放
//! （rewind / 重问）会在同一个文件里长出新分支。读取时只投影**活动分支**：
//! 从最后一条有效 entry 沿 `parentId` 回溯到根、反转即得；非活动分支原样留在
//! 文件里，并作为一条 `migration.unknown_block_dropped` 损耗上报。

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use serde_json::{Map, Value};

use crate::adapters::shared::dialect::python_str;
use crate::adapters::shared::media::image_from_base64;
use crate::adapters::shared::scanner::split_jsonl_lines;
use crate::errors::{DomainError, DomainResult};
use crate::model::{
    Block, BlockKind, ContextCompaction, Message, Session, Timestamp, ToolCall, ToolResult,
    ToolResultBlock, ToolResultStatus,
};
use crate::tool_ops::CanonicalOp;

use super::tool_calls::{call_from_part, result_from_message, truthy};

/// `Path.read_text()` 的等价物：Python 文本模式会把 `\r\n` / `\r` 归一成 `\n`，
/// 之后才轮到 `split_jsonl_lines` 按 `\n` 切分。
fn read_text(path: &Path) -> DomainResult<String> {
    let bytes = fs::read(path)
        .map_err(|_| DomainError::session_not_found("pi", &path.to_string_lossy()))?;
    let text =
        String::from_utf8(bytes).map_err(|_| DomainError::internal("Pi 会话文件不是合法 UTF-8"))?;
    Ok(text.replace("\r\n", "\n").replace('\r', "\n"))
}

/// 原生记录 + 坏行行号。
pub struct Loaded {
    pub header: Value,
    pub entries: Vec<Value>,
    /// 1 起的行号；只记「非最后一条非空行」的坏行。
    pub malformed: Vec<usize>,
}

/// 读入并校验 v3 头部；末行截断容忍，中间坏行记账。
pub fn load(path: &Path) -> DomainResult<Loaded> {
    let text = read_text(path)?;
    let lines = split_jsonl_lines(&text);
    let final_index = lines
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .map_or(-1i64, |index| index as i64);
    let mut records: Vec<Value> = Vec::new();
    let mut malformed: Vec<usize> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(line) {
            Ok(value) => {
                if value.is_object() {
                    records.push(value);
                }
            }
            Err(_) => {
                if index as i64 != final_index {
                    malformed.push(index + 1);
                }
            }
        }
    }
    if records.is_empty() {
        return Err(DomainError::agent_format_changed(
            "pi",
            "header",
            Value::from("Pi v3 session"),
            Value::Null,
        ));
    }
    let header = records.remove(0);
    if !is_v3_header(&header) {
        let mut expected = Map::new();
        expected.insert("type".into(), Value::from("session"));
        expected.insert("version".into(), Value::from(3));
        return Err(DomainError::agent_format_changed(
            "pi",
            "header",
            Value::Object(expected),
            header,
        ));
    }
    Ok(Loaded {
        header,
        entries: records,
        malformed,
    })
}

/// v3 头部：`type == "session"`、`version == 3`，且 id/timestamp/cwd 都是非空串。
pub fn is_v3_header(header: &Value) -> bool {
    header.get("type").and_then(Value::as_str) == Some("session")
        && header.get("version").and_then(Value::as_i64) == Some(3)
        && ["id", "timestamp", "cwd"].iter().all(|key| {
            header
                .get(*key)
                .and_then(Value::as_str)
                .is_some_and(|value| !value.is_empty())
        })
}

/// 只有「id 是字符串且显式带 parentId 键」的 entry 参与分支回溯。
fn is_valid_entry(entry: &Value) -> bool {
    entry.get("id").and_then(Value::as_str).is_some() && entry.get("parentId").is_some()
}

fn entry_id(entry: &Value) -> &str {
    entry.get("id").and_then(Value::as_str).unwrap_or_default()
}

/// 活动分支：从最后一条有效 entry 沿 parentId 回溯到根，再反转。
///
/// 第二个返回值是被选中的 entry id 集合（compaction 的 tail 定位与非活动分支
/// 上报都用它）。
pub fn active_branch(entries: &[Value]) -> (Vec<&Value>, HashSet<String>) {
    let valid: Vec<&Value> = entries
        .iter()
        .filter(|entry| is_valid_entry(entry))
        .collect();
    let Some(last) = valid.last().copied() else {
        return (Vec::new(), HashSet::new());
    };
    let by_id: HashMap<&str, &Value> = valid
        .iter()
        .map(|entry| (entry_id(entry), *entry))
        .collect();
    let mut branch: Vec<&Value> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut current = Some(last);
    while let Some(entry) = current {
        if seen.contains(entry_id(entry)) {
            break;
        }
        branch.push(entry);
        seen.insert(entry_id(entry).to_string());
        current = entry
            .get("parentId")
            .and_then(Value::as_str)
            .and_then(|parent| by_id.get(parent).copied());
    }
    branch.reverse();
    (branch, seen)
}

fn params(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

fn timestamp_of(entry: &Value) -> Option<Timestamp> {
    match entry.get("timestamp") {
        Some(Value::String(text)) => Some(Timestamp::Text(text.clone())),
        Some(Value::Number(number)) => number.as_i64().map(Timestamp::Millis),
        _ => None,
    }
}

/// `parent_ids`：只有 parentId 为真值时才记一条（对齐 Python 的 truthy 判断）。
fn parent_ids(entry: &Value) -> Vec<String> {
    match entry.get("parentId") {
        Some(value) if truthy(value) => vec![python_str(value)],
        _ => Vec::new(),
    }
}

/// 把原生 content 展开成规范 block；`collect_calls` 为真时同时登记工具调用。
///
/// 返回 `(blocks, 本次登记的 (call_id, block 下标))`。
fn content_blocks(
    content: Option<&Value>,
    source_id: &str,
    session: &mut Session,
    collect_calls: bool,
) -> (Vec<Block>, Vec<(String, usize)>) {
    let wrapped;
    let parts: &[Value] = match content {
        Some(Value::String(text)) => {
            let mut part = Map::new();
            part.insert("type".into(), Value::from("text"));
            part.insert("text".into(), Value::from(text.as_str()));
            wrapped = vec![Value::Object(part)];
            &wrapped
        }
        Some(Value::Array(items)) => items,
        _ => &[],
    };
    let mut blocks: Vec<Block> = Vec::new();
    let mut calls: Vec<(String, usize)> = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        if !part.is_object() {
            continue;
        }
        match part.get("type").and_then(Value::as_str) {
            Some("text") => {
                let text = match part.get("text") {
                    Some(value) if truthy(value) => python_str(value),
                    _ => String::new(),
                };
                blocks.push(Block::text(text));
            }
            Some("thinking") => {
                let text = match part.get("thinking") {
                    Some(value) if truthy(value) => python_str(value),
                    _ => String::new(),
                };
                let mut block = Block::new(BlockKind::Thinking);
                block.text = text;
                blocks.push(block);
            }
            Some("toolCall") if collect_calls => {
                let call = call_from_part(part, source_id);
                let call_id = call.source_call_id.clone();
                let mut block = Block::new(BlockKind::Tool);
                block.tool = Some(call);
                blocks.push(block);
                if let Some(call_id) = call_id.filter(|value| !value.is_empty()) {
                    calls.push((call_id, blocks.len() - 1));
                }
            }
            Some("image") => {
                let mime_type = match part.get("mimeType") {
                    Some(value) if truthy(value) => python_str(value),
                    _ => String::new(),
                };
                let asset = image_from_base64(
                    &format!("pi:{source_id}:{index}"),
                    &mime_type,
                    part.get("data").unwrap_or(&Value::Null),
                    None,
                );
                match asset {
                    None => session.lose(
                        "migration.unknown_block_dropped",
                        params(serde_json::json!({
                            "source": "pi", "entry_id": source_id,
                            "block_type": "image", "index": index,
                        })),
                    ),
                    Some(asset) => {
                        let mut block = Block::new(BlockKind::Image);
                        block.image = Some(asset);
                        blocks.push(block);
                    }
                }
            }
            _ => {}
        }
    }
    (blocks, calls)
}

/// `role == "bashExecution"` 的原生消息 → 规范工具调用。
fn bash_call(message: &Value, entry_id: &str) -> ToolCall {
    let output = match message.get("output") {
        Some(value) if truthy(value) => python_str(value),
        _ => String::new(),
    };
    let exit_code = match message.get("exitCode") {
        // Python 显式拒 bool（`isinstance(True, int)` 为真）；serde_json 天然分家。
        Some(Value::Number(number)) => number.as_i64(),
        _ => None,
    };
    let status = if message.get("cancelled").is_some_and(truthy) {
        ToolResultStatus::Interrupted
    } else if !matches!(message.get("exitCode"), None | Some(Value::Null)) && exit_code != Some(0) {
        ToolResultStatus::Error
    } else {
        ToolResultStatus::Success
    };
    let attachments = match message.get("fullOutputPath") {
        Some(value) if truthy(value) => {
            let mut attachment = Map::new();
            attachment.insert("full_output_path".into(), value.clone());
            vec![Value::Object(attachment)]
        }
        _ => Vec::new(),
    };
    let result = ToolResult {
        status,
        blocks: if output.is_empty() {
            Vec::new()
        } else {
            vec![ToolResultBlock::text(output.clone())]
        },
        stdout: Some(output),
        stderr: None,
        exit_code,
        truncated: match message.get("truncated") {
            Some(Value::Bool(flag)) => Some(*flag),
            _ => None,
        },
        attachments,
    };
    let command = match message.get("command") {
        Some(value) if truthy(value) => python_str(value),
        _ => String::new(),
    };
    let mut input = Map::new();
    input.insert("command".into(), Value::from(command));
    let mut call = ToolCall::new(
        "bash",
        Some(CanonicalOp::SHELL_EXEC.to_string()),
        Value::Object(input),
    );
    call.result = Some(result);
    call.source_call_id = Some(format!("bash:{entry_id}"));
    call.source_result_id = Some(entry_id.to_string());
    call.source_message_id = Some(entry_id.to_string());
    call
}

/// 读取一个 pi v3 会话文件。
pub fn read(path: &str) -> DomainResult<Session> {
    let loaded = load(Path::new(path))?;
    let (branch, selected) = active_branch(&loaded.entries);
    let mut session = Session::new(
        "pi",
        loaded.header["id"].as_str().unwrap_or_default(),
        loaded.header["cwd"].as_str().unwrap_or_default(),
    );
    for line in &loaded.malformed {
        session.lose(
            "session.malformed_record",
            params(serde_json::json!({"line": line})),
        );
    }
    let unselected: Vec<Value> = loaded
        .entries
        .iter()
        .filter_map(|entry| entry.get("id").and_then(Value::as_str))
        .filter(|id| !selected.contains(*id))
        .map(Value::from)
        .collect();
    if !unselected.is_empty() {
        session.lose(
            "migration.unknown_block_dropped",
            params(serde_json::json!({
                "source": "pi", "block_type": "inactive_branch",
                "entry_ids": unselected,
            })),
        );
    }

    // call_id → (消息下标, block 下标)；插入序即 Python dict 的迭代序。
    let mut calls: Vec<(String, (usize, usize))> = Vec::new();
    let mut last_message_id: Option<String> = None;
    for entry in branch {
        let kind = entry.get("type").and_then(Value::as_str);
        let id = entry_id(entry).to_string();
        match kind {
            Some("session_info") => {
                if let Some(name) = entry.get("name").and_then(Value::as_str) {
                    session.title = name.to_string();
                }
                continue;
            }
            Some("model_change") => {
                session.model_provider = entry
                    .get("provider")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                session.model = entry
                    .get("modelId")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                continue;
            }
            Some("branch_summary") => {
                let summary = match entry.get("summary") {
                    Some(value) if truthy(value) => python_str(value),
                    _ => String::new(),
                };
                let mut message = Message::new("user");
                message.blocks = vec![Block::text(summary)];
                message.source_id = Some(id.clone());
                message.parent_ids = parent_ids(entry);
                message.created_at = timestamp_of(entry);
                session.messages.push(message);
                last_message_id = Some(id);
                continue;
            }
            Some("compaction") => {
                session.context_compactions.push(compaction_of(
                    entry,
                    &id,
                    &selected,
                    &session,
                    &last_message_id,
                ));
                continue;
            }
            Some("message") => {}
            other => {
                if matches!(other, Some("thinking_level_change") | Some("label")) {
                    continue;
                }
                session.lose(
                    "migration.unknown_block_dropped",
                    params(serde_json::json!({
                        "source": "pi", "entry_id": id,
                        "block_type": other.map_or(Value::Null, Value::from),
                    })),
                );
                continue;
            }
        }

        let empty = Value::Object(Map::new());
        let message_value = match entry.get("message") {
            Some(value) if truthy(value) => value,
            _ => &empty,
        };
        let role = message_value.get("role").and_then(Value::as_str);
        match role {
            Some("bashExecution") => {
                let call = bash_call(message_value, &id);
                let mut block = Block::new(BlockKind::Tool);
                block.tool = Some(call);
                let mut message = Message::new("user");
                message.blocks = vec![block];
                message.source_id = Some(id.clone());
                message.parent_ids = parent_ids(entry);
                message.created_at = timestamp_of(entry);
                session.messages.push(message);
                last_message_id = Some(id);
                continue;
            }
            Some("toolResult") => {
                let call_id = message_value.get("toolCallId").and_then(Value::as_str);
                let located = call_id.and_then(|call_id| {
                    calls
                        .iter()
                        .find(|(key, _)| key == call_id)
                        .map(|(_, position)| *position)
                });
                match located {
                    None => session.lose(
                        "session.orphan_tool_result",
                        params(serde_json::json!({
                            "tool_call_id": call_id.map_or(Value::Null, Value::from),
                        })),
                    ),
                    Some((message_index, block_index)) => {
                        if let Some(call) = session.messages[message_index].blocks[block_index]
                            .tool
                            .as_mut()
                        {
                            call.result = Some(result_from_message(message_value));
                            call.source_result_id = Some(id.clone());
                        }
                    }
                }
                continue;
            }
            Some("user") | Some("assistant") => {}
            other => {
                session.lose(
                    "migration.unknown_block_dropped",
                    params(serde_json::json!({
                        "source": "pi", "entry_id": id,
                        "block_type": format!("message.{}", python_str(
                            &other.map_or(Value::Null, Value::from))),
                    })),
                );
                continue;
            }
        }

        let is_assistant = role == Some("assistant");
        let (blocks, new_calls) = content_blocks(
            message_value.get("content"),
            &id,
            &mut session,
            is_assistant,
        );
        if is_assistant {
            if let Some(provider) = message_value.get("provider").filter(|v| truthy(v)) {
                session.model_provider = Some(python_str(provider));
            }
            if let Some(model) = message_value.get("model").filter(|v| truthy(v)) {
                session.model = Some(python_str(model));
            }
        } else if session.title.is_empty() {
            let text = blocks
                .iter()
                .filter(|block| block.kind == BlockKind::Text)
                .map(|block| block.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let text = text.trim();
            let mut title: String = text.chars().take(80).collect();
            if text.chars().count() > 80 {
                title.push('…');
            }
            session.title = title;
        }
        let mut message = Message::new(role.unwrap_or_default());
        message.blocks = blocks;
        message.source_id = Some(id.clone());
        message.parent_ids = parent_ids(entry);
        message.created_at = timestamp_of(entry);
        session.messages.push(message);
        let message_index = session.messages.len() - 1;
        for (call_id, block_index) in new_calls {
            let position = (message_index, block_index);
            match calls.iter_mut().find(|(key, _)| *key == call_id) {
                // Python dict 覆写同名键：值换掉，位置不变。
                Some(slot) => slot.1 = position,
                None => calls.push((call_id, position)),
            }
        }
        last_message_id = Some(id);
    }

    let unpaired: Vec<String> = calls
        .iter()
        .filter(|(_, (message_index, block_index))| {
            session.messages[*message_index].blocks[*block_index]
                .tool
                .as_ref()
                .is_none_or(|call| call.result.is_none())
        })
        .map(|(call_id, _)| call_id.clone())
        .collect();
    for call_id in unpaired {
        session.lose(
            "session.unpaired_tool_use",
            params(serde_json::json!({"tool_call_id": call_id})),
        );
    }
    Ok(session)
}

fn compaction_of(
    entry: &Value,
    id: &str,
    selected: &HashSet<String>,
    session: &Session,
    last_message_id: &Option<String>,
) -> ContextCompaction {
    let summary = entry.get("summary");
    let first_kept = entry.get("firstKeptEntryId").and_then(Value::as_str);
    let mut compaction = ContextCompaction::new(id, "pi");
    compaction.after_message_id = last_message_id.clone();
    compaction.event_locator = Some(id.to_string());
    compaction.created_at = timestamp_of(entry);
    compaction.summary_status = if summary.is_some_and(truthy) {
        "available".into()
    } else {
        "missing".into()
    };
    compaction.summary_text = match summary {
        Some(value) if truthy(value) => python_str(value),
        _ => String::new(),
    };
    compaction.tail_status = if first_kept.is_some_and(|value| selected.contains(value)) {
        "located".into()
    } else {
        "unknown".into()
    };
    compaction.tail_start_locator = first_kept.map(str::to_string);
    compaction.tail_start_message_index = first_kept.and_then(|first_kept| {
        session
            .messages
            .iter()
            .position(|message| message.source_id.as_deref() == Some(first_kept))
            .map(|index| index as i64 + 1)
    });
    if let Some(tokens) = entry.get("tokensBefore").filter(|v| !v.is_null()) {
        compaction
            .metrics
            .insert("tokens_before".into(), tokens.clone());
    }
    compaction.source_meta.insert(
        "from_hook".into(),
        Value::Bool(entry.get("fromHook").is_some_and(truthy)),
    );
    compaction
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::tool_result_text;
    use serde_json::json;

    fn header() -> Value {
        json!({"type": "session", "version": 3, "id": "s",
               "timestamp": "2026-07-25T00:00:00Z", "cwd": "/private/raw"})
    }

    fn message(id: &str, parent: Value, role: &str, content: Value) -> Value {
        json!({"type": "message", "id": id, "parentId": parent,
               "timestamp": format!("2026-07-25T00:00:0{}Z", id.len()),
               "message": {"role": role, "content": content,
                           "timestamp": 1784937600000i64}})
    }

    fn write(root: &Path, name: &str, records: &[Value], tail: &str) -> String {
        let path = root.join(name);
        let body = records
            .iter()
            .map(|record| serde_json::to_string(record).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&path, format!("{body}{tail}")).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn pairs_parallel_tools_images_thinking_and_missing_result() {
        let root = tempfile::tempdir().unwrap();
        let mut result = message("r", json!("a"), "toolResult", json!([]));
        result["message"] = json!({
            "role": "toolResult", "toolCallId": "c1", "toolName": "bash",
            "isError": false,
            "content": [{"type": "text", "text": "/Users/raw\n"},
                        {"type": "image", "data": "AA==", "mimeType": "image/png"}],
        });
        let path = write(
            root.path(),
            "tools.jsonl",
            &[
                header(),
                message(
                    "u",
                    Value::Null,
                    "user",
                    json!([{"type": "text", "text": "/Users/raw sk-test-token"},
                           {"type": "image", "data": "iVBORw0KGgo=", "mimeType": "image/png"}]),
                ),
                message(
                    "a",
                    json!("u"),
                    "assistant",
                    json!([{"type": "thinking", "thinking": "parallel"},
                           {"type": "toolCall", "id": "c1", "name": "bash",
                            "arguments": {"command": "pwd", "timeout": 3}},
                           {"type": "toolCall", "id": "c2", "name": "read",
                            "arguments": {"path": "/Users/raw/a.txt"}}]),
                ),
                result,
            ],
            "",
        );

        let session = read(&path).unwrap();
        let tools: Vec<&ToolCall> = session
            .messages
            .iter()
            .flat_map(|message| message.blocks.iter())
            .filter_map(|block| block.tool.as_ref())
            .collect();
        let ids: Vec<&str> = tools
            .iter()
            .map(|tool| tool.source_call_id.as_deref().unwrap())
            .collect();
        assert_eq!(ids, ["c1", "c2"]);
        assert_eq!(
            tools[0].input,
            json!({"command": "pwd", "timeout_ms": 3000})
        );
        assert_eq!(tool_result_text(tools[0].result.as_ref()), "/Users/raw\n");
        assert!(tools[1].result.is_none());
        assert!(session.messages[0]
            .blocks
            .iter()
            .any(|block| block.kind == BlockKind::Image));
        assert!(session
            .loss
            .iter()
            .any(|loss| loss.code == "session.unpaired_tool_use"));
    }

    #[test]
    fn preserves_assistant_content_order() {
        let root = tempfile::tempdir().unwrap();
        let path = write(
            root.path(),
            "order.jsonl",
            &[
                header(),
                message("u", Value::Null, "user", json!("go")),
                message(
                    "a",
                    json!("u"),
                    "assistant",
                    json!([{"type": "text", "text": "before"},
                           {"type": "toolCall", "id": "c", "name": "read",
                            "arguments": {"path": "/raw"}},
                           {"type": "text", "text": "after"}]),
                ),
            ],
            "",
        );
        let session = read(&path).unwrap();
        let kinds: Vec<BlockKind> = session
            .messages
            .last()
            .unwrap()
            .blocks
            .iter()
            .map(|block| block.kind)
            .collect();
        assert_eq!(kinds, [BlockKind::Text, BlockKind::Tool, BlockKind::Text]);
    }

    #[test]
    fn selects_last_leaf_branch_and_reports_inactive_entries() {
        let root = tempfile::tempdir().unwrap();
        let path = write(
            root.path(),
            "branch.jsonl",
            &[
                header(),
                message("u", Value::Null, "user", json!("root")),
                message(
                    "dead",
                    json!("u"),
                    "assistant",
                    json!([{"type": "text", "text": "dead"}]),
                ),
                message(
                    "live",
                    json!("u"),
                    "assistant",
                    json!([{"type": "text", "text": "live"}]),
                ),
            ],
            "",
        );
        let session = read(&path).unwrap();
        let ids: Vec<&str> = session
            .messages
            .iter()
            .map(|message| message.source_id.as_deref().unwrap())
            .collect();
        assert_eq!(ids, ["u", "live"]);
        assert_eq!(
            session.loss.last().unwrap().params["entry_ids"],
            json!(["dead"])
        );
    }

    #[test]
    fn ignores_bad_tail_but_reports_bad_middle() {
        let root = tempfile::tempdir().unwrap();
        let records = [header(), message("u", Value::Null, "user", json!("kept"))];
        let path = write(root.path(), "tail.jsonl", &records, "\n{broken");
        let session = read(&path).unwrap();
        assert_eq!(
            session
                .messages
                .iter()
                .map(|message| message.source_id.as_deref().unwrap())
                .collect::<Vec<_>>(),
            ["u"]
        );
        assert!(session.loss.is_empty());

        fs::write(
            &path,
            format!(
                "{}\n{{broken\n{}",
                serde_json::to_string(&records[0]).unwrap(),
                serde_json::to_string(&records[1]).unwrap()
            ),
        )
        .unwrap();
        let session = read(&path).unwrap();
        assert_eq!(session.loss[0].code, "session.malformed_record");
        assert_eq!(session.loss[0].params["line"], json!(2));
    }

    #[test]
    fn rejects_non_v3_header() {
        let root = tempfile::tempdir().unwrap();
        let mut old = header();
        old["version"] = json!(2);
        let path = write(root.path(), "old.jsonl", &[old], "");
        let error = read(&path).unwrap_err();
        assert_eq!(error.code, "agent.format_changed");
    }

    #[test]
    fn maps_bash_execution_message() {
        let root = tempfile::tempdir().unwrap();
        let path = write(
            root.path(),
            "bash.jsonl",
            &[
                header(),
                message("u", Value::Null, "user", json!("run it")),
                json!({"type": "message", "id": "b", "parentId": "u",
                       "timestamp": "2026-07-25T00:00:02Z",
                       "message": {"role": "bashExecution", "command": "pwd",
                                   "output": "/private/raw\n", "exitCode": 0,
                                   "cancelled": false, "truncated": true,
                                   "fullOutputPath": "/private/raw/full.txt",
                                   "timestamp": 2}}),
            ],
            "",
        );
        let session = read(&path).unwrap();
        let tool = session.messages.last().unwrap().blocks[0]
            .tool
            .as_ref()
            .unwrap();
        assert_eq!(tool.op.as_deref(), Some(CanonicalOp::SHELL_EXEC));
        assert_eq!(tool.input, json!({"command": "pwd"}));
        let result = tool.result.as_ref().unwrap();
        assert_eq!(result.stdout.as_deref(), Some("/private/raw\n"));
        assert_eq!(result.truncated, Some(true));
        assert_eq!(
            result.attachments,
            vec![json!({"full_output_path": "/private/raw/full.txt"})]
        );
        assert_eq!(result.status, ToolResultStatus::Success);
    }

    #[test]
    fn bash_status_follows_cancelled_then_exit_code() {
        let cancelled = bash_call(&json!({"cancelled": true, "exitCode": 1}), "b");
        assert_eq!(
            cancelled.result.unwrap().status,
            ToolResultStatus::Interrupted
        );
        let failed = bash_call(&json!({"exitCode": 2}), "b");
        assert_eq!(failed.result.unwrap().status, ToolResultStatus::Error);
        // exitCode 缺席/为 null 都算成功。
        let unknown = bash_call(&json!({"output": "x"}), "b");
        assert_eq!(unknown.result.unwrap().status, ToolResultStatus::Success);
    }

    #[test]
    fn cycles_in_the_parent_chain_terminate() {
        let entries = vec![
            json!({"type": "message", "id": "a", "parentId": "b"}),
            json!({"type": "message", "id": "b", "parentId": "a"}),
        ];
        let (branch, seen) = active_branch(&entries);
        assert_eq!(branch.len(), 2);
        assert_eq!(seen.len(), 2);
    }
}
