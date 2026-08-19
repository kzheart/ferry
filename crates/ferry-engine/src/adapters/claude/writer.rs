//! Claude Code writer：规范化会话树 → 主会话与 subagent JSONL。
//!
//! 语义事实源：`engine/adapters/claude/writer.py`。
//!
//! 与 Python 的一处可见差异：Python 的 `write()` 直接往调用方的 `Session` 上
//! `lose(...)`，因此写入阶段产生的 `migration.tool_degraded` /
//! `migration.fork_parent_fallback` 会被随后的 `plan()` 再数一遍。WP-A 定型的
//! `MigrationTarget::write(&self, session: &Session, ...)` 是只读入参，Rust 侧
//! 把这些损耗收进本地 sink 后丢弃（plan/preview 已经独立统计过同一批降级）。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::adapters::shared::migration::RenderDecision;
use crate::adapters::shared::narration::narrate;
use crate::adapters::shared::writing::{python_json_dumps, write_jsonl};
use crate::errors::{DomainError, DomainResult};
use crate::events::Event;
use crate::model::{
    AgentEdge, BlockKind, Message, Session, ToolCall, ToolResult, ToolResultBlock,
    ToolResultBlockKind, ToolResultStatus,
};
use crate::system::paths::home_dir;
use crate::tool_ops::{has_valid_tool_input, CanonicalOp};

use super::dialect::DIALECT;
use super::editing::{epoch_seconds_now, utc_iso_seconds, uuid4, uuid4_hex};
use super::native_schema::templates;

/// plan / preview / writer 三路共用的调用级判定。
pub type ToolDecider<'a> =
    dyn Fn(&ToolCall, &Session, Option<&Message>) -> DomainResult<RenderDecision> + 'a;

/// 方言里有可写绑定的规范操作一律 native；`fs.patch` / `tool.invoke` 只能降级。
pub fn op_is_native(op: &str) -> bool {
    op != CanonicalOp::FS_PATCH
        && op != CanonicalOp::TOOL_INVOKE
        && DIALECT.write_ops().contains(op)
}

fn block_kind_name(kind: ToolResultBlockKind) -> &'static str {
    match kind {
        ToolResultBlockKind::Text => "text",
        ToolResultBlockKind::Json => "json",
        ToolResultBlockKind::Image => "image",
        ToolResultBlockKind::File => "file",
        ToolResultBlockKind::ToolReference => "tool_reference",
    }
}

fn status_name(status: ToolResultStatus) -> &'static str {
    match status {
        ToolResultStatus::Success => "success",
        ToolResultStatus::Error => "error",
        ToolResultStatus::Interrupted => "interrupted",
        ToolResultStatus::Running => "running",
        ToolResultStatus::Pending => "pending",
        ToolResultStatus::Unknown => "unknown",
    }
}

fn optional(value: Option<&String>) -> Value {
    value.map_or(Value::Null, |text| Value::from(text.as_str()))
}

/// canonical `agent.spawn` 入参 → claude 原生 `Agent` 入参。
fn agent_input(value: &Value) -> Value {
    let entries = value.as_object().cloned().unwrap_or_default();
    let mut native = Map::new();
    native.insert(
        "description".into(),
        entries
            .get("description")
            .cloned()
            .unwrap_or_else(|| Value::from("")),
    );
    native.insert(
        "prompt".into(),
        entries
            .get("prompt")
            .cloned()
            .unwrap_or_else(|| Value::from("")),
    );
    if let Some(subagent) = entries.get("subagent_type").filter(|value| truthy(value)) {
        native.insert("subagent_type".into(), subagent.clone());
    }
    for (canonical, key) in [
        ("task_name", "name"),
        ("model", "model"),
        ("fork_mode", "mode"),
        ("reasoning_effort", "reasoning_effort"),
    ] {
        if let Some(value) = entries.get(canonical) {
            native.insert(key.into(), value.clone());
        }
    }
    Value::Object(native)
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

/// `re.sub(r"[^A-Za-z0-9]", "-", str(Path(path).resolve()))`。
fn slug(path: &str) -> String {
    resolve_lexically(Path::new(path))
        .to_string_lossy()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect()
}

/// `Path.resolve(strict=False)`：能 canonicalize 就 canonicalize，否则词法归一。
fn resolve_lexically(path: &Path) -> PathBuf {
    if let Ok(resolved) = std::fs::canonicalize(path) {
        return resolved;
    }
    let mut absolute = if path.is_absolute() {
        PathBuf::new()
    } else {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"))
    };
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                absolute.pop();
            }
            other => absolute.push(other.as_os_str()),
        }
    }
    absolute
}

fn result_block_payload(block: &ToolResultBlock) -> Value {
    let mut payload = Map::new();
    match block.kind {
        ToolResultBlockKind::Text => {
            payload.insert("type".into(), Value::from("text"));
            payload.insert("text".into(), Value::from(block.text.as_str()));
            return Value::Object(payload);
        }
        ToolResultBlockKind::Image if !block.data.is_null() => {
            let mut source = Map::new();
            source.insert("type".into(), Value::from("base64"));
            source.insert(
                "media_type".into(),
                Value::from(
                    block
                        .mime_type
                        .as_deref()
                        .filter(|mime| !mime.is_empty())
                        .unwrap_or("application/octet-stream"),
                ),
            );
            source.insert("data".into(), block.data.clone());
            payload.insert("type".into(), Value::from("image"));
            payload.insert("source".into(), Value::Object(source));
            return Value::Object(payload);
        }
        ToolResultBlockKind::ToolReference => {
            payload.insert("type".into(), Value::from("tool_reference"));
            if let Some(reference) = block.data.as_object() {
                for (key, value) in reference {
                    payload.insert(key.clone(), value.clone());
                }
            }
            return Value::Object(payload);
        }
        _ => {}
    }
    let mut fallback = Map::new();
    fallback.insert("kind".into(), Value::from(block_kind_name(block.kind)));
    fallback.insert("data".into(), block.data.clone());
    fallback.insert("mime_type".into(), optional(block.mime_type.as_ref()));
    fallback.insert("filename".into(), optional(block.filename.as_ref()));
    fallback.insert("uri".into(), optional(block.uri.as_ref()));
    payload.insert("type".into(), Value::from("text"));
    payload.insert(
        "text".into(),
        Value::from(python_json_dumps(&Value::Object(fallback))),
    );
    Value::Object(payload)
}

/// `(tool_result.content, toolUseResult)`。
fn claude_result(tool: &ToolCall) -> (Value, Value) {
    let Some(result) = tool.result.as_ref() else {
        let mut native = Map::new();
        native.insert("status".into(), Value::from("unknown"));
        native.insert("stdout".into(), Value::from(""));
        native.insert("stderr".into(), Value::from(""));
        native.insert("interrupted".into(), Value::Bool(false));
        native.insert("isImage".into(), Value::Bool(false));
        return (Value::from(""), Value::Object(native));
    };
    let content: Vec<Value> = result.blocks.iter().map(result_block_payload).collect();
    let mut native = Map::new();
    native.insert("status".into(), Value::from(status_name(result.status)));
    native.insert(
        "interrupted".into(),
        Value::Bool(result.status == ToolResultStatus::Interrupted),
    );
    native.insert(
        "isImage".into(),
        Value::Bool(
            result
                .blocks
                .iter()
                .any(|block| block.kind == ToolResultBlockKind::Image),
        ),
    );
    if let Some(stdout) = &result.stdout {
        native.insert("stdout".into(), Value::from(stdout.as_str()));
    }
    if let Some(stderr) = &result.stderr {
        native.insert("stderr".into(), Value::from(stderr.as_str()));
    }
    if let Some(exit_code) = result.exit_code {
        native.insert("exit_code".into(), Value::from(exit_code));
    }
    if let Some(truncated) = result.truncated {
        native.insert("truncated".into(), Value::Bool(truncated));
    }
    (Value::Array(content), Value::Object(native))
}

/// 为整棵树里的每个子会话签发新的 agent id。
fn agent_ids(session: &Session) -> HashMap<String, String> {
    let mut result = HashMap::new();
    for parent in session.walk() {
        let mut edges: HashMap<&str, &AgentEdge> = HashMap::new();
        for edge in &parent.agent_edges {
            edges.insert(edge.child_session_id.as_str(), edge);
        }
        for child in &parent.children {
            let new_id = format!("a{}", &uuid4_hex()[..16]);
            result.insert(child.source_id.clone(), new_id.clone());
            if let Some(agent_id) = &child.agent_id {
                result.insert(agent_id.clone(), new_id.clone());
            }
            if let Some(agent_id) = edges
                .get(child.source_id.as_str())
                .and_then(|edge| edge.agent_id.as_ref())
            {
                result.insert(agent_id.clone(), new_id.clone());
            }
        }
    }
    result
}

/// Claude CLI 拒绝 resume 缺少这些会话字段的记录。
fn ensure_resume_fields(record: &mut Map<String, Value>, cwd: Option<&str>, stamp: Option<&str>) {
    let kind = record
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if matches!(kind.as_str(), "user" | "assistant" | "system") {
        if !record.contains_key("timestamp") {
            record.insert(
                "timestamp".into(),
                Value::from(stamp.map_or_else(default_stamp, str::to_string)),
            );
        }
        record
            .entry("userType")
            .or_insert_with(|| Value::from("external"));
        if let Some(cwd) = cwd.filter(|cwd| !cwd.is_empty()) {
            record.insert("cwd".into(), Value::from(cwd));
        }
        if !record.contains_key("version") {
            record.insert("version".into(), Value::from("ferry"));
        }
        record
            .entry("isSidechain")
            .or_insert_with(|| Value::Bool(false));
    } else if matches!(
        kind.as_str(),
        "queue-operation" | "progress" | "last-prompt"
    ) {
        if let Some(stamp) = stamp {
            record
                .entry("timestamp")
                .or_insert_with(|| Value::from(stamp));
        }
    }
    if let Some(cwd) = cwd.filter(|cwd| !cwd.is_empty()) {
        if record.contains_key("cwd") {
            record.insert("cwd".into(), Value::from(cwd));
        }
    }
}

fn default_stamp() -> String {
    format!("{}.000Z", utc_iso_seconds(epoch_seconds_now() as i64))
}

/// 单个会话节点的记录生成器。
struct Generator<'a> {
    templates: &'a Map<String, Value>,
    sid: &'a str,
    cwd: String,
    agent_id: Option<String>,
    records: Vec<Value>,
    parent: Option<String>,
    timestamp: f64,
    emitted_children: HashSet<String>,
}

impl<'a> Generator<'a> {
    fn base(&mut self, kind: &str) -> Map<String, Value> {
        let mut record = self.templates[kind]
            .as_object()
            .cloned()
            .unwrap_or_default();
        let uuid = uuid4();
        record.insert("uuid".into(), Value::from(uuid.as_str()));
        record.insert(
            "parentUuid".into(),
            self.parent.as_deref().map_or(Value::Null, Value::from),
        );
        record.insert("sessionId".into(), Value::from(self.sid));
        record.insert("cwd".into(), Value::from(self.cwd.as_str()));
        record.insert("isSidechain".into(), Value::Bool(self.agent_id.is_some()));
        match &self.agent_id {
            Some(agent_id) => {
                record.insert("agentId".into(), Value::from(agent_id.as_str()));
            }
            None => {
                record.shift_remove("agentId");
            }
        }
        self.timestamp += 2.0;
        let stamp = format!("{}.000Z", utc_iso_seconds(self.timestamp as i64));
        record.insert("timestamp".into(), Value::from(stamp.as_str()));
        record.insert("userType".into(), Value::from("external"));
        if !record.contains_key("version") {
            record.insert("version".into(), Value::from("ferry"));
        }
        for key in ["toolUseResult", "sourceToolAssistantUUID", "promptSource"] {
            record.shift_remove(key);
        }
        self.parent = Some(uuid);
        let cwd = self.cwd.clone();
        ensure_resume_fields(&mut record, Some(&cwd), Some(&stamp));
        record
    }
}

fn remember(
    source_uuids: &mut HashMap<String, String>,
    message: Option<&Message>,
    record: &Map<String, Value>,
    replace: bool,
) {
    let Some(source_id) = message
        .and_then(|message| message.source_id.as_deref())
        .filter(|source_id| !source_id.is_empty())
    else {
        return;
    };
    let uuid = record["uuid"].as_str().unwrap_or_default().to_string();
    if replace {
        source_uuids.insert(source_id.to_string(), uuid);
    } else {
        source_uuids.entry(source_id.to_string()).or_insert(uuid);
    }
}

fn edge_for_tool<'a>(session: &'a Session, tool: &ToolCall) -> Option<&'a AgentEdge> {
    session.agent_edges.iter().find(|edge| {
        (tool
            .source_call_id
            .as_deref()
            .is_some_and(|id| !id.is_empty())
            && edge.source_call_id == tool.source_call_id)
            || (tool.agent_id.as_deref().is_some_and(|id| !id.is_empty())
                && edge.agent_id == tool.agent_id)
    })
}

#[allow(clippy::too_many_arguments)]
fn generated_lines(
    session: &Session,
    sid: &str,
    cwd: &str,
    templates: &Map<String, Value>,
    agent_map: &HashMap<String, String>,
    source_uuids: &mut HashMap<String, String>,
    fork_parent: Option<&str>,
    decider: Option<&ToolDecider>,
    losses: &mut Vec<Event>,
) -> DomainResult<Vec<Value>> {
    let agent_id = agent_map.get(&session.source_id).cloned();
    let mut generator = Generator {
        templates,
        sid,
        cwd: cwd.to_string(),
        agent_id: agent_id.clone(),
        records: Vec::new(),
        parent: None,
        timestamp: epoch_seconds_now() - session.messages.len() as f64 * 2.0,
        emitted_children: HashSet::new(),
    };

    if let Some(agent_id) = &agent_id {
        let mut header = Map::new();
        header.insert("type".into(), Value::from("fork-context-ref"));
        header.insert("agentId".into(), Value::from(agent_id.as_str()));
        header.insert("parentSessionId".into(), Value::from(sid));
        header.insert(
            "parentLastUuid".into(),
            fork_parent.map_or(Value::Null, Value::from),
        );
        header.insert("contextLength".into(), Value::from(0));
        generator.records.push(Value::Object(header));
    }

    for message in &session.messages {
        let mut texts: Vec<String> = Vec::new();
        for block in &message.blocks {
            match (block.kind, block.tool.as_ref()) {
                (BlockKind::Text, _) => texts.push(block.text.clone()),
                (BlockKind::Tool, Some(tool)) => {
                    let decision = match decider {
                        Some(decider) => Some(decider(tool, session, Some(message))?),
                        None => None,
                    };
                    let native = match &decision {
                        Some(decision) => decision.rendered.is_some(),
                        None => {
                            (DIALECT
                                .binding_for(tool.op.as_deref().unwrap_or(""))
                                .is_some()
                                && has_valid_tool_input(tool.op.as_deref(), &tool.input))
                                || ((tool.op.as_deref() == Some(CanonicalOp::AGENT_SPAWN)
                                    || tool.name == "Agent")
                                    && has_valid_tool_input(tool.op.as_deref(), &tool.input)
                                    && edge_for_tool(session, tool).is_some())
                        }
                    };
                    if native {
                        if !texts.is_empty() {
                            add_text(
                                &mut generator,
                                source_uuids,
                                Some(message),
                                &texts.join("\n\n"),
                            );
                            texts.clear();
                        }
                        add_tool(
                            &mut generator,
                            session,
                            agent_map,
                            source_uuids,
                            Some(message),
                            tool,
                            None,
                        )?;
                    } else {
                        let mut params = Map::new();
                        params.insert("tool_name".into(), Value::from(tool.name.as_str()));
                        if let Some(decision) = &decision {
                            params
                                .insert("fidelity".into(), Value::from(decision.fidelity.as_str()));
                            params.insert(
                                "reason_codes".into(),
                                Value::Array(
                                    decision
                                        .reason_codes
                                        .iter()
                                        .map(|code| Value::from(code.as_str()))
                                        .collect(),
                                ),
                            );
                            params.insert(
                                "ignored_fields".into(),
                                Value::Array(
                                    decision
                                        .ignored_fields
                                        .iter()
                                        .map(|field| Value::from(field.as_str()))
                                        .collect(),
                                ),
                            );
                        }
                        losses.push(Event::new("migration.tool_degraded", params));
                        texts.push(narrate(tool));
                    }
                }
                _ => {}
            }
        }
        if !texts.is_empty() {
            add_text(
                &mut generator,
                source_uuids,
                Some(message),
                &texts.join("\n\n"),
            );
        }
    }

    let children: HashMap<&str, &Session> = session
        .children
        .iter()
        .map(|child| (child.source_id.as_str(), child))
        .collect();
    let pending_edges: Vec<AgentEdge> = session
        .agent_edges
        .iter()
        .filter(|edge| {
            children.contains_key(edge.child_session_id.as_str())
                && !generator.emitted_children.contains(&edge.child_session_id)
        })
        .cloned()
        .collect();
    for edge in pending_edges {
        let child = children[edge.child_session_id.as_str()];
        let mut summary = String::new();
        for message in child.messages.iter().rev() {
            if message.role != "assistant" {
                continue;
            }
            summary = message
                .blocks
                .iter()
                .filter(|block| block.kind == BlockKind::Text && !block.text.is_empty())
                .map(|block| block.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            if !summary.is_empty() {
                break;
            }
        }
        let mut input = Map::new();
        input.insert(
            "description".into(),
            Value::from(if child.title.is_empty() {
                "migrated subagent"
            } else {
                child.title.as_str()
            }),
        );
        input.insert("prompt".into(), Value::from(edge.prompt.as_str()));
        input.insert(
            "subagent_type".into(),
            Value::from(
                edge.agent_type
                    .as_deref()
                    .filter(|value| !value.is_empty())
                    .or(child
                        .agent_type
                        .as_deref()
                        .filter(|value| !value.is_empty()))
                    .unwrap_or("general"),
            ),
        );
        let mut tool = ToolCall::new(
            "Agent",
            Some(CanonicalOp::AGENT_SPAWN.to_string()),
            Value::Object(input),
        );
        tool.result = Some(ToolResult {
            status: ToolResultStatus::Success,
            blocks: if summary.is_empty() {
                Vec::new()
            } else {
                vec![ToolResultBlock::text(summary.as_str())]
            },
            ..ToolResult::default()
        });
        add_tool(
            &mut generator,
            session,
            agent_map,
            source_uuids,
            None,
            &tool,
            Some(&edge),
        )?;
    }
    Ok(generator.records)
}

fn add_text(
    generator: &mut Generator,
    source_uuids: &mut HashMap<String, String>,
    message: Option<&Message>,
    text: &str,
) {
    let assistant = message.is_some_and(|message| message.role == "assistant");
    let kind = if assistant { "assistant" } else { "user" };
    let mut record = generator.base(kind);
    if assistant {
        let mut body = record
            .get("message")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let mut block = Map::new();
        block.insert("type".into(), Value::from("text"));
        block.insert("text".into(), Value::from(text));
        body.insert("content".into(), Value::Array(vec![Value::Object(block)]));
        record.insert("message".into(), Value::Object(body));
    } else {
        let mut body = Map::new();
        body.insert("role".into(), Value::from("user"));
        body.insert("content".into(), Value::from(text));
        record.insert("message".into(), Value::Object(body));
    }
    remember(source_uuids, message, &record, false);
    generator.records.push(Value::Object(record));
}

#[allow(clippy::too_many_arguments)]
fn add_tool(
    generator: &mut Generator,
    session: &Session,
    agent_map: &HashMap<String, String>,
    source_uuids: &mut HashMap<String, String>,
    message: Option<&Message>,
    tool: &ToolCall,
    edge_override: Option<&AgentEdge>,
) -> DomainResult<()> {
    let spawn_like = tool.op.as_deref() == Some(CanonicalOp::AGENT_SPAWN) || tool.name == "Agent";
    let edge = edge_override.or_else(|| {
        if spawn_like {
            edge_for_tool(session, tool)
        } else {
            None
        }
    });

    let (native_name, native_input) = if let Some(edge) = edge {
        generator
            .emitted_children
            .insert(edge.child_session_id.clone());
        ("Agent".to_string(), agent_input(&tool.input))
    } else if tool.op.as_deref() == Some(CanonicalOp::TOOL_INVOKE) {
        let entries = tool
            .input
            .as_object()
            .ok_or_else(|| DomainError::internal("tool.invoke 入参必须是对象"))?;
        (
            entries
                .get("name")
                .map(crate::adapters::shared::dialect::python_str)
                .unwrap_or_default(),
            entries.get("input").cloned().unwrap_or(Value::Null),
        )
    } else {
        let (name, native) = DIALECT
            .render(tool.op.as_deref().unwrap_or(""), &tool.input)
            .ok_or_else(|| {
                DomainError::internal(format!("claude 方言没有 {} 的原生形态", tool.name))
            })?;
        (name.to_string(), Value::Object(native))
    };

    let use_id = format!("toolu_{}", &uuid4_hex()[..24]);
    let mut assistant = generator.base("assistant");
    let mut body = assistant
        .get("message")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut call = Map::new();
    call.insert("type".into(), Value::from("tool_use"));
    call.insert("id".into(), Value::from(use_id.as_str()));
    call.insert("name".into(), Value::from(native_name.as_str()));
    call.insert("input".into(), native_input);
    body.insert("content".into(), Value::Array(vec![Value::Object(call)]));
    assistant.insert("message".into(), Value::Object(body));
    generator.records.push(Value::Object(assistant.clone()));
    if message.is_some() {
        // 子会话必须 fork 在真实 Agent 调用上，而不是同一 canonical 消息里
        // 先输出的文字记录上。
        remember(source_uuids, message, &assistant, edge.is_some());
    } else if let Some(spawn_message_id) = edge
        .and_then(|edge| edge.spawn_message_id.as_deref())
        .filter(|value| !value.is_empty())
    {
        source_uuids.insert(
            spawn_message_id.to_string(),
            assistant["uuid"].as_str().unwrap_or_default().to_string(),
        );
    }

    let (result_content, native_result) = claude_result(tool);
    let mut user = generator.base("user");
    let mut result_block = Map::new();
    result_block.insert("type".into(), Value::from("tool_result"));
    result_block.insert("tool_use_id".into(), Value::from(use_id.as_str()));
    result_block.insert("content".into(), result_content);
    if tool
        .result
        .as_ref()
        .is_some_and(|result| result.status == ToolResultStatus::Error)
    {
        result_block.insert("is_error".into(), Value::Bool(true));
    }
    let mut body = Map::new();
    body.insert("role".into(), Value::from("user"));
    body.insert(
        "content".into(),
        Value::Array(vec![Value::Object(result_block)]),
    );
    user.insert("message".into(), Value::Object(body));
    if native_name == "Bash" {
        user.insert("toolUseResult".into(), native_result);
    } else if let Some(edge) = edge {
        let old_agent = edge.child_session_id.as_str();
        let mut payload = Map::new();
        payload.insert(
            "agentId".into(),
            Value::from(agent_map.get(old_agent).map_or(old_agent, String::as_str)),
        );
        payload.insert(
            "status".into(),
            Value::from(
                edge.status
                    .as_deref()
                    .filter(|status| !status.is_empty())
                    .unwrap_or("completed"),
            ),
        );
        user.insert("toolUseResult".into(), Value::Object(payload));
    } else if tool.result.is_some() {
        user.insert("toolUseResult".into(), native_result);
    }
    generator.records.push(Value::Object(user));
    Ok(())
}

/// subagent 的落盘路径；`workflows/<name>` 结构原样保留。
fn child_path(destination: &Path, sid: &str, child: &Session, new_agent: &str) -> PathBuf {
    let source = PathBuf::from(child.agent_path.clone().unwrap_or_default());
    let parts: Vec<String> = source
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect();
    let base = destination.join(sid).join("subagents");
    if let Some(index) = parts.iter().position(|part| part == "workflows") {
        let mut suffix = base;
        for part in parts.iter().skip(index).take(2) {
            suffix = suffix.join(part);
        }
        return suffix.join(format!("agent-{new_agent}.jsonl"));
    }
    base.join(format!("agent-{new_agent}.jsonl"))
}

/// 写出会话树，返回根会话的新 ID 与主 JSONL 路径。
pub fn write(
    session: &Session,
    cwd: Option<&str>,
    dest_root: Option<&Path>,
    decider: Option<&ToolDecider>,
) -> DomainResult<(String, PathBuf)> {
    let templates = templates();
    let sid = uuid4();
    let cwd = cwd
        .filter(|cwd| !cwd.is_empty())
        .unwrap_or(session.cwd.as_str())
        .to_string();
    let destination = match dest_root {
        Some(root) => root.to_path_buf(),
        None => home_dir().join(".claude").join("projects").join(slug(&cwd)),
    };
    let main_path = destination.join(format!("{sid}.jsonl"));
    let agent_map = agent_ids(session);

    let mut created: Vec<PathBuf> = Vec::new();
    let outcome = (|| -> DomainResult<()> {
        let mut source_uuids: HashMap<String, String> = HashMap::new();
        let mut losses: Vec<Event> = Vec::new();
        let root_records = generated_lines(
            session,
            &sid,
            &cwd,
            &templates,
            &agent_map,
            &mut source_uuids,
            None,
            decider,
            &mut losses,
        )?;
        write_jsonl(&main_path, &root_records)
            .map_err(|error| DomainError::internal(format!("写入 claude 会话失败: {error}")))?;
        created.push(main_path.clone());

        let mut edges: HashMap<&str, &AgentEdge> = HashMap::new();
        for node in session.walk() {
            for edge in &node.agent_edges {
                edges.insert(edge.child_session_id.as_str(), edge);
            }
        }
        for child in session.walk().into_iter().skip(1) {
            let edge = edges.get(child.source_id.as_str()).copied();
            let old_parent_uuid = child
                .forked_from_id
                .as_deref()
                .filter(|value| !value.is_empty())
                .or_else(|| edge.and_then(|edge| edge.spawn_message_id.as_deref()));
            let mut fork_parent = old_parent_uuid
                .and_then(|key| source_uuids.get(key))
                .cloned();
            if fork_parent.is_none() {
                if let Some(last) = root_records.last() {
                    fork_parent = last
                        .get("uuid")
                        .and_then(Value::as_str)
                        .map(std::string::ToString::to_string);
                    losses.push(Event::new("migration.fork_parent_fallback", Map::new()));
                }
            }
            let child_cwd = if child.cwd.is_empty() {
                cwd.clone()
            } else {
                child.cwd.clone()
            };
            let records = generated_lines(
                child,
                &sid,
                &child_cwd,
                &templates,
                &agent_map,
                &mut source_uuids,
                fork_parent.as_deref(),
                decider,
                &mut losses,
            )?;
            let new_agent = agent_map
                .get(&child.source_id)
                .cloned()
                .unwrap_or_else(|| "None".to_string());
            let path = child_path(&destination, &sid, child, &new_agent);
            write_jsonl(&path, &records).map_err(|error| {
                DomainError::internal(format!("写入 claude 子会话失败: {error}"))
            })?;
            created.push(path);
        }
        Ok(())
    })();

    if let Err(error) = outcome {
        for path in &created {
            let _ = std::fs::remove_file(path);
        }
        return Err(error);
    }
    Ok((sid, main_path))
}

/// `MigrationTarget::write` 要求的返回形状：`session_id` + `dest`。
pub fn write_result(
    session: &Session,
    cwd: &str,
    decider: Option<&ToolDecider>,
) -> DomainResult<Map<String, Value>> {
    let (sid, path) = write(session, Some(cwd), None, decider)?;
    let mut result = Map::new();
    result.insert("session_id".into(), Value::from(sid));
    result.insert(
        "dest".into(),
        Value::from(path.to_string_lossy().into_owned()),
    );
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::shared::dialect::register_dialect;
    use crate::model::{text_tool_result, Block};
    use serde_json::json;

    fn setup() {
        register_dialect("claude", &DIALECT);
    }

    fn read_lines(path: &Path) -> Vec<Value> {
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn tool_message(tool: ToolCall) -> Message {
        let mut message = Message::new("assistant");
        message.blocks.push(Block {
            tool: Some(tool),
            ..Block::new(BlockKind::Tool)
        });
        message
    }

    #[test]
    fn text_only_sessions_become_user_and_assistant_records() {
        setup();
        let root = tempfile::tempdir().unwrap();
        let mut session = Session::new("codex", "src", "/work");
        let mut user = Message::new("user");
        user.source_id = Some("m1".into());
        user.blocks.push(Block::text("hello"));
        let mut assistant = Message::new("assistant");
        assistant.blocks.push(Block::text("hi"));
        session.messages.push(user);
        session.messages.push(assistant);

        let (sid, path) = write(&session, Some("/work"), Some(root.path()), None).unwrap();
        assert_eq!(path, root.path().join(format!("{sid}.jsonl")));
        let records = read_lines(&path);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["type"], json!("user"));
        assert_eq!(records[0]["message"]["content"], json!("hello"));
        assert_eq!(records[0]["sessionId"], json!(sid));
        assert_eq!(records[0]["cwd"], json!("/work"));
        assert_eq!(records[0]["userType"], json!("external"));
        assert_eq!(records[0]["isSidechain"], json!(false));
        assert!(records[0].get("agentId").is_none());
        assert_eq!(records[1]["type"], json!("assistant"));
        assert_eq!(
            records[1]["message"]["content"],
            json!([{"type": "text", "text": "hi"}])
        );
        assert_eq!(records[1]["parentUuid"], records[0]["uuid"]);
    }

    #[test]
    fn native_tools_render_through_the_dialect_with_paired_results() {
        setup();
        let root = tempfile::tempdir().unwrap();
        let mut session = Session::new("codex", "src", "/work");
        let mut tool = ToolCall::new(
            "shell",
            Some(CanonicalOp::SHELL_EXEC.to_string()),
            json!({"command": "ls", "workdir": "/w"}),
        );
        tool.result = Some(text_tool_result("out", ToolResultStatus::Success));
        session.messages.push(tool_message(tool));

        let (_, path) = write(&session, Some("/work"), Some(root.path()), None).unwrap();
        let records = read_lines(&path);
        assert_eq!(records.len(), 2);
        let call = &records[0]["message"]["content"][0];
        assert_eq!(call["name"], json!("Bash"));
        assert_eq!(call["input"]["command"], json!("cd /w && ls"));
        let result = &records[1]["message"]["content"][0];
        assert_eq!(result["tool_use_id"], call["id"]);
        assert_eq!(result["content"], json!([{"type": "text", "text": "out"}]));
        // Bash 一律带 toolUseResult。
        assert_eq!(records[1]["toolUseResult"]["status"], json!("success"));
        assert_eq!(records[1]["toolUseResult"]["interrupted"], json!(false));
    }

    #[test]
    fn unrenderable_tools_degrade_into_narration() {
        setup();
        let root = tempfile::tempdir().unwrap();
        let mut session = Session::new("codex", "src", "/work");
        let mut tool = ToolCall::new(
            "apply_patch",
            Some(CanonicalOp::FS_PATCH.to_string()),
            json!({"patch": "*** Begin Patch"}),
        );
        tool.result = Some(text_tool_result("done", ToolResultStatus::Success));
        session.messages.push(tool_message(tool));

        let (_, path) = write(&session, Some("/work"), Some(root.path()), None).unwrap();
        let records = read_lines(&path);
        assert_eq!(records.len(), 1);
        let text = records[0]["message"]["content"][0]["text"]
            .as_str()
            .unwrap();
        assert!(text.starts_with("[History: tool apply_patch was previously invoked]"));
    }

    #[test]
    fn error_results_are_flagged_and_images_are_projected() {
        setup();
        let root = tempfile::tempdir().unwrap();
        let mut session = Session::new("codex", "src", "/work");
        let mut tool = ToolCall::new(
            "Read",
            Some(CanonicalOp::FS_READ.to_string()),
            json!({"file_path": "/a"}),
        );
        tool.result = Some(ToolResult {
            status: ToolResultStatus::Error,
            blocks: vec![
                ToolResultBlock {
                    data: json!("QQ=="),
                    mime_type: Some("image/png".into()),
                    ..ToolResultBlock::new(ToolResultBlockKind::Image)
                },
                ToolResultBlock {
                    data: json!({"x": 1}),
                    ..ToolResultBlock::new(ToolResultBlockKind::Json)
                },
            ],
            ..ToolResult::default()
        });
        session.messages.push(tool_message(tool));

        let (_, path) = write(&session, Some("/work"), Some(root.path()), None).unwrap();
        let records = read_lines(&path);
        let result = &records[1]["message"]["content"][0];
        assert_eq!(result["is_error"], json!(true));
        assert_eq!(result["content"][0]["type"], json!("image"));
        assert_eq!(
            result["content"][0]["source"]["media_type"],
            json!("image/png")
        );
        // json 块投影成文本。
        assert_eq!(result["content"][1]["type"], json!("text"));
        assert!(result["content"][1]["text"]
            .as_str()
            .unwrap()
            .contains("\"kind\": \"json\""));
        assert_eq!(records[1]["toolUseResult"]["isImage"], json!(true));
        assert_eq!(records[1]["toolUseResult"]["status"], json!("error"));
    }

    #[test]
    fn child_sessions_get_their_own_sidechain_file() {
        setup();
        let root = tempfile::tempdir().unwrap();
        let mut session = Session::new("codex", "src", "/work");
        let mut spawn = Message::new("assistant");
        spawn.source_id = Some("m-spawn".into());
        spawn.blocks.push(Block::text("delegating"));
        session.messages.push(spawn);

        let mut child = Session::new("codex", "child", "/work");
        child.agent_path = Some("src/subagents/agent-old.jsonl".into());
        let mut reply = Message::new("assistant");
        reply.blocks.push(Block::text("child result"));
        child.messages.push(reply);
        session.children.push(child);

        let mut edge = AgentEdge::new("src", "child");
        edge.spawn_message_id = Some("m-spawn".into());
        edge.prompt = "do it".into();
        edge.agent_type = Some("explorer".into());
        session.agent_edges.push(edge);

        let (sid, path) = write(&session, Some("/work"), Some(root.path()), None).unwrap();
        let records = read_lines(&path);
        // 文本记录 + Agent 调用/结果对。
        assert_eq!(records.len(), 3);
        let call = &records[1]["message"]["content"][0];
        assert_eq!(call["name"], json!("Agent"));
        assert_eq!(call["input"]["prompt"], json!("do it"));
        assert_eq!(call["input"]["subagent_type"], json!("explorer"));
        let new_agent = records[2]["toolUseResult"]["agentId"].as_str().unwrap();
        assert_eq!(records[2]["toolUseResult"]["status"], json!("completed"));

        let child_file = root
            .path()
            .join(sid)
            .join("subagents")
            .join(format!("agent-{new_agent}.jsonl"));
        let child_records = read_lines(&child_file);
        assert_eq!(child_records[0]["type"], json!("fork-context-ref"));
        assert_eq!(child_records[0]["agentId"], json!(new_agent));
        assert_eq!(child_records[0]["parentLastUuid"], records[1]["uuid"]);
        assert_eq!(child_records[1]["isSidechain"], json!(true));
        assert_eq!(child_records[1]["agentId"], json!(new_agent));
    }

    #[test]
    fn slug_folds_every_non_alphanumeric_character() {
        assert_eq!(slug("/tmp"), slug("/tmp"));
        assert!(!slug("/a b/c").contains(' '));
        assert!(slug("/a b/c")
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-'));
    }

    #[test]
    fn workflow_children_keep_two_path_segments() {
        let mut child = Session::new("claude", "c", "/w");
        child.agent_path = Some("sess/subagents/workflows/build/agent-x.jsonl".into());
        let path = child_path(Path::new("/dest"), "sid", &child, "a1");
        assert_eq!(
            path,
            PathBuf::from("/dest/sid/subagents/workflows/build/agent-a1.jsonl")
        );
        child.agent_path = Some("sess/subagents/agent-x.jsonl".into());
        assert_eq!(
            child_path(Path::new("/dest"), "sid", &child, "a1"),
            PathBuf::from("/dest/sid/subagents/agent-a1.jsonl")
        );
    }
}
