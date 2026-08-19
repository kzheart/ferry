//! OpenCode 当前原生结构到 Canonical Session 的读取转换。
//!
//! 读取全程只走只读 SQLite：官方 `opencode export` CLI **不参与**读路径
//! （预览也一样），避免读会话时拉起外部进程。

use std::collections::{BTreeMap, HashMap};

use rusqlite::Connection;
use serde_json::{Map, Value};

use crate::adapters::shared::media::image_from_data_url;
use crate::adapters::shared::tool_canon::{canonical_tool_input, canonical_tool_op};
use crate::errors::{DomainError, DomainResult};
use crate::model::{
    AgentEdge, Block, BlockKind, ContextCompaction, ImageAsset, Message, Session, Timestamp,
    ToolCall, ToolResult, ToolResultBlock, ToolResultBlockKind, ToolResultStatus,
};
use crate::tool_ops::CanonicalOp;

use super::store;

/// 与 `sessions::reasoning::visible_text` 同语义的本地实现。
///
/// `adapters` 不得引用 `crate::sessions`（分层规则），而 reasoning 的判定只有
/// 一行「非空白字符串」，这里就地实现比反向依赖划算。
fn visible_text(text: &Value) -> Option<&str> {
    text.as_str().filter(|value| !value.trim().is_empty())
}

/// Python `bool(value)` 的 JSON 等价。
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

fn object(value: Option<&Value>) -> Map<String, Value> {
    value
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_else(Map::new)
}

fn text_of(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::to_string)
}

fn timestamp(value: Option<&Value>) -> Option<Timestamp> {
    match value {
        Some(Value::String(text)) => Some(Timestamp::Text(text.clone())),
        Some(Value::Number(number)) => number.as_i64().map(Timestamp::Millis),
        _ => None,
    }
}

/// 原生工具名 + 入参 → `(规范操作, 规范入参)`。
///
/// `task` 强制走 `agent.spawn`：它在 opencode 里是子 Agent 派生而不是普通工具。
fn canonical_input(name: &str, source_input: &Value) -> (Option<String>, Value) {
    if name == "task" {
        return (
            Some(CanonicalOp::AGENT_SPAWN.to_string()),
            canonical_tool_input(name, source_input),
        );
    }
    match canonical_tool_op("opencode", name) {
        None => {
            let mut invoke = Map::new();
            invoke.insert("namespace".into(), Value::from("opencode"));
            invoke.insert("name".into(), Value::from(name));
            invoke.insert("input".into(), source_input.clone());
            (
                Some(CanonicalOp::TOOL_INVOKE.to_string()),
                Value::Object(invoke),
            )
        }
        Some(operation) => (
            Some(operation.to_string()),
            canonical_tool_input(name, source_input),
        ),
    }
}

/// opencode 的 `state` 段（原生 tool part 的执行状态）→ canonical ToolResult。
fn tool_result(state: &Map<String, Value>) -> ToolResult {
    const PROJECTED: [&str; 7] = [
        "input",
        "output",
        "error",
        "metadata",
        "attachments",
        "status",
        "time",
    ];
    let mut metadata = object(state.get("metadata"));
    // 表外的原生状态字段整体挂进 metadata，避免读一次就丢一次。
    let native_state: Map<String, Value> = state
        .iter()
        .filter(|(key, _)| !PROJECTED.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    if !native_state.is_empty() {
        metadata.insert("opencode_state".into(), Value::Object(native_state));
    }

    let mut status = match state.get("status").and_then(Value::as_str) {
        Some("completed") => ToolResultStatus::Success,
        Some("error") => ToolResultStatus::Error,
        Some("running") => ToolResultStatus::Running,
        Some("pending") => ToolResultStatus::Pending,
        _ => ToolResultStatus::Unknown,
    };
    if metadata.get("interrupted") == Some(&Value::Bool(true)) {
        status = ToolResultStatus::Interrupted;
    }

    let mut blocks: Vec<ToolResultBlock> = Vec::new();
    match state.get("output") {
        Some(Value::String(text)) => {
            if !text.is_empty() {
                blocks.push(ToolResultBlock::text(text.clone()));
            }
        }
        Some(other) if !other.is_null() => {
            let mut block = ToolResultBlock::new(ToolResultBlockKind::Json);
            block.data = other.clone();
            blocks.push(block);
        }
        _ => {}
    }

    let error = state.get("error").and_then(Value::as_str).unwrap_or("");
    if !error.is_empty() {
        if !blocks
            .iter()
            .any(|block| block.kind == ToolResultBlockKind::Text && block.text == error)
        {
            blocks.push(ToolResultBlock::text(error));
        }
        if status == ToolResultStatus::Unknown {
            status = ToolResultStatus::Error;
        }
    }

    let attachments: Vec<Value> = match state.get("attachments") {
        Some(Value::Array(items)) => items.clone(),
        Some(Value::Null) | None => Vec::new(),
        Some(other) => vec![other.clone()],
    };
    for attachment in &attachments {
        match attachment.as_object() {
            Some(entries) if entries.get("type") == Some(&Value::from("file")) => {
                let mut block = ToolResultBlock::new(ToolResultBlockKind::File);
                block.mime_type = text_of(entries.get("mime"));
                block.filename = text_of(entries.get("filename"));
                block.uri = text_of(entries.get("url"));
                blocks.push(block);
            }
            _ => {
                let mut block = ToolResultBlock::new(ToolResultBlockKind::Json);
                block.data = attachment.clone();
                blocks.push(block);
            }
        }
    }

    // exit 必须是真整数：Python 显式拒 bool，JSON 里 bool 与 number 天然分家。
    let exit_code = metadata.get("exit").and_then(Value::as_i64);
    let truncated = metadata.get("truncated").and_then(Value::as_bool);
    let stdout = text_of(metadata.get("stdout"));
    let stderr = if error.is_empty() {
        text_of(metadata.get("stderr"))
    } else {
        Some(error.to_string())
    };
    ToolResult {
        status,
        blocks,
        stdout,
        stderr,
        exit_code,
        truncated,
        attachments,
    }
}

/// 第一条带模型信息的消息决定整个会话的模型标注。
fn message_model(data: &Value) -> (Option<String>, Option<String>) {
    for message in data
        .get("messages")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let info = object(message.get("info"));
        let (provider, model) = match info.get("model") {
            Some(Value::Object(entries)) => (
                text_of(entries.get("providerID")),
                text_of(entries.get("modelID")),
            ),
            _ => (
                text_of(info.get("providerID")),
                text_of(info.get("modelID")),
            ),
        };
        let provider = provider.filter(|value| !value.is_empty());
        let model = model.filter(|value| !value.is_empty());
        if provider.is_some() || model.is_some() {
            return (provider, model);
        }
    }
    (None, None)
}

fn losing(session: &mut Session, code: &str, key: &str, value: Value) {
    let mut params = Map::new();
    params.insert(key.into(), value);
    session.lose(code, params);
}

/// 官方 export payload → `(Session, task 边)`。
pub fn parse_session(data: &Value) -> DomainResult<(Session, Vec<AgentEdge>)> {
    let root_info = data
        .get("info")
        .and_then(Value::as_object)
        .ok_or_else(|| DomainError::internal("OpenCode payload 缺少 info"))?
        .clone();
    let session_id = text_of(root_info.get("id")).unwrap_or_default();
    let (model_provider, model) = message_model(data);
    let mut session = Session::new(
        "opencode",
        session_id.clone(),
        root_info
            .get("directory")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    );
    session.title = root_info
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    session.parent_id = text_of(root_info.get("parentID"));
    session.agent_id = text_of(root_info.get("agent"));
    session.model_provider = model_provider;
    session.model = model;

    let native_messages: Vec<Value> = data
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    // Python 的 enumerate(start=1)：压缩尾部定位符是「第 N 条消息」而不是下标。
    let raw_message_indexes: HashMap<String, i64> = native_messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| {
            text_of(object(message.get("info")).get("id")).map(|id| (id, index as i64 + 1))
        })
        .collect();

    let mut edges: Vec<AgentEdge> = Vec::new();
    // 压缩事件的「待配摘要」队列，存的是 session.context_compactions 的下标。
    let mut pending_compactions: Vec<usize> = Vec::new();
    let mut last_visible_message_id: Option<String> = None;

    for (ordinal, native_message) in native_messages.iter().enumerate() {
        let info = object(native_message.get("info"));
        let role = info
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("user")
            .to_string();
        let message_id = text_of(info.get("id"));
        let parts: Vec<Value> = native_message
            .get("parts")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        if let Some(compaction_part) = parts
            .iter()
            .find(|part| part.get("type") == Some(&Value::from("compaction")))
            .and_then(Value::as_object)
        {
            let tail_locator = text_of(compaction_part.get("tail_start_id"));
            let tail_index = tail_locator
                .as_deref()
                .and_then(|locator| raw_message_indexes.get(locator).copied());
            let mut compaction = ContextCompaction::new(
                message_id
                    .clone()
                    .unwrap_or_else(|| format!("compaction:{ordinal}")),
                "opencode",
            );
            compaction.after_message_id = last_visible_message_id.clone();
            compaction.event_locator = message_id.clone();
            compaction.created_at = timestamp(object(info.get("time")).get("created"));
            compaction.trigger = match compaction_part.get("auto") {
                Some(Value::Bool(true)) => "automatic".into(),
                Some(Value::Bool(false)) => "manual".into(),
                _ => "unknown".into(),
            };
            compaction.state = "incomplete".into();
            compaction.tail_status = if tail_index.is_some() {
                "located".into()
            } else {
                "unknown".into()
            };
            compaction.tail_start_locator = tail_locator;
            compaction.tail_start_message_index = tail_index;
            compaction.source_meta = compaction_part
                .iter()
                .filter(|(key, _)| key.as_str() != "type" && key.as_str() != "tail_start_id")
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            session.context_compactions.push(compaction);
            pending_compactions.push(session.context_compactions.len() - 1);
        }

        let is_summary = info.get("mode") == Some(&Value::from("compaction"))
            || info.get("summary") == Some(&Value::Bool(true));
        if is_summary {
            let summary = parts
                .iter()
                .filter(|part| {
                    part.get("type") == Some(&Value::from("text"))
                        && part
                            .get("text")
                            .is_some_and(|text| text.as_str().is_some_and(|text| !text.is_empty()))
                })
                .map(|part| {
                    part.get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string()
                })
                .collect::<Vec<_>>()
                .join("\n");
            if let Some(index) = pending_compactions.iter().rev().copied().find(|index| {
                session.context_compactions[*index]
                    .summary_message_id
                    .is_none()
            }) {
                let compaction = &mut session.context_compactions[index];
                compaction.summary_message_id = message_id.clone();
                compaction.summary_text = summary.clone();
                compaction.summary_status = if summary.is_empty() {
                    "missing".into()
                } else {
                    "available".into()
                };
                compaction.state = if summary.is_empty() {
                    "incomplete".into()
                } else {
                    "completed".into()
                };
            }
        }

        let mut blocks: Vec<Block> = Vec::new();
        for (part_ordinal, part) in parts.iter().enumerate() {
            match part.get("type").and_then(Value::as_str) {
                Some("text") => blocks.push(Block::text(
                    part.get("text").and_then(Value::as_str).unwrap_or(""),
                )),
                Some("file")
                    if part
                        .get("mime")
                        .map(crate::adapters::shared::dialect::python_str)
                        .unwrap_or_default()
                        .starts_with("image/") =>
                {
                    let asset_id = format!(
                        "{}:image:{part_ordinal}",
                        message_id.clone().unwrap_or_else(|| "None".into())
                    );
                    let image: Option<ImageAsset> = image_from_data_url(
                        &asset_id,
                        part.get("url").unwrap_or(&Value::from("")),
                        part.get("filename").and_then(Value::as_str),
                    );
                    match image {
                        None => losing(
                            &mut session,
                            "migration.unknown_block_dropped",
                            "kind",
                            Value::from("file"),
                        ),
                        Some(image) => {
                            let mut block = Block::new(BlockKind::Image);
                            block.image = Some(image);
                            blocks.push(block);
                        }
                    }
                }
                Some("reasoning") => match visible_text(part.get("text").unwrap_or(&Value::Null)) {
                    Some(text) => {
                        blocks.push(Block::text(text));
                        losing(
                            &mut session,
                            "migration.reasoning_metadata_dropped",
                            "metadata_kind",
                            Value::from("metadata"),
                        );
                    }
                    None => losing(
                        &mut session,
                        "migration.reasoning_dropped",
                        "metadata_kind",
                        Value::from("metadata"),
                    ),
                },
                Some("tool") => {
                    let state = object(part.get("state"));
                    let name = part.get("tool").and_then(Value::as_str).unwrap_or("?");
                    let raw_input = match state.get("input") {
                        Some(Value::Null) | None => Value::Object(Map::new()),
                        Some(Value::Bool(false)) => Value::Object(Map::new()),
                        Some(other) => other.clone(),
                    };
                    let (operation, inputs) = canonical_input(name, &raw_input);
                    let mut tool = ToolCall::new(name, operation, inputs.clone());
                    tool.source_call_id = text_of(part.get("callID"));
                    let time = object(state.get("time"));
                    tool.started_at = timestamp(time.get("start"));
                    tool.ended_at = timestamp(time.get("end"));
                    tool.result = Some(tool_result(&state));
                    let mut block = Block::new(BlockKind::Tool);
                    block.tool = Some(tool);
                    blocks.push(block);

                    let child_id = text_of(object(state.get("metadata")).get("sessionId"))
                        .filter(|value| !value.is_empty());
                    if name == "task" {
                        if let Some(child_id) = child_id {
                            let mut edge = AgentEdge::new(session_id.clone(), child_id);
                            edge.source_call_id = text_of(part.get("callID"));
                            edge.spawn_message_id = message_id.clone();
                            let subagent = text_of(inputs.get("subagent_type"));
                            edge.agent_id = subagent.clone();
                            edge.agent_type = subagent;
                            edge.prompt = inputs
                                .get("prompt")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string();
                            edge.status = text_of(state.get("status"));
                            edge.association = "task-metadata".into();
                            edge.confidence = 1.0;
                            edges.push(edge);
                        }
                    }
                }
                Some("step-start") | Some("step-finish") => {}
                other => losing(
                    &mut session,
                    "migration.unknown_block_dropped",
                    "kind",
                    other.map_or(Value::Null, Value::from),
                ),
            }
        }

        if !blocks.is_empty() {
            let mut message = Message::new(role);
            message.blocks = blocks;
            message.source_id = message_id.clone();
            message.parent_ids = text_of(info.get("parentID"))
                .filter(|value| !value.is_empty())
                .map(|parent| vec![parent])
                .unwrap_or_default();
            message.agent_id = text_of(info.get("agent"));
            message.created_at = timestamp(object(info.get("time")).get("created"));
            session.messages.push(message);
            if !is_summary {
                last_visible_message_id = message_id;
            }
        }
    }

    let compacting = object(root_info.get("time")).get("compacting").cloned();
    // Python 的 `if compacting and ...`：0 / "" / null 都不算「正在压缩」。
    let compacting_present = compacting.as_ref().is_some_and(truthy);
    if compacting_present
        && !session
            .context_compactions
            .iter()
            .any(|compaction| compaction.state == "in_progress")
    {
        let mut compaction = ContextCompaction::new(format!("{session_id}:compacting"), "opencode");
        compaction.after_message_id = last_visible_message_id;
        compaction.created_at = timestamp(compacting.as_ref());
        compaction.state = "in_progress".into();
        session.context_compactions.push(compaction);
    }
    Ok((session, edges))
}

/// 会话子树读取：SQLite 的 `parent_id` 关系与 task part 的 metadata 双向取并。
fn read_tree(session_id: &str) -> DomainResult<Session> {
    let connection = store::open_database()?;
    let mut seen: BTreeMap<String, Session> = BTreeMap::new();
    let root = visit(&connection, session_id, session_id, &mut seen)?;
    Ok(root)
}

fn export(connection: &Connection, identifier: &str) -> DomainResult<Value> {
    store::export_from_database(connection, identifier)?
        .ok_or_else(|| DomainError::session_not_found("opencode", identifier))
}

fn child_ids_of(connection: &Connection, identifier: &str) -> DomainResult<Vec<String>> {
    let query = |identifier: &str| -> rusqlite::Result<Vec<String>> {
        let mut statement = connection
            .prepare("SELECT id FROM session WHERE parent_id = ? ORDER BY time_created, id")?;
        let rows = statement.query_map([identifier], |row| row.get::<_, String>(0))?;
        rows.collect()
    };
    query(identifier).map_err(|error| {
        DomainError::agent_format_changed(
            "opencode",
            "sqlite.session.parent_id",
            Value::from("queryable parent-child relation"),
            Value::from(error.to_string()),
        )
    })
}

fn visit(
    connection: &Connection,
    identifier: &str,
    root_id: &str,
    seen: &mut BTreeMap<String, Session>,
) -> DomainResult<Session> {
    let (mut session, task_edges) = parse_session(&export(connection, identifier)?)?;
    session.root_id = Some(root_id.to_string());
    // 环保护：先登记一个占位，子树里再遇到自己就不会无限递归。
    seen.insert(identifier.to_string(), session.clone());

    let mut task_by_child: Vec<(String, Vec<AgentEdge>)> = Vec::new();
    for edge in task_edges {
        match task_by_child
            .iter_mut()
            .find(|(child, _)| *child == edge.child_session_id)
        {
            Some((_, bucket)) => bucket.push(edge),
            None => task_by_child.push((edge.child_session_id.clone(), vec![edge])),
        }
    }

    let database_child_ids = child_ids_of(connection, identifier)?;
    let mut child_ids = database_child_ids.clone();
    for (child_id, _) in &task_by_child {
        if !child_ids.contains(child_id) {
            child_ids.push(child_id.clone());
        }
    }

    for child_id in child_ids {
        if seen.contains_key(&child_id) {
            continue;
        }
        let mut child = visit(connection, &child_id, root_id, seen)?;
        if !database_child_ids.contains(&child_id) && child.parent_id.as_deref() != Some(identifier)
        {
            losing(
                &mut session,
                "session.child_foreign_ignored",
                "child_id",
                Value::from(child_id.as_str()),
            );
            continue;
        }
        // Python 的 `if child.parent_id and ...`：空串 parent_id 是假值，不算冲突。
        if child
            .parent_id
            .as_deref()
            .filter(|parent| !parent.is_empty())
            .is_some_and(|parent| parent != identifier)
        {
            losing(
                &mut session,
                "session.child_parent_conflict",
                "child_id",
                Value::from(child_id.as_str()),
            );
            continue;
        }
        child.parent_id = Some(identifier.to_string());
        let child_agent_id = child.agent_id.clone();
        let child_agent_path = child.agent_path.clone();
        let child_agent_type = child.agent_type.clone();
        session.children.push(child);

        let mut child_edges = task_by_child
            .iter()
            .find(|(child, _)| *child == child_id)
            .map(|(_, edges)| edges.clone())
            .unwrap_or_default();
        if child_edges.is_empty() {
            let mut edge = AgentEdge::new(identifier, child_id.as_str());
            edge.association = "sqlite-parent".into();
            edge.confidence = 0.9;
            child_edges.push(edge);
        }
        for mut edge in child_edges {
            edge.agent_id = edge.agent_id.or_else(|| child_agent_id.clone());
            edge.agent_path = edge.agent_path.or_else(|| child_agent_path.clone());
            edge.agent_type = edge.agent_type.or_else(|| child_agent_type.clone());
            session.agent_edges.push(edge);
        }
    }
    seen.insert(identifier.to_string(), session.clone());
    Ok(session)
}

/// 读取会话子树。
pub fn read(session_id: &str) -> DomainResult<Session> {
    read_tree(session_id)
}

/// Agent 预览读取；OpenCode 的预览与正式读取走同一条只读链路。
pub fn read_preview(session_id: &str) -> DomainResult<Session> {
    read_tree(session_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::shared::dialect::register_dialect;
    use serde_json::json;

    pub(crate) fn register() {
        register_dialect("opencode", &super::super::dialect::DIALECT);
    }

    #[test]
    fn model_is_taken_from_the_first_annotated_message() {
        register();
        let payload = json!({
            "info": {"id": "session-1", "directory": "/tmp"},
            "messages": [{
                "info": {"id": "message-1", "sessionID": "session-1", "role": "user",
                         "model": {"providerID": "fixture-provider",
                                   "modelID": "fixture-model"}},
                "parts": [{"id": "part-1", "messageID": "message-1",
                           "sessionID": "session-1", "type": "text", "text": "original"}]
            }]
        });
        let (session, edges) = parse_session(&payload).unwrap();
        assert_eq!(session.model_provider.as_deref(), Some("fixture-provider"));
        assert_eq!(session.model.as_deref(), Some("fixture-model"));
        assert_eq!(
            session
                .messages
                .iter()
                .map(|message| message.source_id.clone())
                .collect::<Vec<_>>(),
            [Some("message-1".to_string())]
        );
        assert!(edges.is_empty());
    }

    #[test]
    fn a_session_without_model_annotations_stays_explicitly_empty() {
        register();
        let (session, _) = parse_session(&json!({"info": {"id": "session-1", "directory": "/tmp"},
                                  "messages": []}))
        .unwrap();
        assert_eq!(session.model_provider, None);
        assert_eq!(session.model, None);
    }

    #[test]
    fn tool_state_projects_status_streams_and_attachments() {
        register();
        let payload = json!({
            "info": {"id": "s", "directory": "/tmp"},
            "messages": [{
                "info": {"id": "m", "role": "assistant"},
                "parts": [{
                    "id": "p", "type": "tool", "tool": "bash", "callID": "c1",
                    "state": {"status": "error", "input": {"command": "ls"},
                              "output": "partial", "error": "boom",
                              "metadata": {"exit": 2, "truncated": true,
                                           "stdout": "out"},
                              "attachments": [{"type": "file", "mime": "text/plain",
                                               "filename": "a.txt", "url": "file:///a"}],
                              "time": {"start": 10, "end": 20},
                              "extra": {"native": 1}}
                }]
            }]
        });
        let (session, _) = parse_session(&payload).unwrap();
        let tool = session.messages[0].blocks[0].tool.as_ref().unwrap();
        assert_eq!(tool.op.as_deref(), Some(CanonicalOp::SHELL_EXEC));
        assert_eq!(tool.input, json!({"command": "ls"}));
        assert_eq!(tool.started_at, Some(Timestamp::Millis(10)));
        let result = tool.result.as_ref().unwrap();
        assert_eq!(result.status, ToolResultStatus::Error);
        assert_eq!(result.exit_code, Some(2));
        assert_eq!(result.truncated, Some(true));
        assert_eq!(result.stdout.as_deref(), Some("out"));
        // error 非空时 stderr 取 error 而不是 metadata.stderr。
        assert_eq!(result.stderr.as_deref(), Some("boom"));
        let kinds: Vec<ToolResultBlockKind> = result.blocks.iter().map(|b| b.kind).collect();
        assert_eq!(
            kinds,
            [
                ToolResultBlockKind::Text,
                ToolResultBlockKind::Text,
                ToolResultBlockKind::File
            ]
        );
    }

    #[test]
    fn interrupted_metadata_overrides_the_native_status() {
        let mut state = Map::new();
        state.insert("status".into(), json!("completed"));
        state.insert("metadata".into(), json!({"interrupted": true}));
        assert_eq!(tool_result(&state).status, ToolResultStatus::Interrupted);
        // 未知 status + 非空 error → error。
        let mut broken = Map::new();
        broken.insert("error".into(), json!("nope"));
        assert_eq!(tool_result(&broken).status, ToolResultStatus::Error);
        // opencode_state 收纳表外字段。
        let mut extra = Map::new();
        extra.insert("status".into(), json!("completed"));
        extra.insert("title".into(), json!("t"));
        let result = tool_result(&extra);
        assert_eq!(result.status, ToolResultStatus::Success);
        assert!(result.blocks.is_empty());
    }

    #[test]
    fn unknown_parts_and_reasoning_are_recorded_as_loss() {
        register();
        let payload = json!({
            "info": {"id": "s", "directory": "/tmp"},
            "messages": [{
                "info": {"id": "m", "role": "assistant"},
                "parts": [
                    {"type": "step-start"},
                    {"type": "reasoning", "text": "  "},
                    {"type": "reasoning", "text": "thinking"},
                    {"type": "snapshot"}
                ]
            }]
        });
        let (session, _) = parse_session(&payload).unwrap();
        let codes: Vec<&str> = session.loss.iter().map(|item| item.code.as_str()).collect();
        assert_eq!(
            codes,
            [
                "migration.reasoning_dropped",
                "migration.reasoning_metadata_dropped",
                "migration.unknown_block_dropped"
            ]
        );
        assert_eq!(session.loss[2].params["kind"], json!("snapshot"));
        assert_eq!(session.messages[0].blocks.len(), 1);
    }

    #[test]
    fn task_parts_emit_agent_edges_with_full_confidence() {
        register();
        let payload = json!({
            "info": {"id": "parent", "directory": "/tmp"},
            "messages": [{
                "info": {"id": "spawn", "role": "assistant"},
                "parts": [{
                    "id": "p", "type": "tool", "tool": "task", "callID": "call-1",
                    "state": {"status": "completed",
                              "input": {"prompt": "review", "subagent_type": "critic"},
                              "metadata": {"sessionId": "child"}}
                }]
            }]
        });
        let (_, edges) = parse_session(&payload).unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].child_session_id, "child");
        assert_eq!(edges[0].source_call_id.as_deref(), Some("call-1"));
        assert_eq!(edges[0].spawn_message_id.as_deref(), Some("spawn"));
        assert_eq!(edges[0].agent_type.as_deref(), Some("critic"));
        assert_eq!(edges[0].prompt, "review");
        assert_eq!(edges[0].association, "task-metadata");
    }

    #[test]
    fn compaction_parts_pair_with_the_following_summary_message() {
        register();
        let payload = json!({
            "info": {"id": "s", "directory": "/tmp", "time": {}},
            "messages": [
                {"info": {"id": "m0", "role": "user"},
                 "parts": [{"type": "text", "text": "hi"}]},
                {"info": {"id": "m1", "role": "assistant"},
                 "parts": [{"type": "compaction", "auto": true,
                            "tail_start_id": "m0", "note": "keep"}]},
                {"info": {"id": "m2", "role": "assistant", "mode": "compaction"},
                 "parts": [{"type": "text", "text": "summary"}]}
            ]
        });
        let (session, _) = parse_session(&payload).unwrap();
        let compaction = &session.context_compactions[0];
        assert_eq!(compaction.trigger, "automatic");
        assert_eq!(compaction.tail_status, "located");
        // enumerate(start=1)：m0 是第 1 条。
        assert_eq!(compaction.tail_start_message_index, Some(1));
        assert_eq!(compaction.after_message_id.as_deref(), Some("m0"));
        assert_eq!(compaction.summary_message_id.as_deref(), Some("m2"));
        assert_eq!(compaction.summary_text, "summary");
        assert_eq!(compaction.state, "completed");
        assert_eq!(compaction.source_meta["note"], json!("keep"));
        assert!(!compaction.source_meta.contains_key("tail_start_id"));
    }

    #[test]
    fn an_active_compaction_is_synthesised_from_the_session_clock() {
        register();
        let payload = json!({
            "info": {"id": "s", "directory": "/tmp", "time": {"compacting": 999}},
            "messages": [{"info": {"id": "m0", "role": "user"},
                          "parts": [{"type": "text", "text": "hi"}]}]
        });
        let (session, _) = parse_session(&payload).unwrap();
        assert_eq!(session.context_compactions.len(), 1);
        let compaction = &session.context_compactions[0];
        assert_eq!(compaction.id, "s:compacting");
        assert_eq!(compaction.state, "in_progress");
        assert_eq!(compaction.after_message_id.as_deref(), Some("m0"));
        assert_eq!(compaction.created_at, Some(Timestamp::Millis(999)));
    }

    /// 黄金对照：完整 read 链路（SQLite → export → canonical Session）必须与
    /// `tests/golden/canonical/opencode/` 的基线逐字段一致。
    ///
    /// fixture 的 `session.json` 是三张表的导出行，按 `store` 声明的当前列集合
    /// 还原成只读库后走真实读路径（物化方式与 `tests/golden_regen.rs` 一致）。
    #[test]
    fn canonical_sessions_match_the_golden_baseline() {
        register();
        let _guard = super::store::tests::exclusive();
        for case in ["case-01-plain", "case-02-tools"] {
            let fixture = super::store::tests::fixture(case);
            let root = tempfile::tempdir().unwrap();
            let database = root.path().join("opencode.db");
            super::store::tests::materialize(&database, &fixture);
            super::store::set_database_path_override(Some(database));
            let session = read(fixture["session"]["id"].as_str().unwrap());
            super::store::set_database_path_override(None);

            let golden: Value = serde_json::from_str(
                &std::fs::read_to_string(
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../../tests/golden/canonical/opencode")
                        .join(format!("{case}.json")),
                )
                .expect("黄金基线可读"),
            )
            .unwrap();
            assert_eq!(
                serde_json::to_value(session.expect("fixture 可读")).unwrap(),
                golden,
                "{case} 的 canonical Session 与黄金基线不一致"
            );
        }
    }

    #[test]
    fn unmapped_tools_fall_back_to_tool_invoke() {
        register();
        let (operation, inputs) = canonical_input("mystery", &json!({"a": 1}));
        assert_eq!(operation.as_deref(), Some(CanonicalOp::TOOL_INVOKE));
        assert_eq!(
            inputs,
            json!({"namespace": "opencode", "name": "mystery", "input": {"a": 1}})
        );
    }
}
