//! Claude Code reader：JSONL 会话文件 → 规范化会话树。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::adapters::shared::media::image_from_base64;
use crate::adapters::shared::scanner::iter_lines;
use crate::adapters::shared::tool_canon::{canonical_tool_input, canonical_tool_op};
use crate::errors::{DomainError, DomainResult};
use crate::model::{
    AgentEdge, Block, BlockKind, ContextCompaction, ImageAsset, Message, Session, Timestamp,
    ToolCall, ToolResult, ToolResultBlock, ToolResultBlockKind, ToolResultStatus,
};
use crate::tool_ops::CanonicalOp;

/// 解析失败的行在记录流里占位，交给 `session.lose` 汇报。
const MALFORMED: &str = "__ferry_malformed_jsonl__";

/// 原生 `toolUseResult.status` → 规范状态。
fn map_result_status(status: &str) -> ToolResultStatus {
    match status {
        "success" | "completed" | "teammate_spawned" => ToolResultStatus::Success,
        "error" => ToolResultStatus::Error,
        "interrupted" => ToolResultStatus::Interrupted,
        "running" | "async_launched" => ToolResultStatus::Running,
        "pending" => ToolResultStatus::Pending,
        _ => ToolResultStatus::Unknown,
    }
}

fn result_status(block: &Value, native: &Map<String, Value>) -> ToolResultStatus {
    if block.get("is_error") == Some(&Value::Bool(true))
        || native.get("success") == Some(&Value::Bool(false))
    {
        return ToolResultStatus::Error;
    }
    if native.get("interrupted") == Some(&Value::Bool(true)) {
        return ToolResultStatus::Interrupted;
    }
    match native.get("status") {
        Some(Value::String(status)) => map_result_status(status),
        Some(_) => ToolResultStatus::Unknown,
        None => ToolResultStatus::Success,
    }
}

fn json_block(data: Value) -> ToolResultBlock {
    ToolResultBlock {
        data,
        ..ToolResultBlock::new(ToolResultBlockKind::Json)
    }
}

/// tool_result 的 `content` → 结构化结果块。
fn result_blocks(content: Option<&Value>) -> Vec<ToolResultBlock> {
    match content {
        Some(Value::String(text)) => {
            if text.is_empty() {
                Vec::new()
            } else {
                vec![ToolResultBlock::text(text.as_str())]
            }
        }
        Some(Value::Object(_)) => vec![json_block(content.cloned().unwrap_or(Value::Null))],
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                let Some(entry) = item.as_object() else {
                    return json_block(item.clone());
                };
                match entry.get("type").and_then(Value::as_str) {
                    Some("text") => ToolResultBlock::text(
                        entry.get("text").and_then(Value::as_str).unwrap_or(""),
                    ),
                    Some("image") => {
                        let source = entry.get("source");
                        ToolResultBlock {
                            data: source
                                .and_then(|source| source.get("data"))
                                .cloned()
                                .unwrap_or(Value::Null),
                            mime_type: source
                                .and_then(|source| source.get("media_type"))
                                .and_then(Value::as_str)
                                .map(std::string::ToString::to_string),
                            ..ToolResultBlock::new(ToolResultBlockKind::Image)
                        }
                    }
                    Some("tool_reference") => {
                        let mut data = entry.clone();
                        data.shift_remove("type");
                        ToolResultBlock {
                            data: Value::Object(data),
                            ..ToolResultBlock::new(ToolResultBlockKind::ToolReference)
                        }
                    }
                    _ => json_block(item.clone()),
                }
            })
            .collect(),
        Some(other) => vec![json_block(other.clone())],
    }
}

fn tool_result(block: &Value, native: Option<&Value>) -> ToolResult {
    let native = native
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let text_of = |key: &str| {
        native
            .get(key)
            .and_then(Value::as_str)
            .map(std::string::ToString::to_string)
    };
    ToolResult {
        status: result_status(block, &native),
        blocks: result_blocks(block.get("content")),
        stdout: text_of("stdout"),
        stderr: text_of("stderr"),
        // Python 显式拒绝 bool；`as_i64` 天然不接受 JSON 布尔。
        exit_code: native.get("exit_code").and_then(Value::as_i64),
        truncated: native.get("truncated").and_then(Value::as_bool),
        attachments: Vec::new(),
    }
}

/// 读入全部行；解析失败的行落成 `__ferry_malformed_jsonl__` 占位记录。
fn load(path: &Path) -> DomainResult<Vec<Value>> {
    let lines = iter_lines(path)
        .map_err(|error| DomainError::internal(format!("读取 claude 会话失败: {error}")))?;
    let mut records = Vec::new();
    for (index, line) in lines.enumerate() {
        let line =
            line.map_err(|error| DomainError::internal(format!("读取 claude 会话失败: {error}")))?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(&line) {
            Ok(value) => records.push(value),
            Err(error) => {
                let mut placeholder = Map::new();
                placeholder.insert("type".into(), Value::from(MALFORMED));
                placeholder.insert("line_number".into(), Value::from(index as i64 + 1));
                placeholder.insert("error".into(), Value::from(decode_error_message(&error)));
                records.push(Value::Object(placeholder));
            }
        }
    }
    Ok(records)
}

/// serde 的错误串带 `at line/column` 后缀，Python 的 `JSONDecodeError.msg` 不带。
fn decode_error_message(error: &serde_json::Error) -> String {
    let text = error.to_string();
    match text.find(" at line ") {
        Some(position) => text[..position].to_string(),
        None => text,
    }
}

fn native_agent_id(value: &Value) -> Option<&str> {
    value
        .get("agentId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

fn agent_id_of(lines: &[Value], path: &Path) -> Option<String> {
    if let Some(found) = lines.iter().find_map(native_agent_id) {
        return Some(found.to_string());
    }
    let stem = path.file_stem()?.to_string_lossy().into_owned();
    stem.strip_prefix("agent-")
        .map(std::string::ToString::to_string)
}

fn record_type(record: &Value) -> Option<&str> {
    record.get("type").and_then(Value::as_str)
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|float| float != 0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(entries)) => !entries.is_empty(),
    }
}

fn timestamp(value: Option<&Value>) -> Option<Timestamp> {
    match value {
        Some(Value::String(text)) => Some(Timestamp::Text(text.clone())),
        Some(Value::Number(number)) => number.as_i64().map(Timestamp::Millis),
        _ => None,
    }
}

fn text_field(record: &Value, key: &str) -> Option<String> {
    record
        .get(key)
        .and_then(Value::as_str)
        .map(std::string::ToString::to_string)
}

fn compact_summary_text(record: &Value) -> String {
    let content = record
        .get("message")
        .and_then(|message| message.get("content"));
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter(|item| item.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|item| item.get("text").filter(|text| truthy(Some(text))))
            .map(|text| text.as_str().unwrap_or_default().to_string())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// 压缩边界的 `logicalParentUuid` 常指向 system 记录，需沿 parentUuid 回溯到真实消息。
fn resolve_anchor(by_uuid: &HashMap<&str, &Value>, start: Option<&str>) -> Option<String> {
    let mut cursor = start.map(std::string::ToString::to_string);
    let mut seen: HashSet<String> = HashSet::new();
    while let Some(current) = cursor.clone() {
        if current.is_empty() || !seen.insert(current.clone()) {
            return cursor;
        }
        let Some(record) = by_uuid.get(current.as_str()) else {
            return Some(current);
        };
        if matches!(record_type(record), Some("user" | "assistant"))
            && !truthy(record.get("isCompactSummary"))
        {
            return Some(current);
        }
        cursor = text_field(record, "parentUuid");
    }
    cursor
}

fn context_compactions(lines: &[Value]) -> Vec<ContextCompaction> {
    let by_uuid: HashMap<&str, &Value> = lines
        .iter()
        .filter_map(|record| {
            record
                .get("uuid")
                .and_then(Value::as_str)
                .map(|uuid| (uuid, record))
        })
        .collect();

    let mut compactions: Vec<ContextCompaction> = Vec::new();
    let mut by_boundary: HashMap<String, usize> = HashMap::new();
    for (index, record) in lines.iter().enumerate() {
        if record_type(record) != Some("system")
            || record.get("subtype").and_then(Value::as_str) != Some("compact_boundary")
        {
            continue;
        }
        let metadata = record
            .get("compactMetadata")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let boundary_id =
            text_field(record, "uuid").unwrap_or_else(|| format!("compact-boundary:{index}"));
        let pre_tokens = metadata.get("preTokens").cloned();
        let post_tokens = metadata.get("postTokens").cloned();
        let mut metrics = Map::new();
        let mut put = |key: &str, value: Option<Value>| {
            if let Some(value) = value.filter(|value| !value.is_null()) {
                metrics.insert(key.to_string(), value);
            }
        };
        put("pre_tokens", pre_tokens.clone());
        put("post_tokens", post_tokens.clone());
        put(
            "cumulative_dropped_tokens",
            metadata.get("cumulativeDroppedTokens").cloned(),
        );
        put("duration_ms", metadata.get("durationMs").cloned());
        if let (Some(pre), Some(post)) = (
            pre_tokens.as_ref().and_then(Value::as_i64),
            post_tokens.as_ref().and_then(Value::as_i64),
        ) {
            metrics.insert("dropped_tokens".into(), Value::from((pre - post).max(0)));
        }
        let trigger = match metadata.get("trigger").and_then(Value::as_str) {
            Some("auto") => "automatic",
            Some("manual") => "manual",
            _ => "unknown",
        };
        let mut source_meta = Map::new();
        source_meta.insert(
            "preserved_segment".into(),
            metadata
                .get("preservedSegment")
                .cloned()
                .unwrap_or(Value::Null),
        );
        source_meta.insert(
            "preserved_messages".into(),
            metadata
                .get("preservedMessages")
                .cloned()
                .unwrap_or(Value::Null),
        );

        let mut compaction = ContextCompaction::new(boundary_id.clone(), "claude");
        compaction.after_message_id = resolve_anchor(
            &by_uuid,
            record.get("logicalParentUuid").and_then(Value::as_str),
        );
        compaction.event_locator = Some(boundary_id.clone());
        compaction.created_at = timestamp(record.get("timestamp"));
        compaction.trigger = trigger.to_string();
        compaction.state = "incomplete".to_string();
        compaction.metrics = metrics;
        compaction.source_meta = source_meta;
        by_boundary.insert(boundary_id, compactions.len());
        compactions.push(compaction);
    }

    for (index, record) in lines.iter().enumerate() {
        if record.get("isCompactSummary") != Some(&Value::Bool(true)) {
            continue;
        }
        let summary = compact_summary_text(record);
        let parent_id = text_field(record, "parentUuid");
        let slot = parent_id
            .as_ref()
            .and_then(|parent| by_boundary.get(parent.as_str()).copied());
        let slot = match slot {
            Some(slot) => slot,
            None => {
                let id = parent_id
                    .clone()
                    .or_else(|| text_field(record, "uuid"))
                    .unwrap_or_else(|| format!("compact-summary:{index}"));
                let mut compaction = ContextCompaction::new(id, "claude");
                compaction.after_message_id = resolve_anchor(
                    &by_uuid,
                    record.get("logicalParentUuid").and_then(Value::as_str),
                );
                compaction.event_locator = parent_id.clone();
                compaction.created_at = timestamp(record.get("timestamp"));
                compaction.state = "incomplete".to_string();
                compactions.push(compaction);
                compactions.len() - 1
            }
        };
        let compaction = &mut compactions[slot];
        compaction.summary_message_id = text_field(record, "uuid");
        compaction.summary_text = summary.clone();
        compaction.summary_status = if summary.is_empty() {
            "missing".to_string()
        } else {
            "available".to_string()
        };
        compaction.state = if summary.is_empty() {
            "incomplete".to_string()
        } else {
            "completed".to_string()
        };
    }
    compactions
}

/// 一次 Agent 派生调用在父会话里的落点。
#[derive(Clone, Debug)]
struct SpawnDescriptor {
    call_id: Option<String>,
    result_id: Option<String>,
    message_id: Option<String>,
    status: Option<String>,
    tool_input: Value,
}

struct DecodeResult {
    session: Session,
    path: PathBuf,
    /// 保持 Python dict 的插入序：同一 agent_id 后写覆盖值但不改位置。
    spawns: Vec<(String, SpawnDescriptor)>,
}

/// tool_use 在 canonical 消息树里的落点。
#[derive(Clone, Copy)]
enum ToolSlot {
    /// 尚在当前记录的本地块列表里。
    Current(usize),
    /// 已落进 `session.messages`。
    Committed(usize, usize),
    /// 所在消息被整条丢弃（tool_result 载体且无可见文本）。
    Dropped,
}

struct PendingTool {
    name: String,
    slot: ToolSlot,
}

fn decode_transcript(path: &Path, is_child: bool) -> DomainResult<DecodeResult> {
    let lines = load(path)?;
    let visible: Vec<&Value> = lines
        .iter()
        .filter(|record| {
            matches!(record_type(record), Some("user" | "assistant"))
                && (is_child || !truthy(record.get("isSidechain")))
        })
        .collect();
    let first = visible.first().copied();
    let agent_id = if is_child {
        agent_id_of(&lines, path)
    } else {
        None
    };
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned());
    let source_id = agent_id.clone().unwrap_or_else(|| {
        first
            .and_then(|record| text_field(record, "sessionId"))
            .or_else(|| stem.clone())
            .unwrap_or_default()
    });
    let cwd = first
        .and_then(|record| text_field(record, "cwd"))
        .unwrap_or_default();
    let mut session = Session::new("claude", source_id, cwd);
    session.agent_id = agent_id.clone();
    session.context_compactions = context_compactions(&lines);

    for record in &lines {
        if record_type(record) == Some(MALFORMED) {
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
    for record in &lines {
        if record_type(record) == Some("ai-title") {
            if let Some(title) = record.get("title").filter(|title| truthy(Some(title))) {
                session.title = title.as_str().unwrap_or_default().to_string();
            }
        }
        if record_type(record) == Some("fork-context-ref") {
            session.forked_from_id = text_field(record, "parentLastUuid");
            session.parent_id = text_field(record, "parentSessionId");
        }
    }

    let mut pending: HashMap<Option<String>, PendingTool> = HashMap::new();
    let mut pending_order: Vec<Option<String>> = Vec::new();
    let mut spawn_messages: HashMap<Option<String>, Option<String>> = HashMap::new();
    let mut spawns: Vec<(String, SpawnDescriptor)> = Vec::new();

    for record in &visible {
        if truthy(record.get("isMeta")) {
            continue;
        }
        let body = record.get("message");
        let content = body.and_then(|body| body.get("content"));
        let role = body
            .and_then(|body| body.get("role"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let mut message = Message::new(role);
        message.source_id = text_field(record, "uuid");
        message.parent_ids = match record.get("parentUuid").filter(|value| truthy(Some(value))) {
            Some(parent) => vec![parent.as_str().unwrap_or_default().to_string()],
            None => Vec::new(),
        };
        message.turn_id = text_field(record, "promptId");
        message.agent_id = native_agent_id(record)
            .map(std::string::ToString::to_string)
            .or_else(|| agent_id.clone());
        message.created_at = timestamp(record.get("timestamp"));

        if let Some(Value::String(text)) = content {
            message.blocks.push(Block::text(text.as_str()));
            session.messages.push(message);
            continue;
        }

        let mut result_carrier = false;
        let items = content
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut local_pending: Vec<Option<String>> = Vec::new();
        for (item_index, item) in items.iter().enumerate() {
            let Some(entry) = item.as_object() else {
                lose_unknown_block(&mut session, Value::Null);
                continue;
            };
            match entry.get("type").and_then(Value::as_str) {
                Some("text") => {
                    message.blocks.push(Block::text(
                        entry.get("text").and_then(Value::as_str).unwrap_or(""),
                    ));
                }
                Some("image") => {
                    let source = entry.get("source");
                    let asset_id = format!(
                        "{}:image:{item_index}",
                        record.get("uuid").and_then(Value::as_str).unwrap_or("None")
                    );
                    let image = image_from_base64(
                        &asset_id,
                        source
                            .and_then(|source| source.get("media_type"))
                            .and_then(Value::as_str)
                            .unwrap_or(""),
                        &source
                            .and_then(|source| source.get("data"))
                            .cloned()
                            .unwrap_or_else(|| Value::from("")),
                        None,
                    );
                    match image {
                        None => lose_unknown_block(&mut session, Value::from("image")),
                        Some(image) => message.blocks.push(image_block(image)),
                    }
                }
                Some("thinking") => {
                    let visible_text = entry
                        .get("thinking")
                        .and_then(Value::as_str)
                        .filter(|text| !text.trim().is_empty());
                    let mut params = Map::new();
                    params.insert("metadata_kind".into(), Value::from("signature"));
                    match visible_text {
                        Some(text) => {
                            message.blocks.push(Block::text(text));
                            session.lose("migration.reasoning_metadata_dropped", params);
                        }
                        None => session.lose("migration.reasoning_dropped", params),
                    }
                }
                Some("tool_use") => {
                    let name = entry
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let source_input = entry
                        .get("input")
                        .filter(|input| truthy(Some(input)))
                        .cloned()
                        .unwrap_or_else(|| Value::Object(Map::new()));
                    let mapped = if name == "Agent" {
                        Some(CanonicalOp::AGENT_SPAWN)
                    } else {
                        canonical_tool_op("claude", &name)
                    };
                    let (op, canonical_input) = match mapped {
                        Some(op) => (op.to_string(), canonical_tool_input(&name, &source_input)),
                        None => {
                            let mut wrapper = Map::new();
                            wrapper.insert("namespace".into(), Value::from("claude"));
                            wrapper.insert("name".into(), Value::from(name.as_str()));
                            wrapper.insert("input".into(), source_input.clone());
                            (CanonicalOp::TOOL_INVOKE.to_string(), Value::Object(wrapper))
                        }
                    };
                    let mut tool = ToolCall::new(name.as_str(), Some(op.clone()), canonical_input);
                    tool.source_call_id = entry
                        .get("id")
                        .and_then(Value::as_str)
                        .map(std::string::ToString::to_string);
                    let key = entry
                        .get("id")
                        .and_then(Value::as_str)
                        .map(std::string::ToString::to_string);
                    let block_index = message.blocks.len();
                    message.blocks.push(tool_block(tool));
                    if pending
                        .insert(
                            key.clone(),
                            PendingTool {
                                name,
                                slot: ToolSlot::Current(block_index),
                            },
                        )
                        .is_none()
                    {
                        pending_order.push(key.clone());
                    }
                    local_pending.push(key.clone());
                    if op == CanonicalOp::AGENT_SPAWN {
                        spawn_messages.insert(key, text_field(record, "uuid"));
                    }
                }
                Some("tool_result") => {
                    result_carrier = true;
                    let key = entry
                        .get("tool_use_id")
                        .and_then(Value::as_str)
                        .map(std::string::ToString::to_string);
                    let Some(found) = pending.remove(&key) else {
                        let mut params = Map::new();
                        params.insert(
                            "call_id".into(),
                            entry.get("tool_use_id").cloned().unwrap_or(Value::Null),
                        );
                        session.lose("session.orphan_tool_result", params);
                        continue;
                    };
                    local_pending.retain(|item| item != &key);
                    let native = record.get("toolUseResult");
                    let result = tool_result(item, native);
                    let agent = native
                        .and_then(native_agent_id)
                        .map(std::string::ToString::to_string);
                    let target = match found.slot {
                        ToolSlot::Current(block) => message.blocks.get_mut(block),
                        ToolSlot::Committed(index, block) => session
                            .messages
                            .get_mut(index)
                            .and_then(|message| message.blocks.get_mut(block)),
                        ToolSlot::Dropped => None,
                    }
                    .and_then(|block| block.tool.as_mut());
                    let Some(tool) = target else {
                        continue;
                    };
                    tool.source_result_id = text_field(record, "uuid");
                    tool.result = Some(result);
                    tool.agent_id = agent.clone();
                    if let Some(agent_id) = agent {
                        let descriptor = SpawnDescriptor {
                            call_id: tool.source_call_id.clone(),
                            result_id: tool.source_result_id.clone(),
                            message_id: spawn_messages.get(&tool.source_call_id).cloned().flatten(),
                            status: native
                                .and_then(|native| native.get("status"))
                                .and_then(Value::as_str)
                                .map(std::string::ToString::to_string),
                            tool_input: tool.input.clone(),
                        };
                        upsert_spawn(&mut spawns, agent_id, descriptor);
                    }
                }
                other => lose_unknown_block(&mut session, other.map_or(Value::Null, Value::from)),
            }
        }

        let has_visible_text = message
            .blocks
            .iter()
            .any(|block| block.kind == BlockKind::Text && !block.text.trim().is_empty());
        // Python：tool_result 载体且无可见文本 -> 整条丢弃；随后 `if blocks:` 再挡一层。
        let keep = (has_visible_text || !result_carrier) && !message.blocks.is_empty();
        if keep {
            let index = session.messages.len();
            session.messages.push(message);
            for key in local_pending {
                if let Some(entry) = pending.get_mut(&key) {
                    if let ToolSlot::Current(block) = entry.slot {
                        entry.slot = ToolSlot::Committed(index, block);
                    }
                }
            }
        } else {
            for key in local_pending {
                if let Some(entry) = pending.get_mut(&key) {
                    entry.slot = ToolSlot::Dropped;
                }
            }
        }
    }

    for key in pending_order {
        let Some(tool) = pending.get(&key) else {
            continue;
        };
        let mut params = Map::new();
        params.insert("tool_name".into(), Value::from(tool.name.as_str()));
        session.lose("session.unpaired_tool_use", params);
    }

    Ok(DecodeResult {
        session,
        path: path.to_path_buf(),
        spawns,
    })
}

fn upsert_spawn(
    spawns: &mut Vec<(String, SpawnDescriptor)>,
    agent_id: String,
    descriptor: SpawnDescriptor,
) {
    match spawns.iter_mut().find(|(key, _)| *key == agent_id) {
        Some(slot) => slot.1 = descriptor,
        None => spawns.push((agent_id, descriptor)),
    }
}

fn lose_unknown_block(session: &mut Session, kind: Value) {
    let mut params = Map::new();
    params.insert("kind".into(), kind);
    session.lose("migration.unknown_block_dropped", params);
}

fn tool_block(tool: ToolCall) -> Block {
    Block {
        tool: Some(tool),
        ..Block::new(BlockKind::Tool)
    }
}

fn image_block(image: ImageAsset) -> Block {
    Block {
        image: Some(image),
        ..Block::new(BlockKind::Image)
    }
}

/// 排序键：逐路径分量比较，对齐 Python `sorted(Path...)` 的元组序。
fn path_sort_key(path: &Path) -> Vec<String> {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect()
}

fn child_transcripts(main_path: &Path) -> Vec<PathBuf> {
    let child_dir = main_path.with_extension("").join("subagents");
    if !child_dir.exists() {
        return Vec::new();
    }
    let mut found: Vec<PathBuf> = walkdir::WalkDir::new(&child_dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(walkdir::DirEntry::into_path)
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("agent-") && name.ends_with(".jsonl"))
        })
        .collect();
    found.sort_by_key(|path| path_sort_key(path));
    found
}

/// 读取一棵 claude 会话树（主会话 + subagents）。
pub fn read(path: &str) -> DomainResult<Session> {
    let main_path = PathBuf::from(path);
    let root = decode_transcript(&main_path, false)?;
    let root_id = root.session.source_id.clone();
    let parent_dir = main_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();

    let mut decoded = vec![root];
    for child_path in child_transcripts(&main_path) {
        decoded.push(decode_transcript(&child_path, true)?);
    }
    decoded[0].session.root_id = Some(root_id.clone());

    let mut by_agent: HashMap<String, usize> = HashMap::new();
    for (index, item) in decoded.iter().enumerate().skip(1) {
        if let Some(agent_id) = item.session.agent_id.clone() {
            by_agent.insert(agent_id, index);
        }
    }

    let total = decoded.len();
    let mut parent_of: Vec<Option<usize>> = vec![None; total];
    let mut children_of: Vec<Vec<usize>> = vec![Vec::new(); total];
    let mut edges_of: Vec<Vec<AgentEdge>> = vec![Vec::new(); total];
    let mut assigned: HashSet<usize> = HashSet::new();

    let spawn_plan: Vec<(usize, Vec<(String, SpawnDescriptor)>)> = decoded
        .iter()
        .enumerate()
        .map(|(index, item)| (index, item.spawns.clone()))
        .collect();

    for (parent, spawns) in spawn_plan {
        for (agent_id, spawn) in spawns {
            let Some(&child) = by_agent.get(&agent_id) else {
                continue;
            };
            if child == parent
                || assigned.contains(&child)
                || is_ancestor(&parent_of, child, parent)
            {
                continue;
            }
            let agent_path = relative_agent_path(&decoded[child].path, &parent_dir);
            let parent_session_id = decoded[parent].session.source_id.clone();
            {
                let child_session = &mut decoded[child].session;
                child_session.parent_id = Some(parent_session_id.clone());
                child_session.root_id = Some(root_id.clone());
                child_session.agent_path = Some(agent_path.clone());
            }
            let input = spawn.tool_input.clone();
            let mut edge =
                AgentEdge::new(parent_session_id, decoded[child].session.source_id.clone());
            edge.source_call_id = spawn.call_id.clone();
            edge.spawn_message_id = spawn.message_id.clone();
            edge.result_message_id = spawn.result_id.clone();
            edge.agent_id = Some(agent_id);
            edge.agent_path = Some(agent_path);
            edge.agent_type = input
                .get("subagent_type")
                .and_then(Value::as_str)
                .map(std::string::ToString::to_string);
            edge.prompt = input
                .get("prompt")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            edge.status = spawn.status.clone();
            edge.association = "agent-id".to_string();
            edge.confidence = 1.0;
            children_of[parent].push(child);
            edges_of[parent].push(edge);
            parent_of[child] = Some(parent);
            assigned.insert(child);
        }
    }

    for child in 1..total {
        if assigned.contains(&child) {
            continue;
        }
        let agent_path = relative_agent_path(&decoded[child].path, &parent_dir);
        let (child_id, child_agent) = {
            let child_session = &mut decoded[child].session;
            child_session.parent_id = Some(root_id.clone());
            child_session.root_id = Some(root_id.clone());
            child_session.agent_path = Some(agent_path.clone());
            (
                child_session.source_id.clone(),
                child_session.agent_id.clone(),
            )
        };
        let mut edge = AgentEdge::new(root_id.clone(), child_id);
        edge.agent_id = child_agent.clone();
        edge.agent_path = Some(agent_path);
        edge.association = "directory-fallback".to_string();
        edge.confidence = 0.25;
        children_of[0].push(child);
        edges_of[0].push(edge);
        let mut params = Map::new();
        params.insert(
            "child_id".into(),
            child_agent.map_or(Value::Null, Value::from),
        );
        decoded[0].session.lose("session.subagent_unlinked", params);
    }

    let mut slots: Vec<Option<Session>> =
        decoded.into_iter().map(|item| Some(item.session)).collect();
    Ok(assemble(0, &mut slots, &children_of, &mut edges_of))
}

fn relative_agent_path(child_path: &Path, parent_dir: &Path) -> String {
    child_path
        .strip_prefix(parent_dir)
        .unwrap_or(child_path)
        .to_string_lossy()
        .into_owned()
}

fn is_ancestor(parent_of: &[Option<usize>], candidate: usize, mut node: usize) -> bool {
    let mut steps = 0;
    while let Some(parent) = parent_of[node] {
        if parent == candidate {
            return true;
        }
        node = parent;
        steps += 1;
        if steps > parent_of.len() {
            return true;
        }
    }
    false
}

fn assemble(
    index: usize,
    slots: &mut [Option<Session>],
    children_of: &[Vec<usize>],
    edges_of: &mut [Vec<AgentEdge>],
) -> Session {
    let mut session = slots[index].take().expect("每个节点只装配一次");
    session.agent_edges = std::mem::take(&mut edges_of[index]);
    for child in &children_of[index] {
        let assembled = assemble(*child, slots, children_of, edges_of);
        session.children.push(assembled);
    }
    session
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write_session(path: &Path, records: &[Value]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let payload: String = records
            .iter()
            .map(|record| format!("{}\n", serde_json::to_string(record).unwrap()))
            .collect();
        std::fs::write(path, payload).unwrap();
    }

    #[test]
    fn tool_results_pair_with_their_calls() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("s.jsonl");
        write_session(
            &path,
            &[
                json!({"uuid": "u1", "type": "user", "cwd": "/w", "sessionId": "sid",
                       "message": {"role": "user", "content": "go"}}),
                json!({"uuid": "a1", "parentUuid": "u1", "type": "assistant",
                       "message": {"role": "assistant", "content": [
                           {"type": "tool_use", "id": "t1", "name": "Bash",
                            "input": {"command": "ls"}}]}}),
                json!({"uuid": "r1", "parentUuid": "a1", "type": "user",
                       "toolUseResult": {"status": "success", "stdout": "a", "exit_code": 0},
                       "message": {"role": "user", "content": [
                           {"type": "tool_result", "tool_use_id": "t1", "content": "a"}]}}),
            ],
        );
        let session = read(path.to_str().unwrap()).unwrap();
        assert_eq!(session.source_id, "sid");
        assert_eq!(session.cwd, "/w");
        // tool_result 载体没有可见文本 -> 整条消息丢弃。
        assert_eq!(session.messages.len(), 2);
        let tool = session.messages[1].blocks[0].tool.as_ref().unwrap();
        assert_eq!(tool.op.as_deref(), Some(CanonicalOp::SHELL_EXEC));
        assert_eq!(tool.input, json!({"command": "ls"}));
        let result = tool.result.as_ref().unwrap();
        assert_eq!(result.status, ToolResultStatus::Success);
        assert_eq!(result.stdout.as_deref(), Some("a"));
        assert_eq!(result.exit_code, Some(0));
        assert_eq!(tool.source_result_id.as_deref(), Some("r1"));
        assert!(session.loss.is_empty());
    }

    #[test]
    fn unknown_tools_fall_back_to_tool_invoke() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("s.jsonl");
        write_session(
            &path,
            &[json!({"uuid": "a1", "type": "assistant",
                     "message": {"role": "assistant", "content": [
                         {"type": "tool_use", "id": "t1", "name": "Mystery",
                          "input": {"x": 1}}]}})],
        );
        let session = read(path.to_str().unwrap()).unwrap();
        let tool = session.messages[0].blocks[0].tool.as_ref().unwrap();
        assert_eq!(tool.op.as_deref(), Some(CanonicalOp::TOOL_INVOKE));
        assert_eq!(
            tool.input,
            json!({"namespace": "claude", "name": "Mystery", "input": {"x": 1}})
        );
        assert_eq!(session.loss[0].code, "session.unpaired_tool_use");
        assert_eq!(session.loss[0].params["tool_name"], json!("Mystery"));
    }

    #[test]
    fn malformed_lines_and_orphan_results_are_reported() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("s.jsonl");
        std::fs::write(
            &path,
            "{not json}\n{\"uuid\": \"r\", \"type\": \"user\", \"message\": {\"role\": \"user\", \
             \"content\": [{\"type\": \"tool_result\", \"tool_use_id\": \"gone\"}, \
             {\"type\": \"text\", \"text\": \"hi\"}]}}\n",
        )
        .unwrap();
        let session = read(path.to_str().unwrap()).unwrap();
        let codes: Vec<&str> = session.loss.iter().map(|loss| loss.code.as_str()).collect();
        assert_eq!(
            codes,
            ["session.malformed_record", "session.orphan_tool_result"]
        );
        assert_eq!(session.loss[0].params["line_number"], json!(1));
        assert_eq!(session.loss[1].params["call_id"], json!("gone"));
        // 有可见文本 -> 载体消息保留。
        assert_eq!(session.messages.len(), 1);
    }

    #[test]
    fn thinking_blocks_degrade_to_text_or_drop() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("s.jsonl");
        write_session(
            &path,
            &[
                json!({"uuid": "a1", "type": "assistant", "message": {"role": "assistant",
                       "content": [{"type": "thinking", "thinking": "loud"}]}}),
                json!({"uuid": "a2", "type": "assistant", "message": {"role": "assistant",
                       "content": [{"type": "thinking", "thinking": "  "},
                                   {"type": "text", "text": "x"}]}}),
            ],
        );
        let session = read(path.to_str().unwrap()).unwrap();
        assert_eq!(session.messages[0].blocks[0].text, "loud");
        let codes: Vec<&str> = session.loss.iter().map(|loss| loss.code.as_str()).collect();
        assert_eq!(
            codes,
            [
                "migration.reasoning_metadata_dropped",
                "migration.reasoning_dropped"
            ]
        );
    }

    #[test]
    fn subagents_link_through_agent_ids_and_fall_back_to_the_directory() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("s.jsonl");
        write_session(
            &path,
            &[
                json!({"uuid": "u1", "type": "user", "sessionId": "sid", "cwd": "/w",
                       "message": {"role": "user", "content": "spawn"}}),
                json!({"uuid": "a1", "parentUuid": "u1", "type": "assistant",
                       "message": {"role": "assistant", "content": [
                           {"type": "tool_use", "id": "t1", "name": "Agent",
                            "input": {"description": "d", "prompt": "p",
                                      "subagent_type": "explorer"}}]}}),
                json!({"uuid": "r1", "parentUuid": "a1", "type": "user",
                       "toolUseResult": {"agentId": "ag1", "status": "completed"},
                       "message": {"role": "user", "content": [
                           {"type": "tool_result", "tool_use_id": "t1", "content": "done"}]}}),
            ],
        );
        let subagents = root.path().join("s/subagents");
        write_session(
            &subagents.join("agent-ag1.jsonl"),
            &[
                json!({"uuid": "c1", "type": "assistant", "agentId": "ag1", "isSidechain": true,
                     "message": {"role": "assistant", "content": [
                         {"type": "text", "text": "child"}]}}),
            ],
        );
        write_session(
            &subagents.join("agent-orphan.jsonl"),
            &[
                json!({"uuid": "c2", "type": "assistant", "isSidechain": true,
                     "message": {"role": "assistant", "content": [
                         {"type": "text", "text": "loose"}]}}),
            ],
        );

        let session = read(path.to_str().unwrap()).unwrap();
        assert_eq!(session.children.len(), 2);
        assert_eq!(session.agent_edges.len(), 2);
        let linked = &session.agent_edges[0];
        assert_eq!(linked.association, "agent-id");
        assert_eq!(linked.confidence, 1.0);
        assert_eq!(linked.agent_id.as_deref(), Some("ag1"));
        assert_eq!(linked.source_call_id.as_deref(), Some("t1"));
        assert_eq!(linked.spawn_message_id.as_deref(), Some("a1"));
        assert_eq!(linked.result_message_id.as_deref(), Some("r1"));
        assert_eq!(linked.agent_type.as_deref(), Some("explorer"));
        assert_eq!(linked.prompt, "p");
        assert_eq!(linked.status.as_deref(), Some("completed"));
        assert_eq!(
            linked.agent_path.as_deref(),
            Some("s/subagents/agent-ag1.jsonl")
        );

        let fallback = &session.agent_edges[1];
        assert_eq!(fallback.association, "directory-fallback");
        assert_eq!(fallback.confidence, 0.25);
        assert_eq!(fallback.agent_id.as_deref(), Some("orphan"));
        assert_eq!(session.loss[0].code, "session.subagent_unlinked");
        assert_eq!(session.loss[0].params["child_id"], json!("orphan"));
        assert_eq!(session.children[0].root_id.as_deref(), Some("sid"));
        assert_eq!(session.children[0].parent_id.as_deref(), Some("sid"));
    }

    #[test]
    fn compact_boundaries_and_summaries_pair_up() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("s.jsonl");
        write_session(
            &path,
            &[
                json!({"uuid": "u1", "type": "user", "sessionId": "sid",
                       "message": {"role": "user", "content": "hi"}}),
                json!({"uuid": "sys", "type": "system", "subtype": "turn_duration",
                       "parentUuid": "u1"}),
                json!({"uuid": "b1", "type": "system", "subtype": "compact_boundary",
                       "logicalParentUuid": "sys", "timestamp": "2026-01-01T00:00:00Z",
                       "compactMetadata": {"preTokens": 100, "postTokens": 40,
                                           "trigger": "auto", "durationMs": 12}}),
                json!({"uuid": "s1", "parentUuid": "b1", "type": "assistant",
                       "isCompactSummary": true,
                       "message": {"role": "assistant", "content": [
                           {"type": "text", "text": "summary"}]}}),
            ],
        );
        let session = read(path.to_str().unwrap()).unwrap();
        assert_eq!(session.context_compactions.len(), 1);
        let compaction = &session.context_compactions[0];
        assert_eq!(compaction.id, "b1");
        assert_eq!(compaction.trigger, "automatic");
        assert_eq!(compaction.state, "completed");
        assert_eq!(compaction.summary_status, "available");
        assert_eq!(compaction.summary_text, "summary");
        assert_eq!(compaction.summary_message_id.as_deref(), Some("s1"));
        // logicalParentUuid 指向 system 记录 -> 回溯到 u1。
        assert_eq!(compaction.after_message_id.as_deref(), Some("u1"));
        assert_eq!(compaction.metrics["dropped_tokens"], json!(60));
        assert_eq!(compaction.metrics["duration_ms"], json!(12));
        assert!(!compaction.metrics.contains_key("cumulative_dropped_tokens"));
    }

    #[test]
    fn ai_title_and_fork_context_are_picked_up() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("s.jsonl");
        write_session(
            &path,
            &[
                json!({"type": "ai-title", "title": "Named"}),
                json!({"type": "fork-context-ref", "parentLastUuid": "p9",
                       "parentSessionId": "psid"}),
                json!({"uuid": "u1", "type": "user", "sessionId": "sid",
                       "message": {"role": "user", "content": "hi"}}),
            ],
        );
        let session = read(path.to_str().unwrap()).unwrap();
        assert_eq!(session.title, "Named");
        assert_eq!(session.forked_from_id.as_deref(), Some("p9"));
        assert_eq!(session.parent_id.as_deref(), Some("psid"));
    }
}
