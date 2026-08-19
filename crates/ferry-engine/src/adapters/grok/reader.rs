//! 把当前 Grok 的 ACP/update bundle 读成 canonical session。
//!
//! 读取管线：`filter_rewind_updates` → `aggregate_updates` → canonical；
//! 只有在完全没有 updates 时才回退到 `chat_history.jsonl`。

use std::collections::HashMap;
use std::path::Path;

use serde_json::{Map, Value};

use crate::adapters::shared::dialect::python_str;
use crate::errors::DomainResult;
use crate::jsonutil::canonical_json;
use crate::model::{
    Block, BlockKind, ContextCompaction, Message, Session, ToolCall, ToolResult, ToolResultBlock,
    ToolResultBlockKind, ToolResultStatus,
};
use crate::tool_ops::CanonicalOp;

use super::dialect::DIALECT;
use super::rewind::filter_rewind_updates;
use super::store::{load_grok_bundle, GrokBundle};
use super::updates::{aggregate_updates, PromptBlock, PromptTool};

/// Grok 把命令输出编码成字节数组（`[97, 110, ...]`），解回 UTF-8 文本。
fn bytes_text(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::Array(items)) => {
            if items.is_empty() {
                return Some(String::new());
            }
            let bytes: Option<Vec<u8>> = items
                .iter()
                .map(|item| {
                    item.as_u64()
                        .filter(|_| !item.is_boolean())
                        .filter(|byte| *byte < 256)
                        .map(|byte| byte as u8)
                })
                .collect();
            bytes.map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        }
        Some(Value::String(text)) => Some(text.clone()),
        _ => None,
    }
}

/// `rawOutput` 类型信封 → `(语义文本, 元数据)`；解不开返回 `(None, {})`。
///
/// 信封里除文本外只有回显输入或传输元数据，拆出文本不构成信息损失，却能让结果
/// 以原生 text 块迁移而不是 json 投影。
fn unwrap_output(value: &Value) -> (Option<String>, Map<String, Value>) {
    let empty = Map::new();
    let Some(entries) = value.as_object() else {
        return (None, empty);
    };
    let kind = entries.get("type").and_then(Value::as_str);
    let nested_string = |outer: &str, inner: &str| -> Option<String> {
        entries.get(outer)?.get(inner)?.as_str().map(str::to_string)
    };
    match kind {
        Some("ReadFile") => match nested_string("FileContent", "content") {
            Some(text) => (Some(text), empty),
            None => (None, empty),
        },
        Some("ListDir") => match nested_string("Content", "content") {
            Some(text) => (Some(text), empty),
            None => (None, empty),
        },
        Some("Text") => match entries.get("text").and_then(Value::as_str) {
            Some(text) => (Some(text.to_string()), empty),
            None => (None, empty),
        },
        Some("Todo") => match nested_string("TodosUpdated", "summary_for_prompt") {
            Some(text) => (Some(text), empty),
            None => (None, empty),
        },
        // EditsApplied 只回显 old/new 输入，编辑成功本身没有输出文本。
        Some("SearchReplace") if entries.get("EditsApplied").is_some_and(Value::is_object) => {
            (Some(String::new()), empty)
        }
        Some("Bash") => {
            let text = bytes_text(entries.get("output")).or_else(|| {
                entries
                    .get("output_for_prompt")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            });
            match text {
                None => (None, empty),
                Some(text) => {
                    let mut meta = Map::new();
                    if let Some(code) = plain_int(entries.get("exit_code")) {
                        meta.insert("exit_code".into(), Value::from(code));
                    }
                    if let Some(Value::Bool(truncated)) = entries.get("truncated") {
                        meta.insert("truncated".into(), Value::Bool(*truncated));
                    }
                    (Some(text), meta)
                }
            }
        }
        Some("GrepSearch") => match bytes_text(entries.get("stdout")) {
            None => (None, empty),
            Some(text) => {
                let mut meta = Map::new();
                if let Some(code) = plain_int(entries.get("exit_code")) {
                    meta.insert("exit_code".into(), Value::from(code));
                }
                if let Some(stderr) = bytes_text(entries.get("stderr")).filter(|s| !s.is_empty()) {
                    meta.insert("stderr".into(), Value::from(stderr));
                }
                (Some(text), meta)
            }
        },
        _ => (None, empty),
    }
}

/// `isinstance(value, int) and not isinstance(value, bool)`。
fn plain_int(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    if value.is_boolean() {
        return None;
    }
    value.as_i64()
}

/// 由 rawOutput 与聚合状态构造 canonical 工具结果。
fn tool_result(value: &Value, status: &str) -> ToolResult {
    let (text, meta) = unwrap_output(value);
    let blocks = match (text, value) {
        (Some(text), _) if text.is_empty() => Vec::new(),
        (Some(text), _) => vec![ToolResultBlock::text(text)],
        (None, Value::String(text)) => vec![ToolResultBlock::text(text.as_str())],
        (None, Value::Null) => Vec::new(),
        (None, other) => {
            let mut block = ToolResultBlock::new(ToolResultBlockKind::Json);
            block.data = other.clone();
            vec![block]
        }
    };
    let mut mapped = match status {
        "completed" => ToolResultStatus::Success,
        "failed" => ToolResultStatus::Error,
        "pending" => ToolResultStatus::Pending,
        _ => ToolResultStatus::Unknown,
    };
    let exit_code = meta.get("exit_code").and_then(Value::as_i64);
    if mapped == ToolResultStatus::Success && exit_code.is_some_and(|code| code != 0) {
        mapped = ToolResultStatus::Error;
    }
    ToolResult {
        status: mapped,
        blocks,
        stdout: None,
        stderr: meta
            .get("stderr")
            .and_then(Value::as_str)
            .map(str::to_string),
        exit_code,
        truncated: meta.get("truncated").and_then(Value::as_bool),
        attachments: Vec::new(),
    }
}

/// Grok 的 arguments 可能是 dict 也可能是 JSON 字符串，先解包再归一。
fn normalize(name: &str, raw: &Value) -> (Option<String>, Value) {
    let decoded = match raw {
        Value::String(text) => serde_json::from_str::<Value>(text).unwrap_or_else(|_| raw.clone()),
        other => other.clone(),
    };
    match DIALECT.parse(name, &decoded) {
        Some((op, value)) => (Some(op.to_string()), value),
        None => {
            let mut fallback = Map::new();
            fallback.insert("namespace".into(), Value::from("grok"));
            fallback.insert("name".into(), Value::from(name));
            fallback.insert("input".into(), raw.clone());
            (
                Some(CanonicalOp::TOOL_INVOKE.to_string()),
                Value::Object(fallback),
            )
        }
    }
}

fn tool_call(data: &PromptTool) -> ToolCall {
    let (op, input) = normalize(&data.name, &data.input);
    let mut call = ToolCall::new(data.name.as_str(), op, input);
    call.result = data
        .output
        .as_ref()
        .map(|output| tool_result(output, &data.status));
    call.source_call_id = Some(data.id.clone());
    call
}

/// 没有 updates 时的回退路径：直接读 `chat_history.jsonl` v1 行。
fn chat_messages(bundle: &GrokBundle, session: &mut Session) {
    // tool_call_id → (消息下标, 块下标)。Python 持有对象引用，Rust 走坐标回写。
    let mut calls: HashMap<String, (usize, usize)> = HashMap::new();
    for (index, row) in bundle.chat.iter().enumerate() {
        let role = row
            .get("type")
            .or_else(|| row.get("role"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let source_id = match row.get("id") {
            Some(value) if truthy(value) => python_str(value),
            _ => format!("chat:{index}"),
        };
        match role {
            "user" => {
                let content = row.get("content");
                let parts: Vec<Value> = match content {
                    Some(Value::Array(items)) => items.clone(),
                    other => {
                        let text = other.filter(|value| truthy(value)).map(python_str);
                        vec![serde_json::json!({
                            "type": "text", "text": text.unwrap_or_default(),
                        })]
                    }
                };
                let blocks = parts
                    .iter()
                    .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
                    .map(|part| {
                        let text = part
                            .get("text")
                            .filter(|value| truthy(value))
                            .map(python_str);
                        Block::text(text.unwrap_or_default())
                    })
                    .collect();
                let mut message = Message::new("user");
                message.blocks = blocks;
                message.source_id = Some(source_id);
                session.messages.push(message);
            }
            "assistant" => {
                let text = row
                    .get("content")
                    .filter(|value| truthy(value))
                    .map(python_str)
                    .unwrap_or_default();
                let mut message = Message::new("assistant");
                message.source_id = Some(source_id);
                message.blocks.push(Block::text(text));
                if let Some(natives) = row.get("tool_calls").and_then(Value::as_array) {
                    for native in natives {
                        let name = native
                            .get("name")
                            .filter(|value| truthy(value))
                            .map(python_str)
                            .unwrap_or_else(|| "tool".to_string());
                        let arguments = native
                            .get("arguments")
                            .filter(|value| truthy(value))
                            .cloned()
                            .unwrap_or_else(|| Value::Object(Map::new()));
                        let (op, input) = normalize(&name, &arguments);
                        let mut call = ToolCall::new(name.as_str(), op, input);
                        let raw_id = native.get("id").cloned().unwrap_or(Value::Null);
                        call.source_call_id = match &raw_id {
                            Value::Null => None,
                            other => Some(python_str(other)),
                        };
                        let mut block = Block::new(BlockKind::Tool);
                        block.tool = Some(call);
                        message.blocks.push(block);
                        calls.insert(
                            canonical_json(&raw_id).unwrap_or_default(),
                            (session.messages.len(), message.blocks.len() - 1),
                        );
                    }
                }
                session.messages.push(message);
            }
            "reasoning" => {
                let content = row.get("content").filter(|value| truthy(value));
                let Some(content) = content else { continue };
                if let Some(message) = session.messages.last_mut() {
                    if message.role == "assistant" {
                        let mut block = Block::new(BlockKind::Thinking);
                        block.text = python_str(content);
                        message.blocks.push(block);
                    }
                }
            }
            "tool_result" => {
                let key = canonical_json(row.get("tool_call_id").unwrap_or(&Value::Null))
                    .unwrap_or_default();
                if let Some((message_index, block_index)) = calls.get(&key).copied() {
                    let content = row.get("content").cloned().unwrap_or(Value::Null);
                    if let Some(call) = session.messages[message_index].blocks[block_index]
                        .tool
                        .as_mut()
                    {
                        call.result = Some(tool_result(&content, "completed"));
                    }
                }
            }
            _ => {}
        }
    }
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

/// 读一个 bundle 目录成 canonical session（不装配子会话树）。
pub fn read(path: &Path) -> DomainResult<Session> {
    let bundle = load_grok_bundle(path)?;
    let summary = &bundle.summary;
    let title = summary
        .get("generated_title")
        .filter(|value| truthy(value))
        .or_else(|| summary.get("session_summary").filter(|value| truthy(value)))
        .map(python_str)
        .unwrap_or_default();
    let mut session = Session::new("grok", bundle.session_id(), bundle.cwd());
    session.title = title;
    session.parent_id = summary
        .get("parent_session_id")
        .filter(|value| !value.is_null())
        .map(python_str);
    session.model = summary
        .get("current_model_id")
        .filter(|value| !value.is_null())
        .map(python_str);
    for diagnostic in &bundle.diagnostics {
        session.lose("session.malformed_record", diagnostic.clone());
    }
    if bundle.updates.is_empty() {
        chat_messages(&bundle, &mut session);
        return Ok(session);
    }
    for prompt in aggregate_updates(&filter_rewind_updates(&bundle.updates)) {
        if !prompt.user.is_empty() {
            let mut message = Message::new("user");
            message.blocks.push(Block::text(prompt.user.concat()));
            message.source_id = Some(format!("{}:user", prompt.id));
            session.messages.push(message);
        }
        let mut blocks: Vec<Block> = Vec::new();
        for item in &prompt.blocks {
            match item {
                PromptBlock::Tool(call_id) => {
                    let Some(data) = prompt.tool(call_id) else {
                        continue;
                    };
                    let mut block = Block::new(BlockKind::Tool);
                    block.tool = Some(tool_call(data));
                    blocks.push(block);
                }
                PromptBlock::Text(text) if !text.is_empty() => blocks.push(Block::text(text)),
                PromptBlock::Thinking(text) if !text.is_empty() => {
                    let mut block = Block::new(BlockKind::Thinking);
                    block.text = text.clone();
                    blocks.push(block);
                }
                _ => {}
            }
        }
        if !blocks.is_empty() {
            let mut message = Message::new("assistant");
            message.blocks = blocks;
            message.source_id = Some(format!("{}:assistant", prompt.id));
            session.messages.push(message);
        }
        if let Some(compaction) = prompt.compaction.as_ref() {
            let id = compaction
                .get("id")
                .filter(|value| truthy(value))
                .map(python_str)
                .unwrap_or_else(|| format!("{}:compaction", prompt.id));
            let mut record = ContextCompaction::new(id, "grok");
            record.after_message_id = session
                .messages
                .last()
                .and_then(|message| message.source_id.clone());
            let summary_text = compaction.get("summary").filter(|value| truthy(value));
            record.summary_status = if summary_text.is_some() {
                "available".to_string()
            } else {
                "missing".to_string()
            };
            record.summary_text = summary_text.map(python_str).unwrap_or_default();
            if let Some(tokens) = compaction
                .get("tokensBefore")
                .filter(|value| !value.is_null())
            {
                record
                    .metrics
                    .insert("tokens_before".into(), tokens.clone());
            }
            session.context_compactions.push(record);
        }
        if !prompt.unknown.is_empty() {
            let mut params = Map::new();
            params.insert("source".into(), Value::from("grok"));
            params.insert("block_type".into(), Value::from("session_update"));
            params.insert("count".into(), Value::from(prompt.unknown.len() as i64));
            session.lose("migration.unknown_block_dropped", params);
        }
    }
    Ok(session)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    fn bundle(
        root: &Path,
        summary: Value,
        updates: &[Value],
        chat: &[Value],
    ) -> std::path::PathBuf {
        let path = root.join("bundle");
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("summary.json"), summary.to_string()).unwrap();
        let dump = |rows: &[Value]| {
            rows.iter()
                .map(|row| row.to_string())
                .collect::<Vec<_>>()
                .join("\n")
                + "\n"
        };
        if !updates.is_empty() {
            fs::write(path.join("updates.jsonl"), dump(updates)).unwrap();
        }
        if !chat.is_empty() {
            fs::write(path.join("chat_history.jsonl"), dump(chat)).unwrap();
        }
        path
    }

    fn summary(id: &str) -> Value {
        json!({"info": {"id": id, "cwd": "/w"}, "chat_format_version": 1,
               "generated_title": "T", "current_model_id": "grok-code-fast-1"})
    }

    #[test]
    fn bash_envelopes_unwrap_to_text_with_exit_code_and_truncation() {
        let output = json!({"type": "Bash", "output": [104, 105],
                            "exit_code": 2, "truncated": true});
        let result = tool_result(&output, "completed");
        // exit_code 非零把 success 纠成 error。
        assert_eq!(result.status, ToolResultStatus::Error);
        assert_eq!(result.blocks[0].text, "hi");
        assert_eq!(result.exit_code, Some(2));
        assert_eq!(result.truncated, Some(true));
    }

    #[test]
    fn grep_envelopes_carry_stderr() {
        let output = json!({"type": "GrepSearch", "stdout": "hit\n",
                            "stderr": [98, 97, 100], "exit_code": 0});
        let result = tool_result(&output, "completed");
        assert_eq!(result.status, ToolResultStatus::Success);
        assert_eq!(result.blocks[0].text, "hit\n");
        assert_eq!(result.stderr.as_deref(), Some("bad"));
    }

    #[test]
    fn unknown_envelopes_fall_back_to_a_json_block() {
        let result = tool_result(&json!({"weird": 1}), "failed");
        assert_eq!(result.status, ToolResultStatus::Error);
        assert_eq!(result.blocks[0].kind, ToolResultBlockKind::Json);
        assert_eq!(result.blocks[0].data, json!({"weird": 1}));
        // SearchReplace 的空输出不产出任何块。
        let applied = json!({"type": "SearchReplace", "EditsApplied": {"old": "a"}});
        assert!(tool_result(&applied, "completed").blocks.is_empty());
    }

    #[test]
    fn json_string_arguments_are_decoded_before_normalisation() {
        let (op, input) = normalize("read_file", &json!("{\"target_file\": \"/a\"}"));
        assert_eq!(op.as_deref(), Some(CanonicalOp::FS_READ));
        assert_eq!(input, json!({"file_path": "/a"}));
        // 无法归一时保留**原始**入参（不是解码后的）。
        let (op, input) = normalize("mystery", &json!("raw text"));
        assert_eq!(op.as_deref(), Some(CanonicalOp::TOOL_INVOKE));
        assert_eq!(
            input,
            json!({"namespace": "grok", "name": "mystery", "input": "raw text"})
        );
    }

    #[test]
    fn updates_win_over_chat_history() {
        let root = tempfile::tempdir().unwrap();
        let path = bundle(
            root.path(),
            summary("s1"),
            &[json!({"method": "session/update", "params": {
                "update": {"sessionUpdate": "user_message_chunk",
                           "content": {"type": "text", "text": "from-updates"}},
                "_meta": {"promptId": "p1"}}})],
            &[json!({"type": "user", "id": "u1", "content": "from-chat"})],
        );
        let session = read(&path).unwrap();
        assert_eq!(session.messages[0].blocks[0].text, "from-updates");
        assert_eq!(session.messages[0].source_id.as_deref(), Some("p1:user"));
    }

    #[test]
    fn chat_fallback_pairs_tool_results_by_call_id() {
        let root = tempfile::tempdir().unwrap();
        let path = bundle(
            root.path(),
            summary("s2"),
            &[],
            &[
                json!({"type": "user", "id": "u1",
                       "content": [{"type": "text", "text": "go"}]}),
                json!({"type": "assistant", "id": "a1", "content": "ok",
                       "tool_calls": [{"id": "t1", "name": "read_file",
                                       "arguments": {"target_file": "/a"}}]}),
                json!({"type": "reasoning", "content": "because"}),
                json!({"type": "tool_result", "id": "r1", "tool_call_id": "t1",
                       "content": "file body"}),
            ],
        );
        let session = read(&path).unwrap();
        assert_eq!(session.messages.len(), 2);
        let blocks = &session.messages[1].blocks;
        assert_eq!(blocks[0].text, "ok");
        let call = blocks[1].tool.as_ref().unwrap();
        assert_eq!(call.op.as_deref(), Some(CanonicalOp::FS_READ));
        assert_eq!(call.result.as_ref().unwrap().blocks[0].text, "file body");
        // reasoning 追加在 assistant 消息尾部。
        assert_eq!(blocks[2].kind, BlockKind::Thinking);
        assert_eq!(blocks[2].text, "because");
    }

    #[test]
    fn compaction_and_unknown_updates_land_in_the_session() {
        let root = tempfile::tempdir().unwrap();
        let path = bundle(
            root.path(),
            summary("s3"),
            &[
                json!({"method": "session/update", "params": {
                    "update": {"sessionUpdate": "agent_message_chunk",
                               "content": {"type": "text", "text": "hi"}},
                    "_meta": {"promptId": "p1"}}}),
                json!({"method": "session/update", "params": {
                    "update": {"kind": "compaction", "summary": "sum",
                               "tokensBefore": 42},
                    "_meta": {"promptId": "p1"}}}),
                json!({"method": "session/update", "params": {
                    "update": {"kind": "mystery"}, "_meta": {"promptId": "p1"}}}),
            ],
            &[],
        );
        let session = read(&path).unwrap();
        let compaction = &session.context_compactions[0];
        assert_eq!(compaction.summary_status, "available");
        assert_eq!(compaction.summary_text, "sum");
        assert_eq!(compaction.metrics["tokens_before"], json!(42));
        assert_eq!(compaction.after_message_id.as_deref(), Some("p1:assistant"));
        assert_eq!(session.loss[0].code, "migration.unknown_block_dropped");
        assert_eq!(session.loss[0].params["count"], json!(1));
    }

    #[test]
    fn malformed_records_become_session_losses() {
        let root = tempfile::tempdir().unwrap();
        let path = bundle(root.path(), summary("s4"), &[], &[]);
        fs::write(
            path.join("updates.jsonl"),
            "{broken\n{\"method\": \"session/update\", \"params\": {\"update\": \
             {\"sessionUpdate\": \"agent_message_chunk\", \"content\": \
             {\"type\": \"text\", \"text\": \"x\"}}, \"_meta\": {\"promptId\": \"p\"}}}\n",
        )
        .unwrap();
        let session = read(&path).unwrap();
        assert_eq!(session.loss[0].code, "session.malformed_record");
        assert_eq!(session.loss[0].params["line"], json!(1));
        assert_eq!(session.loss[0].params["reason"], json!("invalid_json"));
    }
}
