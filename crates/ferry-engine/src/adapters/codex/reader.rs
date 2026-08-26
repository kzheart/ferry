//! Codex reader：rollout JSONL → canonical session model。
//!
//! Rollout 是 append-only 的 JSONL。解析器把「字节偏移 + 解析器状态」一起缓存：
//! 文件追加时只解析新尾部并增量并入已有 Session。任何前缀不一致（inode 更换、
//! 截断、尾部窗口比对失败）都会回退全量重解析。
//!
//! 与 Python 的一处刻意差异：Python 的 `view()` 直接在缓存对象上追加合成消息，
//! 再靠 `snapshot()/restore()` 撤销；Rust 的 [`RolloutParser::view`] 返回**克隆**，
//! 缓存对象始终保持「纯解析状态」，因此不需要 baseline 快照。对外可观察行为
//! 一致，且不会被树装配的副作用污染缓存。

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde_json::{Map, Value};

use crate::adapters::shared::media::image_from_data_url;
use crate::errors::{DomainError, DomainResult};
use crate::model::{Block, BlockKind, ContextCompaction, Message, Session, Timestamp, ToolCall};
use crate::system::paths::expanduser;
use crate::tool_ops::CanonicalOp;

/// `sessions::reasoning::visible_text` 的本地副本。
///
/// adapters 不得引用 `crate::sessions`（见 `adapters/mod.rs` 的分层规则），
/// 而这份判定只有三行，复制比反转依赖更省事。
fn visible_text(value: &Value) -> Option<&str> {
    value.as_str().filter(|text| !text.trim().is_empty())
}

use super::{tool_calls, tool_results, topology};

const SKIP_USER_PREFIX: [&str; 4] = [
    "<environment_context>",
    "<user_instructions>",
    "<ENVIRONMENT_CONTEXT>",
    "<turn_aborted>",
];

/// `response_item.payload.type` 的合法取值集合；出现在**记录级** type 上即为
/// 格式漂移。
const RESPONSE_PAYLOAD_TYPES: [&str; 6] = [
    "message",
    "reasoning",
    "function_call",
    "function_call_output",
    "custom_tool_call",
    "custom_tool_call_output",
];

const MALFORMED_JSONL: &str = "__ferry_malformed_jsonl__";
const MALFORMED_RECORD: &str = "__ferry_malformed_record__";

/// 增量前提被打破（前缀变化 / 迟到的 session_meta），需要全量重解析。
#[derive(Debug)]
struct RestartParse;

/// 从 Codex 私有的 `reasoning.summary` 结构提取可读摘要。
fn summary_text(payload: &Map<String, Value>) -> Option<String> {
    let summary = payload.get("summary").filter(|value| !value.is_null());
    match summary {
        None => None,
        Some(Value::String(_)) => {
            visible_text(summary.expect("已判定为字符串")).map(str::to_string)
        }
        Some(Value::Array(items)) => {
            let mut parts = Vec::new();
            for item in items {
                match item {
                    Value::Object(entry) => {
                        if let Some(text) = entry.get("text").and_then(Value::as_str) {
                            if !text.trim().is_empty() {
                                parts.push(text.to_string());
                            }
                        }
                    }
                    Value::String(text) if !text.trim().is_empty() => parts.push(text.clone()),
                    _ => {}
                }
            }
            if parts.is_empty() {
                None
            } else {
                Some(parts.join("\n"))
            }
        }
        // 空数组 / 空串在 Python 里被 `or []` 归一成 []，再走 list 分支。
        Some(_) => None,
    }
}

/// 可安全消费的字节数：最后一个换行之后若是完整 JSON 也一并消费。
fn complete_span(data: &[u8]) -> usize {
    let end = data
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index + 1);
    let tail = &data[end..];
    if tail.is_empty() {
        return end;
    }
    match serde_json::from_slice::<Value>(tail) {
        Ok(_) => data.len(),
        Err(_) => end,
    }
}

/// 把完整字节段切成记录；返回 `(records, 行数)`。
fn batch_records(chunk: &[u8], start_line: usize) -> (Vec<Value>, usize) {
    let mut lines: Vec<&[u8]> = chunk.split(|byte| *byte == b'\n').collect();
    if chunk.ends_with(b"\n") {
        lines.pop();
    }
    let total = lines.len();
    let mut records = Vec::with_capacity(total);
    for (offset, raw) in lines.into_iter().enumerate() {
        let line_number = start_line + offset;
        if raw.iter().all(|byte| byte.is_ascii_whitespace()) {
            continue;
        }
        match serde_json::from_slice::<Value>(raw) {
            Ok(Value::Object(entries)) => records.push(Value::Object(entries)),
            Ok(_) => {
                records.push(malformed(
                    MALFORMED_RECORD,
                    line_number,
                    "record is not an object",
                ));
            }
            Err(error) => {
                records.push(malformed(MALFORMED_JSONL, line_number, &python_msg(&error)));
            }
        }
    }
    (records, total)
}

/// serde 的错误串里带行列信息，Python 的 `error.msg` 只有原因短语。
fn python_msg(error: &serde_json::Error) -> String {
    let text = error.to_string();
    match text.find(" at line ") {
        Some(index) => text[..index].to_string(),
        None => text,
    }
}

fn malformed(kind: &str, line_number: usize, error: &str) -> Value {
    let mut record = Map::new();
    record.insert("type".into(), Value::from(kind));
    record.insert("line_number".into(), Value::from(line_number as i64));
    record.insert("error".into(), Value::from(error));
    Value::Object(record)
}

fn codex_compaction(
    record: &Map<String, Value>,
    ordinal: usize,
    after_message_id: Option<String>,
) -> ContextCompaction {
    let empty = Map::new();
    let payload = record
        .get("payload")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let summary = payload
        .get("message")
        .and_then(Value::as_str)
        .map(|text| text.trim().to_string())
        .unwrap_or_default();
    let replacement = payload
        .get("replacement_history")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let encrypted = replacement.iter().any(|item| {
        item.as_object().is_some_and(|entry| {
            entry.get("type").and_then(Value::as_str) == Some("compaction")
                && entry
                    .get("encrypted_content")
                    .and_then(Value::as_str)
                    .is_some_and(|text| !text.is_empty())
        })
    });
    let window_id = payload.get("window_id").cloned().unwrap_or(Value::Null);
    // `str(window_id or f"record:{ordinal}")`：falsy 的 window_id 退回记录序号。
    let id = if truthy(&window_id) {
        crate::adapters::shared::dialect::python_str(&window_id)
    } else {
        format!("record:{ordinal}")
    };
    let mut compaction = ContextCompaction::new(id, "codex");
    compaction.after_message_id = after_message_id;
    compaction.event_locator = Some(format!("record:{ordinal}"));
    compaction.created_at = timestamp_of(record.get("timestamp"));
    compaction.state = "completed".to_string();
    compaction.summary_status = if !summary.is_empty() {
        "available".to_string()
    } else if encrypted {
        "protected".to_string()
    } else {
        "missing".to_string()
    };
    compaction.summary_text = summary;
    let mut meta = Map::new();
    meta.insert(
        "replacement_history_present".into(),
        Value::Bool(!replacement.is_empty()),
    );
    meta.insert(
        "replacement_item_count".into(),
        Value::from(replacement.len() as i64),
    );
    meta.insert(
        "window_number".into(),
        payload.get("window_number").cloned().unwrap_or(Value::Null),
    );
    meta.insert(
        "first_window_id".into(),
        payload
            .get("first_window_id")
            .cloned()
            .unwrap_or(Value::Null),
    );
    meta.insert(
        "previous_window_id".into(),
        payload
            .get("previous_window_id")
            .cloned()
            .unwrap_or(Value::Null),
    );
    meta.insert("window_id".into(), window_id);
    compaction.source_meta = meta;
    compaction
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

fn timestamp_of(value: Option<&Value>) -> Option<Timestamp> {
    match value {
        Some(Value::String(text)) => Some(Timestamp::Text(text.clone())),
        Some(Value::Number(number)) => number.as_i64().map(Timestamp::Millis),
        _ => None,
    }
}

/// 待配对工具调用的位置：尚未落盘的暂存区，或已并入的消息。
#[derive(Clone, Copy, Debug)]
enum ToolSlot {
    Pending(usize),
    Message(usize, usize),
}

/// 可增量喂入记录批次的 rollout 解析器。
struct RolloutParser {
    path: PathBuf,
    session: Option<Session>,
    saw_meta: bool,
    pending: HashMap<String, ToolSlot>,
    current_tools: Vec<Block>,
    current_reasoning: Vec<Block>,
    ordinal: usize,
    line_count: usize,
    // 增量缓存簿记
    offset: u64,
    node: Option<(u64, u64)>,
    mtime_ns: i128,
    size: u64,
    window: Vec<u8>,
}

impl RolloutParser {
    fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            session: None,
            saw_meta: false,
            pending: HashMap::new(),
            current_tools: Vec::new(),
            current_reasoning: Vec::new(),
            ordinal: 0,
            line_count: 0,
            offset: 0,
            node: None,
            mtime_ns: 0,
            size: 0,
            window: Vec::new(),
        }
    }

    fn feed_bytes(
        &mut self,
        chunk: &[u8],
        meta_override: Option<&Map<String, Value>>,
    ) -> DomainResult<Result<(), RestartParse>> {
        let (records, lines) = batch_records(chunk, self.line_count + 1);
        self.line_count += lines;
        self.feed(&records, meta_override)
    }

    fn feed(
        &mut self,
        records: &[Value],
        meta_override: Option<&Map<String, Value>>,
    ) -> DomainResult<Result<(), RestartParse>> {
        for record in records {
            if let Some(kind) = record.get("type").and_then(Value::as_str) {
                if RESPONSE_PAYLOAD_TYPES.contains(&kind) {
                    return Err(DomainError::agent_format_changed(
                        "codex",
                        "jsonl[].type",
                        Value::from("response_item with payload.type"),
                        Value::from(kind),
                    ));
                }
            }
        }
        let batch_has_meta = records
            .iter()
            .any(|record| record.get("type").and_then(Value::as_str) == Some("session_meta"));
        if self.session.is_none() {
            let discovered;
            let meta = match meta_override {
                Some(meta) => meta,
                None => {
                    discovered = records
                        .iter()
                        .find(|record| {
                            record.get("type").and_then(Value::as_str) == Some("session_meta")
                        })
                        .and_then(|record| record.get("payload"))
                        .and_then(Value::as_object)
                        .cloned()
                        .unwrap_or_default();
                    &discovered
                }
            };
            self.saw_meta = batch_has_meta || meta_override.is_some();
            self.create_session(meta)?;
        } else if batch_has_meta && !self.saw_meta {
            // 首批没等到 meta 却先建了会话：身份可能算错，推倒重来。
            return Ok(Err(RestartParse));
        }
        let session = self.session.as_mut().expect("会话已创建");
        for record in records {
            let kind = record.get("type").and_then(Value::as_str).unwrap_or("");
            if kind == MALFORMED_JSONL || kind == MALFORMED_RECORD {
                let mut params = Map::new();
                params.insert(
                    "line_number".into(),
                    record.get("line_number").cloned().unwrap_or(Value::Null),
                );
                params.insert(
                    "error".into(),
                    record.get("error").cloned().unwrap_or(Value::Null),
                );
                session.lose("session.malformed_record", params);
            }
        }
        for record in records {
            self.apply(record);
            self.ordinal += 1;
        }
        Ok(Ok(()))
    }

    fn create_session(&mut self, meta: &Map<String, Value>) -> DomainResult<()> {
        let ident =
            topology::identity(meta).ok_or_else(|| topology::missing_identity_error(&self.path))?;
        let mut session = Session::new(
            "codex",
            ident.id.clone(),
            meta.get("cwd").and_then(Value::as_str).unwrap_or(""),
        );
        session.root_id = Some(ident.root_id);
        session.parent_association = ident.parent_id.as_ref().map(|_| "parent-metadata".into());
        session.parent_id = ident.parent_id;
        session.forked_from_id = ident.forked_from_id;
        session.agent_id = ident.agent_id;
        session.agent_path = ident.agent_path;
        session.agent_type = ident.agent_type;
        session.agent_nickname = ident.agent_nickname;
        session.agent_role = ident.agent_role;
        session.model_provider = ident.model_provider;
        session.model = ident.model;
        session.depth = ident.depth;
        self.session = Some(session);
        Ok(())
    }

    /// 把暂存的思考/工具块前置进 `blocks`，并把 pending 定位重指到目标消息。
    fn flush_pending_into(
        &mut self,
        blocks: &mut Vec<Block>,
        message_index: usize,
        message_source_id: Option<&str>,
    ) {
        for block in &mut self.current_tools {
            if let Some(tool) = block.tool.as_mut() {
                if let Some(source_id) = message_source_id {
                    if tool.source_message_id.is_none() {
                        tool.source_message_id = Some(source_id.to_string());
                    }
                }
            }
        }
        let reasoning_len = self.current_reasoning.len();
        let mut prefix: Vec<Block> = Vec::with_capacity(reasoning_len + self.current_tools.len());
        prefix.append(&mut self.current_reasoning);
        prefix.append(&mut self.current_tools);
        blocks.splice(0..0, prefix);
        for slot in self.pending.values_mut() {
            if let ToolSlot::Pending(index) = *slot {
                *slot = ToolSlot::Message(message_index, reasoning_len + index);
            }
        }
    }

    fn apply(&mut self, record: &Value) {
        let ordinal = self.ordinal;
        let record_type = record.get("type").and_then(Value::as_str).unwrap_or("");
        if record_type == "compacted" {
            let session = self.session.as_mut().expect("会话已创建");
            let after = session
                .messages
                .iter()
                .rev()
                .find_map(|message| message.source_id.clone());
            let entries = record.as_object().cloned().unwrap_or_default();
            session
                .context_compactions
                .push(codex_compaction(&entries, ordinal, after));
            return;
        }
        if record_type != "response_item" {
            return;
        }
        let empty = Map::new();
        let payload = record
            .get("payload")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or(empty);
        match payload.get("type").and_then(Value::as_str).unwrap_or("") {
            "message" => self.apply_message(record, &payload, ordinal),
            "custom_tool_call" | "function_call" => self.apply_tool_call(&payload, ordinal),
            "custom_tool_call_output" | "function_call_output" => self.apply_tool_output(&payload),
            "reasoning" => {
                let text = summary_text(&payload);
                let session = self.session.as_mut().expect("会话已创建");
                match text {
                    Some(text) => {
                        self.current_reasoning.push(Block::text(text));
                        let mut params = Map::new();
                        params.insert("metadata_kind".into(), Value::from("encrypted_content"));
                        session.lose("migration.reasoning_metadata_dropped", params);
                    }
                    None => {
                        let mut params = Map::new();
                        params.insert("metadata_kind".into(), Value::from("encrypted_content"));
                        session.lose("migration.reasoning_dropped", params);
                    }
                }
            }
            other => {
                let session = self.session.as_mut().expect("会话已创建");
                let mut params = Map::new();
                params.insert(
                    "kind".into(),
                    payload
                        .get("type")
                        .cloned()
                        .unwrap_or_else(|| Value::from(other)),
                );
                session.lose("migration.unknown_block_dropped", params);
            }
        }
    }

    fn apply_message(&mut self, record: &Value, payload: &Map<String, Value>, ordinal: usize) {
        let role = payload.get("role").and_then(Value::as_str).unwrap_or("");
        let content: Vec<Value> = match payload.get("content") {
            Some(Value::String(text)) => {
                let mut entry = Map::new();
                entry.insert(
                    "type".into(),
                    Value::from(if role == "user" {
                        "input_text"
                    } else {
                        "output_text"
                    }),
                );
                entry.insert("text".into(), Value::from(text.as_str()));
                vec![Value::Object(entry)]
            }
            Some(Value::Array(items)) => items.clone(),
            _ => Vec::new(),
        };
        let texts: Vec<&str> = content
            .iter()
            .filter_map(Value::as_object)
            .filter(|entry| {
                matches!(
                    entry.get("type").and_then(Value::as_str),
                    Some("input_text") | Some("output_text")
                )
            })
            .map(|entry| entry.get("text").and_then(Value::as_str).unwrap_or(""))
            .collect();
        let text = texts
            .into_iter()
            .filter(|item| !item.is_empty())
            .collect::<Vec<_>>()
            .join("\n");

        let mut image_blocks: Vec<Block> = Vec::new();
        for (content_index, item) in content.iter().enumerate() {
            let Some(entry) = item.as_object() else {
                continue;
            };
            if entry.get("type").and_then(Value::as_str) != Some("input_image") {
                continue;
            }
            let url = entry.get("image_url").cloned().unwrap_or(Value::from(""));
            let image = image_from_data_url(
                &format!("record:{ordinal}:image:{content_index}"),
                &url,
                None,
            );
            match image {
                None => {
                    let session = self.session.as_mut().expect("会话已创建");
                    let mut params = Map::new();
                    params.insert("kind".into(), Value::from("input_image"));
                    session.lose("migration.unknown_block_dropped", params);
                }
                Some(image) => {
                    let mut block = Block::new(BlockKind::Image);
                    block.image = Some(image);
                    image_blocks.push(block);
                }
            }
        }

        let trimmed = text.trim();
        if role == "user"
            && SKIP_USER_PREFIX
                .iter()
                .any(|prefix| trimmed.starts_with(prefix))
        {
            return;
        }
        let created_at = timestamp_of(record.get("timestamp"));
        if role == "user" && !(self.current_tools.is_empty() && self.current_reasoning.is_empty()) {
            let source_id = format!("record:{ordinal}");
            let message_index = self.session.as_ref().expect("会话已创建").messages.len();
            let mut pending_blocks = Vec::new();
            self.flush_pending_into(&mut pending_blocks, message_index, Some(&source_id));
            let mut message = Message::new("assistant");
            message.blocks = pending_blocks;
            message.source_id = Some(source_id);
            message.created_at = created_at.clone();
            self.session
                .as_mut()
                .expect("会话已创建")
                .messages
                .push(message);
        }
        if trimmed.is_empty()
            && image_blocks.is_empty()
            && self.current_tools.is_empty()
            && self.current_reasoning.is_empty()
        {
            return;
        }
        let mut blocks: Vec<Block> = Vec::new();
        if !trimmed.is_empty() {
            blocks.push(Block::text(text.as_str()));
        }
        blocks.extend(image_blocks);
        let message_index = self.session.as_ref().expect("会话已创建").messages.len();
        if role == "assistant" {
            let source_id = format!("record:{ordinal}");
            self.flush_pending_into(&mut blocks, message_index, Some(&source_id));
        }
        let mut message = Message::new(role);
        message.blocks = blocks;
        message.source_id = Some(format!("record:{ordinal}"));
        message.created_at = created_at;
        self.session
            .as_mut()
            .expect("会话已创建")
            .messages
            .push(message);
    }

    fn apply_tool_call(&mut self, payload: &Map<String, Value>, _ordinal: usize) {
        let payload_type = payload.get("type").and_then(Value::as_str).unwrap_or("");
        let mut call: ToolCall = {
            let session = self.session.as_mut().expect("会话已创建");
            if payload_type == "function_call" {
                tool_calls::parse_function_call(payload)
            } else if payload.get("name").and_then(Value::as_str) == Some("spawn_agent") {
                let raw = payload.get("input").cloned().unwrap_or(Value::from(""));
                ToolCall::new(
                    "spawn_agent",
                    Some(CanonicalOp::AGENT_SPAWN.to_string()),
                    tool_calls::spawn_input(&tool_calls::json_args(&raw)),
                )
            } else {
                tool_calls::parse_custom_call(payload, session)
            }
        };
        call.source_call_id = payload
            .get("call_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        if call.op.as_deref() == Some(CanonicalOp::AGENT_SPAWN) {
            call.source_message_id = self
                .session
                .as_ref()
                .expect("会话已创建")
                .messages
                .iter()
                .rev()
                .find(|message| message.role == "user" || message.role == "assistant")
                .and_then(|message| message.source_id.clone());
        }
        let key = payload
            .get("call_id")
            .map(crate::adapters::shared::dialect::python_str)
            .unwrap_or_else(|| "None".to_string());
        self.pending
            .insert(key, ToolSlot::Pending(self.current_tools.len()));
        let mut block = Block::new(BlockKind::Tool);
        block.tool = Some(call);
        self.current_tools.push(block);
    }

    fn apply_tool_output(&mut self, payload: &Map<String, Value>) {
        let key = payload
            .get("call_id")
            .map(crate::adapters::shared::dialect::python_str)
            .unwrap_or_else(|| "None".to_string());
        let slot = self.pending.remove(&key);
        let output = payload.get("output").cloned().unwrap_or(Value::from(""));
        let result = tool_results::parse_result(&output);
        let result_id = payload
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string);
        let tool = match slot {
            Some(ToolSlot::Pending(index)) => self
                .current_tools
                .get_mut(index)
                .and_then(|block| block.tool.as_mut()),
            Some(ToolSlot::Message(message_index, block_index)) => self
                .session
                .as_mut()
                .expect("会话已创建")
                .messages
                .get_mut(message_index)
                .and_then(|message| message.blocks.get_mut(block_index))
                .and_then(|block| block.tool.as_mut()),
            None => None,
        };
        match tool {
            Some(tool) => {
                tool.result = Some(result);
                tool.source_result_id = result_id;
            }
            None => {
                let session = self.session.as_mut().expect("会话已创建");
                let mut params = Map::new();
                params.insert(
                    "call_id".into(),
                    payload.get("call_id").cloned().unwrap_or(Value::Null),
                );
                session.lose("session.orphan_tool_result", params);
            }
        }
    }

    /// 把解析状态定稿成可对外返回的 Session（不修改缓存对象）。
    fn view(&self) -> Session {
        let mut session = self
            .session
            .clone()
            .unwrap_or_else(|| Session::new("codex", "", ""));
        if !self.current_tools.is_empty() || !self.current_reasoning.is_empty() {
            let mut message = Message::new("assistant");
            message.blocks = self
                .current_reasoning
                .iter()
                .cloned()
                .chain(self.current_tools.iter().cloned())
                .collect();
            session.messages.push(message);
        }
        let candidates: Vec<usize> = session
            .context_compactions
            .iter()
            .enumerate()
            .filter(|(_, compaction)| {
                compaction
                    .source_meta
                    .get("replacement_history_present")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            })
            .map(|(index, _)| index)
            .collect();
        for index in &candidates {
            let meta = &mut session.context_compactions[*index].source_meta;
            // 重建整表以保住剩余键的插入序（`Map::remove` 在 preserve_order 下是
            // swap_remove，会打乱顺序）。
            let rebuilt: Map<String, Value> = meta
                .iter()
                .filter(|(key, _)| key.as_str() != "active")
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            *meta = rebuilt;
        }
        if let Some(last) = candidates.last() {
            session.context_compactions[*last]
                .source_meta
                .insert("active".into(), Value::Bool(true));
        }
        session
    }
}

/// 单文件全量解析（无缓存）：测试与回退路径使用。
pub fn read_one(path: &Path, meta: Option<&Map<String, Value>>) -> DomainResult<Session> {
    let mut parser = RolloutParser::new(path);
    let data = fs::read(path)
        .map_err(|error| DomainError::internal(format!("读取 rollout 失败: {error}")))?;
    // 全量解析的首批必然含 session_meta（若有），不会触发 RestartParse。
    parser.feed_bytes(&data, meta)??;
    Ok(parser.view())
}

const WINDOW: usize = 4096;
const CACHE_MAX_ENTRIES: usize = 128;
const CACHE_MAX_ENTRY_SOURCE_BYTES: u64 = 1024 * 1024;
const CACHE_MAX_TOTAL_SOURCE_BYTES: u64 = 16 * 1024 * 1024;
const TREE_LOCK_SHARDS: usize = 64;

/// rollout → 增量解析器的 LRU 缓存。
struct RolloutCache {
    /// `(key → parser)`，`order` 维护 LRU 次序（尾部最新）。
    entries: HashMap<String, RolloutParser>,
    order: Vec<String>,
}

impl RolloutCache {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            order: Vec::new(),
        }
    }

    fn touch(&mut self, key: &str) {
        if let Some(position) = self.order.iter().position(|item| item == key) {
            let key = self.order.remove(position);
            self.order.push(key);
        }
    }

    fn read(&mut self, path: &Path) -> DomainResult<Session> {
        let key = path.to_string_lossy().into_owned();
        let Ok(metadata) = fs::metadata(path) else {
            // 路径不可 stat（消失中/测试桩）：不缓存，直接读。
            self.entries.remove(&key);
            self.order.retain(|item| item != &key);
            return read_one(path, None);
        };
        if self.entries.contains_key(&key) {
            match self.advance(&key, &metadata) {
                Ok(Ok(session)) => {
                    self.touch(&key);
                    return Ok(session);
                }
                Ok(Err(RestartParse)) => {}
                Err(error) => return Err(error),
            }
        }
        let parser = Self::full_parse(path, &metadata)?;
        let session = parser.view();
        self.entries.insert(key.clone(), parser);
        self.order.retain(|item| item != &key);
        self.order.push(key);
        self.evict();
        Ok(session)
    }

    fn advance(
        &mut self,
        key: &str,
        metadata: &fs::Metadata,
    ) -> DomainResult<Result<Session, RestartParse>> {
        let stat = file_node(metadata);
        let size = metadata.len();
        let mtime_ns = mtime_nanos(metadata);
        let parser = self.entries.get_mut(key).expect("调用方已确认命中");
        if Some(stat) != parser.node || size < parser.offset {
            return Ok(Err(RestartParse));
        }
        if size == parser.size && mtime_ns == parser.mtime_ns {
            return Ok(Ok(parser.view()));
        }
        if size == parser.offset {
            // 只有 mtime 变了（touch/元数据变更），内容前提不可信。
            return Ok(Err(RestartParse));
        }
        let mut stream = fs::File::open(key)
            .map_err(|error| DomainError::internal(format!("读取 rollout 失败: {error}")))?;
        let window_len = parser.window.len() as u64;
        stream
            .seek(SeekFrom::Start(parser.offset - window_len))
            .map_err(|error| DomainError::internal(format!("定位 rollout 失败: {error}")))?;
        let mut probe = vec![0u8; parser.window.len()];
        if stream.read_exact(&mut probe).is_err() || probe != parser.window {
            return Ok(Err(RestartParse));
        }
        let mut data = Vec::new();
        stream
            .read_to_end(&mut data)
            .map_err(|error| DomainError::internal(format!("读取 rollout 失败: {error}")))?;
        let span = complete_span(&data);
        if span > 0 {
            if let Err(RestartParse) = parser.feed_bytes(&data[..span], None)? {
                return Ok(Err(RestartParse));
            }
            parser.offset += span as u64;
            let tail = &data[..span];
            let tail = &tail[tail.len().saturating_sub(WINDOW)..];
            parser.window.extend_from_slice(tail);
            let keep = parser.window.len().saturating_sub(WINDOW);
            parser.window.drain(..keep);
        }
        parser.mtime_ns = mtime_ns;
        parser.size = size;
        Ok(Ok(parser.view()))
    }

    fn full_parse(path: &Path, metadata: &fs::Metadata) -> DomainResult<RolloutParser> {
        let mut parser = RolloutParser::new(path);
        let data = fs::read(path)
            .map_err(|error| DomainError::internal(format!("读取 rollout 失败: {error}")))?;
        let span = complete_span(&data);
        parser.feed_bytes(&data[..span], None)??;
        parser.offset = span as u64;
        parser.node = Some(file_node(metadata));
        parser.mtime_ns = mtime_nanos(metadata);
        parser.size = metadata.len();
        parser.window = data[..span][data[..span].len().saturating_sub(WINDOW)..].to_vec();
        Ok(parser)
    }

    fn evict(&mut self) {
        loop {
            // 单条上限独立于 LRU 次序：并发读取可能让超大条目不再位于队首。
            if let Some(oversized) = self
                .order
                .iter()
                .find(|key| {
                    self.entries
                        .get(*key)
                        .is_some_and(|entry| entry.size > CACHE_MAX_ENTRY_SOURCE_BYTES)
                })
                .cloned()
            {
                self.order.retain(|key| key != &oversized);
                self.entries.remove(&oversized);
                continue;
            }
            let total: u64 = self.entries.values().map(|entry| entry.size).sum();
            let over_entries = self.entries.len() > CACHE_MAX_ENTRIES;
            let over_total = total > CACHE_MAX_TOTAL_SOURCE_BYTES;
            if !over_entries && !over_total {
                return;
            }
            if self.order.is_empty() {
                return;
            }
            let oldest = self.order.remove(0);
            self.entries.remove(&oldest);
        }
    }
}

impl From<RestartParse> for DomainError {
    fn from(_: RestartParse) -> Self {
        DomainError::internal("Codex rollout 增量解析前提被打破")
    }
}

fn file_node(metadata: &fs::Metadata) -> (u64, u64) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        (metadata.dev(), metadata.ino())
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        (0, 0)
    }
}

fn mtime_nanos(metadata: &fs::Metadata) -> i128 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|delta| delta.as_nanos() as i128)
        .unwrap_or(0)
}

fn parse_cache() -> &'static Mutex<RolloutCache> {
    static CACHE: OnceLock<Mutex<RolloutCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(RolloutCache::new()))
}

/// 清空增量解析缓存（测试用）。
pub fn clear_cache() {
    let mut cache = parse_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.entries.clear();
    cache.order.clear();
}

/// 树装配会读取多份缓存条目；固定分片锁避免按会话永久泄漏 mutex。
fn tree_lock(key: &str) -> &'static Mutex<()> {
    use std::hash::{Hash, Hasher};
    static LOCKS: OnceLock<[Mutex<()>; TREE_LOCK_SHARDS]> = OnceLock::new();
    let locks = LOCKS.get_or_init(|| std::array::from_fn(|_| Mutex::new(())));
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    &locks[hasher.finish() as usize % TREE_LOCK_SHARDS]
}

fn cached_read_one(path: &Path) -> DomainResult<Session> {
    let mut cache = parse_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.read(path)
}

/// 读一条 rollout 并递归加载同根的全部后代。
pub fn read(path: &str, sessions_dir: Option<&Path>) -> DomainResult<Session> {
    // `Path.resolve()` 默认非严格：路径不存在也照样归一，失败留给后续读取报错。
    let expanded = expanduser(path);
    let rollout = fs::canonicalize(&expanded).unwrap_or(expanded);
    let lock = tree_lock(&rollout.to_string_lossy());
    let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    topology::read_tree(&rollout, &cached_read_one, sessions_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write_rollout(dir: &Path, name: &str, records: &[Value]) -> PathBuf {
        let path = dir.join(name);
        let payload: String = records
            .iter()
            .map(|record| serde_json::to_string(record).unwrap() + "\n")
            .collect();
        fs::write(&path, payload).unwrap();
        path
    }

    fn base_records() -> Vec<Value> {
        vec![
            json!({"type": "session_meta", "payload": {"id": "s1", "cwd": "/w"}}),
            json!({"type": "response_item", "payload": {
                "type": "message", "role": "user",
                "content": [{"type": "input_text", "text": "hi"}]}}),
            json!({"type": "response_item", "payload": {
                "type": "message", "role": "assistant", "id": "m1",
                "content": [{"type": "output_text", "text": "ok"}]}}),
        ]
    }

    #[test]
    fn complete_span_keeps_half_written_tails() {
        assert_eq!(complete_span(b"{\"a\":1}\n"), 8);
        // 尾部是完整 JSON -> 一并消费。
        assert_eq!(complete_span(b"{\"a\":1}\n{\"b\":2}"), 15);
        // 尾部是半行 -> 留到下一轮。
        assert_eq!(complete_span(b"{\"a\":1}\n{\"b\":"), 8);
        assert_eq!(complete_span(b""), 0);
    }

    #[test]
    fn malformed_lines_become_loss_records() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout-x.jsonl");
        fs::write(
            &path,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"s1\",\"cwd\":\"\"}}\nnot json\n[1]\n",
        )
        .unwrap();
        let session = read_one(&path, None).unwrap();
        let codes: Vec<&str> = session
            .loss
            .iter()
            .map(|event| event.code.as_str())
            .collect();
        assert_eq!(
            codes,
            ["session.malformed_record", "session.malformed_record"]
        );
        assert_eq!(
            session.loss[1].params["error"],
            json!("record is not an object")
        );
        assert_eq!(session.loss[1].params["line_number"], json!(3));
    }

    #[test]
    fn record_level_payload_types_are_a_format_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_rollout(
            dir.path(),
            "rollout-x.jsonl",
            &[json!({"type": "function_call", "payload": {}})],
        );
        let error = read_one(&path, None).unwrap_err();
        assert_eq!(error.code, "agent.format_changed");
        assert_eq!(error.params()["actual"], json!("function_call"));
    }

    #[test]
    fn environment_context_messages_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let mut records = base_records();
        records.insert(
            1,
            json!({"type": "response_item", "payload": {
                "type": "message", "role": "user",
                "content": [{"type": "input_text", "text": "<environment_context>x"}]}}),
        );
        let path = write_rollout(dir.path(), "rollout-x.jsonl", &records);
        let session = read_one(&path, None).unwrap();
        assert_eq!(session.messages.len(), 2);
        assert_eq!(session.messages[0].blocks[0].text, "hi");
    }

    #[test]
    fn tool_calls_pair_with_their_outputs_across_message_flushes() {
        let dir = tempfile::tempdir().unwrap();
        let records = vec![
            json!({"type": "session_meta", "payload": {"id": "s1", "cwd": "/w"}}),
            json!({"type": "response_item", "payload": {
                "type": "message", "role": "user",
                "content": [{"type": "input_text", "text": "go"}]}}),
            json!({"type": "response_item", "payload": {
                "type": "function_call", "call_id": "c1", "name": "exec_command",
                "arguments": "{\"cmd\":\"ls\"}"}}),
            json!({"type": "response_item", "payload": {
                "type": "message", "role": "assistant", "content": [
                    {"type": "output_text", "text": "done"}]}}),
            // 输出迟到：调用此时已被并入上面那条 assistant 消息。
            json!({"type": "response_item", "payload": {
                "type": "function_call_output", "call_id": "c1", "id": "o1",
                "output": "listing"}}),
        ];
        let path = write_rollout(dir.path(), "rollout-x.jsonl", &records);
        let session = read_one(&path, None).unwrap();
        let tool = session.messages[1].blocks[0].tool.as_ref().unwrap();
        assert_eq!(tool.op.as_deref(), Some(CanonicalOp::SHELL_EXEC));
        assert_eq!(tool.source_message_id.as_deref(), Some("record:3"));
        assert_eq!(tool.source_result_id.as_deref(), Some("o1"));
        assert_eq!(tool.result.as_ref().unwrap().blocks[0].text, "listing");
    }

    #[test]
    fn orphan_outputs_are_recorded_as_loss() {
        let dir = tempfile::tempdir().unwrap();
        let records = vec![
            json!({"type": "session_meta", "payload": {"id": "s1", "cwd": "/w"}}),
            json!({"type": "response_item", "payload": {
                "type": "function_call_output", "call_id": "ghost", "output": "x"}}),
        ];
        let path = write_rollout(dir.path(), "rollout-x.jsonl", &records);
        let session = read_one(&path, None).unwrap();
        assert_eq!(session.loss[0].code, "session.orphan_tool_result");
        assert_eq!(session.loss[0].params["call_id"], json!("ghost"));
    }

    #[test]
    fn trailing_tools_are_synthesised_into_an_assistant_message() {
        let dir = tempfile::tempdir().unwrap();
        let records = vec![
            json!({"type": "session_meta", "payload": {"id": "s1", "cwd": "/w"}}),
            json!({"type": "response_item", "payload": {
                "type": "message", "role": "user",
                "content": [{"type": "input_text", "text": "go"}]}}),
            json!({"type": "response_item", "payload": {
                "type": "function_call", "call_id": "c1", "name": "exec_command",
                "arguments": "{\"cmd\":\"ls\"}"}}),
        ];
        let path = write_rollout(dir.path(), "rollout-x.jsonl", &records);
        let session = read_one(&path, None).unwrap();
        assert_eq!(session.messages.len(), 2);
        assert_eq!(session.messages[1].role, "assistant");
        assert_eq!(session.messages[1].source_id, None);
    }

    #[test]
    fn compactions_flag_only_the_last_replaceable_window() {
        let dir = tempfile::tempdir().unwrap();
        let records = vec![
            json!({"type": "session_meta", "payload": {"id": "s1", "cwd": "/w"}}),
            json!({"type": "compacted", "payload": {
                "message": " summary ", "window_id": "w1",
                "replacement_history": [{"type": "compaction", "encrypted_content": "x"}]}}),
            json!({"type": "compacted", "payload": {
                "window_id": "w2",
                "replacement_history": [{"type": "compaction", "encrypted_content": "y"}]}}),
            json!({"type": "compacted", "payload": {"window_id": "w3"}}),
        ];
        let path = write_rollout(dir.path(), "rollout-x.jsonl", &records);
        let session = read_one(&path, None).unwrap();
        assert_eq!(session.context_compactions.len(), 3);
        assert_eq!(session.context_compactions[0].summary_text, "summary");
        assert_eq!(session.context_compactions[0].summary_status, "available");
        assert_eq!(session.context_compactions[1].summary_status, "protected");
        assert_eq!(session.context_compactions[2].summary_status, "missing");
        assert!(session.context_compactions[0]
            .source_meta
            .get("active")
            .is_none());
        assert_eq!(
            session.context_compactions[1].source_meta["active"],
            json!(true)
        );
    }

    #[test]
    fn incremental_reads_extend_the_cached_parser() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_rollout(dir.path(), "rollout-x.jsonl", &base_records());
        clear_cache();
        let first = cached_read_one(&path).unwrap();
        assert_eq!(first.messages.len(), 2);

        // 追加一轮，mtime 与 size 都变，走增量分支。
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        use std::io::Write as _;
        writeln!(
            file,
            "{}",
            serde_json::to_string(&json!({"type": "response_item", "payload": {
                "type": "message", "role": "user",
                "content": [{"type": "input_text", "text": "again"}]}}))
            .unwrap()
        )
        .unwrap();
        drop(file);
        let second = cached_read_one(&path).unwrap();
        assert_eq!(second.messages.len(), 3);
        assert_eq!(second.messages[2].blocks[0].text, "again");
        // 再读一次不应重复累积。
        let third = cached_read_one(&path).unwrap();
        assert_eq!(third.messages.len(), 3);
        clear_cache();
    }

    #[test]
    fn oversized_rollouts_are_not_kept_in_the_incremental_cache() {
        let dir = tempfile::tempdir().unwrap();
        let records = vec![
            json!({"type": "session_meta", "payload": {"id": "large", "cwd": "/w"}}),
            json!({"type": "response_item", "payload": {
            "type": "message", "role": "user",
            "content": [{"type": "input_text", "text": "x".repeat(
                CACHE_MAX_ENTRY_SOURCE_BYTES as usize + 1024
            )}]}}),
        ];
        let path = write_rollout(dir.path(), "rollout-large.jsonl", &records);
        clear_cache();
        assert_eq!(cached_read_one(&path).unwrap().source_id, "large");
        let key = path.to_string_lossy().into_owned();
        assert!(
            parse_cache()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .entries
                .get(&key)
                .is_none(),
            "超过单条上限的 parser 不得常驻"
        );
    }

    #[test]
    fn truncation_forces_a_full_reparse() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_rollout(dir.path(), "rollout-x.jsonl", &base_records());
        clear_cache();
        assert_eq!(cached_read_one(&path).unwrap().messages.len(), 2);
        // 整份重写成另一个会话。
        let replaced = vec![
            json!({"type": "session_meta", "payload": {"id": "s2", "cwd": "/w"}}),
            json!({"type": "response_item", "payload": {
                "type": "message", "role": "user",
                "content": [{"type": "input_text", "text": "fresh"}]}}),
        ];
        write_rollout(dir.path(), "rollout-x.jsonl", &replaced);
        let session = cached_read_one(&path).unwrap();
        assert_eq!(session.source_id, "s2");
        assert_eq!(session.messages.len(), 1);
        clear_cache();
    }

    #[test]
    fn reasoning_summaries_degrade_to_text_and_record_loss() {
        let dir = tempfile::tempdir().unwrap();
        let records = vec![
            json!({"type": "session_meta", "payload": {"id": "s1", "cwd": "/w"}}),
            json!({"type": "response_item", "payload": {
                "type": "reasoning", "summary": [{"text": "think"}],
                "encrypted_content": "zzz"}}),
            json!({"type": "response_item", "payload": {
                "type": "reasoning", "encrypted_content": "zzz"}}),
            json!({"type": "response_item", "payload": {
                "type": "message", "role": "assistant",
                "content": [{"type": "output_text", "text": "done"}]}}),
        ];
        let path = write_rollout(dir.path(), "rollout-x.jsonl", &records);
        let session = read_one(&path, None).unwrap();
        let codes: Vec<&str> = session
            .loss
            .iter()
            .map(|event| event.code.as_str())
            .collect();
        assert_eq!(
            codes,
            [
                "migration.reasoning_metadata_dropped",
                "migration.reasoning_dropped"
            ]
        );
        assert_eq!(session.messages[0].blocks[0].text, "think");
        assert_eq!(session.messages[0].blocks[1].text, "done");
    }
}
