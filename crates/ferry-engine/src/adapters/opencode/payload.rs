//! Canonical Session 到 OpenCode 当前 payload 的编译与重映射。
//!
//! 语义事实源：`engine/adapters/opencode/payload.py`。
//!
//! 三个不变量：
//! 1. 同一父记录内的 part id 必须能按**字典序**恢复原顺序（`new_ordered_id`），
//!    否则 opencode 会按随机 id 重排 part；
//! 2. 消息时间戳必须**严格递增**（毫秒），否则并列时间同样退化成按 id 排序；
//! 3. `opencode import` 会严格校验完整的 `Session.Info`，模板只保留结构字段，
//!    必填默认值在 [`canonical_payload`] 里补齐。

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use rand::RngCore as _;
use serde_json::{Map, Value};

use crate::adapters::shared::migration::RenderDecision;
use crate::adapters::shared::narration::narrate;
use crate::adapters::shared::scanner::iso_ms;
use crate::errors::{DomainError, DomainResult};
use crate::events::Event;
use crate::model::{AgentEdge, Message, Session, Timestamp, ToolCall};
use crate::tool_ops::{has_valid_tool_input, CanonicalOp};

use super::tool_calls;

/// `tool_decider`：plan / preview / write 三路共用的调用级判定。
pub type ToolDecider<'a> =
    &'a dyn Fn(&ToolCall, &Session, &Message) -> DomainResult<RenderDecision>;

/// 目标端模型标注的兜底值（Python 的字面量）。
const DEFAULT_PROVIDER: &str = "openai";
const DEFAULT_MODEL: &str = "gpt-5.6-sol";

fn random_bytes(count: usize) -> Vec<u8> {
    let mut buffer = vec![0u8; count];
    rand::rng().fill_bytes(&mut buffer);
    buffer
}

fn token_hex(bytes: usize) -> String {
    random_bytes(bytes)
        .into_iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn token_urlsafe(bytes: usize) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random_bytes(bytes))
}

/// `f"{prefix}_{token_hex(6)}{token_urlsafe(12)[:14]}"`。
pub fn new_id(prefix: &str) -> String {
    let suffix: String = token_urlsafe(12).chars().take(14).collect();
    format!("{prefix}_{}{suffix}", token_hex(6))
}

/// 生成同一父记录内可按字典序恢复原顺序的 ID。
pub fn new_ordered_id(prefix: &str, ordinal: usize) -> String {
    format!("{prefix}_{ordinal:08x}{}", token_hex(10))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or_default()
}

fn object(value: Option<&Value>) -> Map<String, Value> {
    value
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_else(Map::new)
}

fn timestamp_value(created_at: Option<&Timestamp>) -> Value {
    match created_at {
        Some(Timestamp::Millis(value)) => Value::from(*value),
        Some(Timestamp::Text(text)) => Value::from(text.as_str()),
        None => Value::Null,
    }
}

/// 保留源会话顺序，并为 OpenCode 生成严格递增的毫秒时间戳。
fn message_times(messages: &[Message], now: i64) -> Vec<i64> {
    let parsed: Vec<Option<i64>> = messages
        .iter()
        .map(|message| iso_ms(&timestamp_value(message.created_at.as_ref())))
        .collect();
    let fallback = parsed.iter().flatten().copied().min().unwrap_or(now) - messages.len() as i64;
    strictly_increasing(&parsed, fallback)
}

/// 把可空时间序列压成严格递增序列；缺失值顺延，重复值抬高一毫秒。
fn strictly_increasing(parsed: &[Option<i64>], fallback: i64) -> Vec<i64> {
    let mut ordered = Vec::with_capacity(parsed.len());
    let mut previous: Option<i64> = None;
    for value in parsed {
        let candidate = match value {
            Some(value) => *value,
            None => previous.map_or(fallback, |previous| previous + 1),
        };
        let current = match previous {
            None => candidate,
            Some(previous) => candidate.max(previous + 1),
        };
        ordered.push(current);
        previous = Some(current);
    }
    ordered
}

/// 按 export 数组顺序消除时间戳并列，避免随机 ID 成为排序依据。
fn normalize_payload_message_times(payload: &mut Value) {
    let messages_len = payload
        .get("messages")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    // 只有 info 与 time 都是 dict 时才算源时间（对齐 Python 的两层 isinstance）。
    let source_times: Vec<Map<String, Value>> = payload
        .get("messages")
        .and_then(Value::as_array)
        .map(|messages| {
            messages
                .iter()
                .map(|message| {
                    let info = object(message.get("info"));
                    object(info.get("time"))
                })
                .collect()
        })
        .unwrap_or_default();
    let parsed: Vec<Option<i64>> = source_times
        .iter()
        .map(|time| iso_ms(time.get("created").unwrap_or(&Value::Null)))
        .collect();
    let fallback = parsed
        .iter()
        .flatten()
        .copied()
        .min()
        .unwrap_or_else(now_ms)
        - messages_len as i64;
    let created_times = strictly_increasing(&parsed, fallback);

    let mut previous_completed: Option<i64> = None;
    if let Some(messages) = payload.get_mut("messages").and_then(Value::as_array_mut) {
        for (index, message) in messages.iter_mut().enumerate() {
            let source_time = &source_times[index];
            let created = created_times[index];
            let mut normalized = source_time.clone();
            normalized.insert("created".into(), Value::from(created));
            if source_time.contains_key("completed") {
                let original_completed =
                    iso_ms(source_time.get("completed").unwrap_or(&Value::Null));
                let duration = match (original_completed, parsed[index]) {
                    (Some(completed), Some(original_created)) => {
                        (completed - original_created).max(0)
                    }
                    _ => 0,
                };
                let mut completed = created + duration;
                if let Some(previous) = previous_completed {
                    completed = completed.max(previous + 1);
                }
                normalized.insert("completed".into(), Value::from(completed));
                previous_completed = Some(completed);
            }
            let entries = match message.as_object_mut() {
                Some(entries) => entries,
                None => continue,
            };
            if !entries.get("info").is_some_and(Value::is_object) {
                entries.insert("info".into(), Value::Object(Map::new()));
            }
            entries["info"]
                .as_object_mut()
                .expect("上一行已保证 info 是对象")
                .insert("time".into(), Value::Object(normalized));
        }
    }

    let payload_entries = match payload.as_object_mut() {
        Some(entries) => entries,
        None => return,
    };
    if !payload_entries.get("info").is_some_and(Value::is_object) {
        payload_entries.insert("info".into(), Value::Object(Map::new()));
    }
    let info = payload_entries["info"]
        .as_object_mut()
        .expect("上一行已保证 info 是对象");
    let mut session_time = object(info.get("time"));
    if messages_len > 0 {
        let first = created_times[0];
        let last = created_times[created_times.len() - 1];
        let source_created = iso_ms(session_time.get("created").unwrap_or(&Value::Null));
        let source_updated = iso_ms(session_time.get("updated").unwrap_or(&Value::Null));
        session_time.insert(
            "created".into(),
            Value::from(source_created.unwrap_or(first).min(first)),
        );
        session_time.insert(
            "updated".into(),
            Value::from(source_updated.unwrap_or(last).max(last)),
        );
    } else {
        let now = now_ms();
        let created = iso_ms(session_time.get("created").unwrap_or(&Value::Null)).unwrap_or(now);
        let updated = iso_ms(session_time.get("updated").unwrap_or(&Value::Null))
            .unwrap_or(created)
            .max(created);
        session_time.insert("created".into(), Value::from(created));
        session_time.insert("updated".into(), Value::from(updated));
    }
    info.insert("time".into(), Value::Object(session_time));
}

/// 子会话的最终产出：最后一条 assistant 消息的可见正文。
fn assistant_result(session: &Session) -> String {
    for message in session.messages.iter().rev() {
        if message.role != "assistant" {
            continue;
        }
        let text = message
            .blocks
            .iter()
            .filter(|block| block.kind == crate::model::BlockKind::Text && !block.text.is_empty())
            .map(|block| block.text.clone())
            .collect::<Vec<_>>()
            .join("\n");
        if !text.is_empty() {
            return text;
        }
    }
    String::new()
}

/// 子 Agent 派生的原生 task part。
///
/// `state.metadata` 的 `parentSessionId` / `sessionId` 是 opencode 恢复会话树的
/// 唯一线索，两个键都必须写。
#[allow(clippy::too_many_arguments)]
fn task_part(
    templates: &Map<String, Value>,
    sid: &str,
    mid: &str,
    ordinal: usize,
    child: &Session,
    child_sid: &str,
    edge: Option<&AgentEdge>,
    when: i64,
    source_call_id: Option<&str>,
) -> Value {
    let mut part = object(templates.get("part.tool"));
    let prompt = edge.map(|edge| edge.prompt.clone()).unwrap_or_default();
    let call_id = edge
        .and_then(|edge| edge.source_call_id.clone())
        .filter(|value| !value.is_empty())
        .or_else(|| source_call_id.map(str::to_string))
        .unwrap_or_else(|| format!("call-{}", token_hex(8)));
    let title = if child.title.is_empty() {
        "Subagent".to_string()
    } else {
        child.title.clone()
    };
    let description = if child.title.is_empty() {
        "migrated subagent".to_string()
    } else {
        child.title.clone()
    };
    let subagent_type = edge
        .and_then(|edge| edge.agent_type.clone())
        .filter(|value| !value.is_empty())
        .or_else(|| child.agent_type.clone().filter(|value| !value.is_empty()))
        .unwrap_or_else(|| "general".to_string());

    let mut input = Map::new();
    input.insert("description".into(), Value::from(description));
    input.insert("prompt".into(), Value::from(prompt));
    input.insert("subagent_type".into(), Value::from(subagent_type));
    let mut metadata = Map::new();
    metadata.insert("parentSessionId".into(), Value::from(sid));
    metadata.insert("sessionId".into(), Value::from(child_sid));
    let mut time = Map::new();
    time.insert("start".into(), Value::from(when));
    time.insert("end".into(), Value::from(when));
    let mut state = Map::new();
    state.insert("status".into(), Value::from("completed"));
    state.insert("input".into(), Value::Object(input));
    state.insert("output".into(), Value::from(assistant_result(child)));
    state.insert("title".into(), Value::from(title));
    state.insert("metadata".into(), Value::Object(metadata));
    state.insert("time".into(), Value::Object(time));

    part.insert("id".into(), Value::from(new_ordered_id("prt", ordinal)));
    part.insert("messageID".into(), Value::from(mid));
    part.insert("sessionID".into(), Value::from(sid));
    part.insert("type".into(), Value::from("tool"));
    part.insert("tool".into(), Value::from("task"));
    part.insert("callID".into(), Value::from(call_id));
    part.insert("state".into(), Value::Object(state));
    Value::Object(part)
}

/// 一条消息的 part 装配器（对齐 Python 内嵌的 `add_part` / `add_tool_part`）。
struct PartBuilder<'a> {
    templates: &'a Map<String, Value>,
    parts: Vec<Value>,
    mid: String,
    sid: String,
    message_time: i64,
}

impl PartBuilder<'_> {
    fn add_part(&mut self, kind: &str, fill: Map<String, Value>) -> bool {
        let key = format!("part.{kind}");
        let Some(template) = self.templates.get(&key).and_then(Value::as_object) else {
            return false;
        };
        let mut part = template.clone();
        part.insert(
            "id".into(),
            Value::from(new_ordered_id("prt", self.parts.len())),
        );
        part.insert("messageID".into(), Value::from(self.mid.as_str()));
        part.insert("sessionID".into(), Value::from(self.sid.as_str()));
        for (key, value) in fill {
            part.insert(key, value);
        }
        self.parts.push(Value::Object(part));
        true
    }

    fn add_tool_part(
        &mut self,
        tool: &str,
        native_input: Value,
        output: &str,
        title: &str,
        metadata: Map<String, Value>,
        canonical_tool: &ToolCall,
    ) -> bool {
        let result = canonical_tool.result.as_ref();
        let state_status = match result.map(|result| result.status) {
            Some(crate::model::ToolResultStatus::Success) => "completed",
            Some(crate::model::ToolResultStatus::Error) => "error",
            Some(crate::model::ToolResultStatus::Running) => "running",
            Some(crate::model::ToolResultStatus::Pending) => "pending",
            _ => "completed",
        };
        let mut native_metadata = metadata;
        if let Some(result) = result {
            if let Some(exit_code) = result.exit_code {
                native_metadata.insert("exit".into(), Value::from(exit_code));
            }
            if let Some(truncated) = result.truncated {
                native_metadata.insert("truncated".into(), Value::Bool(truncated));
            }
            if let Some(stdout) = &result.stdout {
                native_metadata.insert("stdout".into(), Value::from(stdout.as_str()));
            }
            if let Some(stderr) = &result.stderr {
                native_metadata.insert("stderr".into(), Value::from(stderr.as_str()));
            }
        }

        // Python 先 clone 模板 state 再 `clear()`：模板里的字段一个都不保留。
        let mut state = Map::new();
        state.insert("status".into(), Value::from(state_status));
        state.insert("input".into(), native_input);
        if state_status == "pending" {
            state.insert("raw".into(), Value::from(""));
        } else {
            state.insert(
                "title".into(),
                Value::from(title.chars().take(80).collect::<String>()),
            );
            state.insert("metadata".into(), Value::Object(native_metadata));
            let mut time = Map::new();
            time.insert("start".into(), Value::from(self.message_time));
            if state_status == "completed" || state_status == "error" {
                time.insert("end".into(), Value::from(self.message_time));
            }
            state.insert("time".into(), Value::Object(time));
            if state_status == "error" {
                let text = result
                    .and_then(|result| result.stderr.clone())
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| {
                        if output.is_empty() {
                            "Tool failed".to_string()
                        } else {
                            output.to_string()
                        }
                    });
                state.insert("error".into(), Value::from(text));
            } else {
                state.insert("output".into(), Value::from(output));
            }
        }
        if let Some(result) = result {
            if !result.attachments.is_empty() {
                state.insert(
                    "attachments".into(),
                    Value::Array(result.attachments.clone()),
                );
            }
        }

        let mut fill = Map::new();
        fill.insert("tool".into(), Value::from(tool));
        fill.insert(
            "callID".into(),
            Value::from(format!("call-{}", token_hex(8))),
        );
        fill.insert("state".into(), Value::Object(state));
        self.add_part("tool", fill)
    }
}

fn degradation_loss(tool: &ToolCall, decision: Option<&RenderDecision>) -> Event {
    let mut params = Map::new();
    params.insert("tool_name".into(), Value::from(tool.name.as_str()));
    if let Some(decision) = decision {
        params.insert("fidelity".into(), Value::from(decision.fidelity.as_str()));
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
    Event::new("migration.tool_degraded", params)
}

/// Canonical Session → OpenCode import payload。
///
/// 返回值的第二项是本次编译产生的损耗记录（Python 直接写进 `sess.loss`；Rust 的
/// `MigrationTarget::write` 只拿 `&Session`，由调用方决定怎么处理）。
#[allow(clippy::too_many_arguments)]
pub fn canonical_payload(
    session: &Session,
    sid: &str,
    cwd: &str,
    parent_sid: Option<&str>,
    templates: &Map<String, Value>,
    sid_map: &BTreeMap<String, String>,
    tool_decider: Option<ToolDecider<'_>>,
) -> DomainResult<(Value, Vec<Event>)> {
    let now = now_ms();
    let times = message_times(&session.messages, now);
    let mut session_created = times.first().copied().unwrap_or(now);
    if session
        .messages
        .first()
        .is_some_and(|message| message.role == "assistant")
    {
        session_created -= 1;
    }
    let session_updated = times.last().copied().unwrap_or(session_created);

    let mut info = object(templates.get("info"));
    info.insert("id".into(), Value::from(sid));
    info.insert("directory".into(), Value::from(cwd));
    info.insert(
        "title".into(),
        Value::from(if session.title.is_empty() {
            format!("migrated from {}", session.source_tool)
        } else {
            session.title.clone()
        }),
    );
    let mut session_time = Map::new();
    session_time.insert("created".into(), Value::from(session_created));
    session_time.insert("updated".into(), Value::from(session_updated));
    info.insert("time".into(), Value::Object(session_time));

    // `opencode import` 严格校验完整 Session.Info；模板只留结构字段，必填项补默认值。
    let slug_tail: String = {
        let characters: Vec<char> = sid.chars().collect();
        characters[characters.len().saturating_sub(8)..]
            .iter()
            .collect::<String>()
            .to_lowercase()
    };
    let defaults: Vec<(&str, Value)> = vec![
        ("slug", Value::from(format!("ferry-{slug_tail}"))),
        ("projectID", Value::from("global")),
        ("path", Value::from("")),
        ("agent", Value::from("build")),
        (
            "summary",
            serde_json::json!({"additions": 0, "deletions": 0, "files": 0}),
        ),
        ("cost", Value::from(0)),
        (
            "tokens",
            serde_json::json!({"input": 0, "output": 0, "reasoning": 0,
                               "cache": {"read": 0, "write": 0}}),
        ),
    ];
    for (key, value) in defaults {
        info.entry(key.to_string()).or_insert(value);
    }
    match parent_sid {
        Some(parent) => {
            info.insert("parentID".into(), Value::from(parent));
        }
        None => {
            info.remove("parentID");
        }
    }
    info.remove("share");

    let mut messages: Vec<Value> = Vec::new();
    let mut loss: Vec<Event> = Vec::new();
    let mut last_user_mid: Option<String> = None;
    let children: BTreeMap<&str, &Session> = session
        .children
        .iter()
        .map(|child| (child.source_id.as_str(), child))
        .collect();
    // dict comprehension 的后来者覆盖语义：同一 child 的最后一条边胜出。
    let mut edges_by_child: Vec<(String, usize)> = Vec::new();
    for (index, edge) in session.agent_edges.iter().enumerate() {
        match edges_by_child
            .iter_mut()
            .find(|(child, _)| *child == edge.child_session_id)
        {
            Some(slot) => slot.1 = index,
            None => edges_by_child.push((edge.child_session_id.clone(), index)),
        }
    }
    let mut linked_children: Vec<String> = Vec::new();
    let mut emitted_edges: Vec<usize> = Vec::new();
    let provider_id = session
        .model_provider
        .clone()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_PROVIDER.to_string());
    let model_id = session
        .model
        .clone()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_MODEL.to_string());

    for (message, message_time) in session.messages.iter().zip(times.iter().copied()) {
        let mid = new_id("msg");
        let mut message_info = object(
            templates
                .get(&format!("msg.{}", message.role))
                .or_else(|| templates.get("msg.user")),
        );
        message_info.insert("id".into(), Value::from(mid.as_str()));
        message_info.insert("sessionID".into(), Value::from(sid));
        if message.role == "assistant" {
            if last_user_mid.is_none() {
                // 孤立的 assistant 消息：合成一条父 user 消息，否则 opencode 拒收。
                let synthetic = new_id("msg");
                let mut parent_info = object(templates.get("msg.user"));
                parent_info.insert("id".into(), Value::from(synthetic.as_str()));
                parent_info.insert("sessionID".into(), Value::from(sid));
                parent_info.insert("role".into(), Value::from("user"));
                parent_info.insert(
                    "time".into(),
                    serde_json::json!({"created": message_time - 1}),
                );
                parent_info.insert("agent".into(), Value::from("build"));
                parent_info.insert(
                    "model".into(),
                    serde_json::json!({"providerID": provider_id, "modelID": model_id}),
                );
                parent_info.insert("summary".into(), serde_json::json!({"diffs": []}));
                let mut parent_part = object(templates.get("part.text"));
                parent_part.insert("id".into(), Value::from(new_ordered_id("prt", 0)));
                parent_part.insert("messageID".into(), Value::from(synthetic.as_str()));
                parent_part.insert("sessionID".into(), Value::from(sid));
                parent_part.insert("type".into(), Value::from("text"));
                parent_part.insert("text".into(), Value::from("[Migrated subagent task]"));
                messages.push(serde_json::json!({
                    "info": Value::Object(parent_info),
                    "parts": [Value::Object(parent_part)],
                }));
                last_user_mid = Some(synthetic);
            }
            // completed + finish 缺失会让 runtime 认为该轮未结束而死循环。
            message_info.insert(
                "time".into(),
                serde_json::json!({"created": message_time, "completed": message_time}),
            );
            message_info.insert("finish".into(), Value::from("stop"));
            message_info.insert("mode".into(), Value::from("build"));
            message_info.insert("agent".into(), Value::from("build"));
            message_info.insert("path".into(), serde_json::json!({"cwd": cwd, "root": cwd}));
            message_info.insert("cost".into(), Value::from(0));
            message_info.insert(
                "tokens".into(),
                serde_json::json!({"total": 0, "input": 0, "output": 0, "reasoning": 0,
                                   "cache": {"write": 0, "read": 0}}),
            );
            message_info.insert("modelID".into(), Value::from(model_id.as_str()));
            message_info.insert("providerID".into(), Value::from(provider_id.as_str()));
            match &last_user_mid {
                Some(parent) => {
                    message_info.insert("parentID".into(), Value::from(parent.as_str()));
                }
                None => {
                    message_info.remove("parentID");
                }
            }
        } else {
            message_info.insert("time".into(), serde_json::json!({"created": message_time}));
            message_info.insert("agent".into(), Value::from("build"));
            message_info.insert(
                "model".into(),
                serde_json::json!({"providerID": provider_id, "modelID": model_id}),
            );
            message_info.insert("summary".into(), serde_json::json!({"diffs": []}));
            last_user_mid = Some(mid.clone());
        }

        let mut builder = PartBuilder {
            templates,
            parts: Vec::new(),
            mid: mid.clone(),
            sid: sid.to_string(),
            message_time,
        };

        for block in &message.blocks {
            match block.kind {
                crate::model::BlockKind::Text => {
                    let mut fill = Map::new();
                    fill.insert("text".into(), Value::from(block.text.as_str()));
                    builder.add_part("text", fill);
                }
                crate::model::BlockKind::Tool => {
                    let Some(tool) = block.tool.as_ref() else {
                        continue;
                    };
                    let decision = match tool_decider {
                        Some(decider) => Some(decider(tool, session, message)?),
                        None => None,
                    };
                    if tool.op.as_deref() == Some(CanonicalOp::AGENT_SPAWN) {
                        let edge_index = pick_edge(session, tool, message, &emitted_edges);
                        let edge = edge_index.map(|index| &session.agent_edges[index]);
                        let child = edge
                            .and_then(|edge| children.get(edge.child_session_id.as_str()).copied());
                        let child_sid =
                            edge.and_then(|edge| sid_map.get(&edge.child_session_id).cloned());
                        let renderable = decision
                            .as_ref()
                            .is_none_or(|decision| decision.rendered.is_some());
                        match (child, child_sid) {
                            (Some(child), Some(child_sid)) if renderable => {
                                let part = task_part(
                                    templates,
                                    sid,
                                    &mid,
                                    builder.parts.len(),
                                    child,
                                    &child_sid,
                                    edge,
                                    message_time,
                                    tool.source_call_id.as_deref(),
                                );
                                builder.parts.push(part);
                                if let Some(index) = edge_index {
                                    emitted_edges.push(index);
                                }
                                linked_children.push(child.source_id.clone());
                            }
                            _ => {
                                loss.push(degradation_loss(tool, decision.as_ref()));
                                let mut fill = Map::new();
                                fill.insert("text".into(), Value::from(narrate(tool)));
                                builder.add_part("text", fill);
                            }
                        }
                        continue;
                    }
                    let dropped = decision
                        .as_ref()
                        .is_some_and(|decision| decision.rendered.is_none());
                    let op = tool.op.as_deref().unwrap_or("");
                    let rendered = !dropped
                        && tool_calls::has_writer(op)
                        && has_valid_tool_input(tool.op.as_deref(), &tool.input)
                        && {
                            let mut add =
                                |name: &str,
                                 input: Value,
                                 output: &str,
                                 title: &str,
                                 metadata: Map<String, Value>,
                                 canonical: &ToolCall| {
                                    builder.add_tool_part(
                                        name, input, output, title, metadata, canonical,
                                    )
                                };
                            tool_calls::write_tool_part(op, &mut add, tool)
                        };
                    if !rendered {
                        loss.push(degradation_loss(tool, decision.as_ref()));
                        let mut fill = Map::new();
                        fill.insert("text".into(), Value::from(narrate(tool)));
                        builder.add_part("text", fill);
                    }
                }
                _ => {}
            }
        }

        // 没有在工具块上落地的子会话边：补在它声明的 spawn 消息上。
        for (child_id, edge_index) in &edges_by_child {
            let edge = &session.agent_edges[*edge_index];
            if linked_children.iter().any(|linked| linked == child_id)
                || edge.spawn_message_id != message.source_id
                || !children.contains_key(child_id.as_str())
                || !sid_map.contains_key(child_id)
            {
                continue;
            }
            let part = task_part(
                templates,
                sid,
                &mid,
                builder.parts.len(),
                children[child_id.as_str()],
                &sid_map[child_id],
                Some(edge),
                message_time,
                None,
            );
            builder.parts.push(part);
            linked_children.push(child_id.clone());
        }

        let parts = builder.parts;
        if !parts.is_empty() {
            if message.role == "assistant" {
                let has_tool = parts
                    .iter()
                    .any(|part| part.get("type") == Some(&Value::from("tool")));
                message_info.insert(
                    "finish".into(),
                    Value::from(if has_tool { "tool-calls" } else { "stop" }),
                );
            }
            messages.push(serde_json::json!({
                "info": Value::Object(message_info),
                "parts": Value::Array(parts),
            }));
        }
    }

    let mut payload = Map::new();
    payload.insert("info".into(), Value::Object(info));
    payload.insert("messages".into(), Value::Array(messages));
    Ok((Value::Object(payload), loss))
}

/// 三级匹配：source_call_id 精确命中 → 同一 spawn 消息上唯一的边。
fn pick_edge(
    session: &Session,
    tool: &ToolCall,
    message: &Message,
    emitted: &[usize],
) -> Option<usize> {
    let candidates: Vec<usize> = (0..session.agent_edges.len())
        .filter(|index| !emitted.contains(index))
        .collect();
    if let Some(call_id) = tool.source_call_id.as_deref().filter(|id| !id.is_empty()) {
        if let Some(index) = candidates
            .iter()
            .copied()
            .find(|index| session.agent_edges[*index].source_call_id.as_deref() == Some(call_id))
        {
            return Some(index);
        }
    }
    let at_message: Vec<usize> = candidates
        .into_iter()
        .filter(|index| session.agent_edges[*index].spawn_message_id == message.source_id)
        .collect();
    if at_message.len() == 1 {
        return Some(at_message[0]);
    }
    None
}

/// 原生 payload 的 ID / 会话归属重映射（迁移时把整棵树换成新签发的 id）。
pub fn remap_payload(
    payload: &Value,
    sid: &str,
    cwd: &str,
    parent_sid: Option<&str>,
    sid_map: &BTreeMap<String, String>,
) -> Value {
    let mut payload = payload.clone();
    {
        let info = payload
            .as_object_mut()
            .and_then(|entries| entries.get_mut("info"))
            .and_then(Value::as_object_mut);
        if let Some(info) = info {
            info.insert("id".into(), Value::from(sid));
            info.insert("directory".into(), Value::from(cwd));
            match parent_sid {
                Some(parent) => {
                    info.insert("parentID".into(), Value::from(parent));
                }
                None => {
                    info.remove("parentID");
                }
            }
        }
    }

    let mut message_ids: BTreeMap<String, String> = BTreeMap::new();
    if let Some(messages) = payload.get("messages").and_then(Value::as_array) {
        for message in messages {
            if let Some(old_id) = object(message.get("info"))
                .get("id")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
            {
                message_ids.insert(old_id.to_string(), new_id("msg"));
            }
        }
    }

    let mut last_user_mid: Option<String> = None;
    if let Some(messages) = payload.get_mut("messages").and_then(Value::as_array_mut) {
        for message in messages.iter_mut() {
            let (mid, role) = {
                let Some(info) = message.get_mut("info").and_then(Value::as_object_mut) else {
                    continue;
                };
                let old_id = info.get("id").and_then(Value::as_str).unwrap_or("");
                let mid = message_ids
                    .get(old_id)
                    .cloned()
                    .unwrap_or_else(|| new_id("msg"));
                info.insert("id".into(), Value::from(mid.as_str()));
                info.insert("sessionID".into(), Value::from(sid));
                let role = info
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if role == "assistant" {
                    let parent = info
                        .get("parentID")
                        .and_then(Value::as_str)
                        .and_then(|old| message_ids.get(old).cloned())
                        .or_else(|| last_user_mid.clone());
                    match parent {
                        Some(parent) => {
                            info.insert("parentID".into(), Value::from(parent));
                        }
                        None => {
                            info.remove("parentID");
                        }
                    }
                }
                (mid, role)
            };
            if role == "user" {
                last_user_mid = Some(mid.clone());
            }
            let Some(parts) = message.get_mut("parts").and_then(Value::as_array_mut) else {
                continue;
            };
            for (ordinal, part) in parts.iter_mut().enumerate() {
                let Some(entries) = part.as_object_mut() else {
                    continue;
                };
                entries.insert("id".into(), Value::from(new_ordered_id("prt", ordinal)));
                entries.insert("messageID".into(), Value::from(mid.as_str()));
                entries.insert("sessionID".into(), Value::from(sid));
                if entries.get("tool") != Some(&Value::from("task")) {
                    continue;
                }
                let mut state = object(entries.get("state"));
                let mut metadata = object(state.get("metadata"));
                metadata.insert("parentSessionId".into(), Value::from(sid));
                if let Some(child_id) = metadata.get("sessionId").and_then(Value::as_str) {
                    if let Some(target) = sid_map.get(child_id) {
                        metadata.insert("sessionId".into(), Value::from(target.as_str()));
                    }
                }
                state.insert("metadata".into(), Value::Object(metadata));
                entries.insert("state".into(), Value::Object(state));
            }
        }
    }
    normalize_payload_message_times(&mut payload);
    payload
}

/// 给尚未在 payload 里出现的子会话补 task part（必要时合成消息）。
pub fn ensure_task_links(
    payload: &mut Value,
    session: &Session,
    sid: &str,
    sid_map: &BTreeMap<String, String>,
    templates: &Map<String, Value>,
) -> DomainResult<()> {
    let mut linked: Vec<String> = Vec::new();
    for message in payload
        .get("messages")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        for part in message
            .get("parts")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            if part.get("tool") != Some(&Value::from("task")) {
                continue;
            }
            let state = object(part.get("state"));
            let metadata = object(state.get("metadata"));
            if let Some(child_id) = metadata
                .get("sessionId")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
            {
                linked.push(child_id.to_string());
                if let Some(target) = sid_map.get(child_id) {
                    linked.push(target.clone());
                }
            }
        }
    }

    let mut last_user: Option<String> =
        payload
            .get("messages")
            .and_then(Value::as_array)
            .and_then(|messages| {
                messages.iter().rev().find_map(|message| {
                    let info = object(message.get("info"));
                    (info.get("role") == Some(&Value::from("user")))
                        .then(|| info.get("id").and_then(Value::as_str).map(str::to_string))
                        .flatten()
                })
            });

    let mut now = payload
        .get("messages")
        .and_then(Value::as_array)
        .map(|messages| {
            messages
                .iter()
                .filter_map(|message| {
                    let info = object(message.get("info"));
                    let time = object(info.get("time"));
                    time.get("created").and_then(Value::as_i64)
                })
                .max()
        })
        .unwrap_or_default()
        .unwrap_or_else(now_ms)
        + 1;

    let edges: BTreeMap<&str, &AgentEdge> = session
        .agent_edges
        .iter()
        .map(|edge| (edge.child_session_id.as_str(), edge))
        .collect();

    for child in &session.children {
        let target_child = sid_map.get(&child.source_id).ok_or_else(|| {
            DomainError::internal(format!("OpenCode 子会话缺少目标 id: {}", child.source_id))
        })?;
        if linked.iter().any(|item| item == target_child) {
            continue;
        }
        let edge = edges.get(child.source_id.as_str()).copied();
        let spawn_index = edge
            .and_then(|edge| edge.spawn_message_id.clone())
            .filter(|value| !value.is_empty())
            .and_then(|spawn| {
                payload
                    .get("messages")
                    .and_then(Value::as_array)
                    .and_then(|messages| {
                        messages.iter().position(|message| {
                            object(message.get("info")).get("id")
                                == Some(&Value::from(spawn.as_str()))
                        })
                    })
            });

        if let Some(index) = spawn_index {
            let messages = payload
                .get_mut("messages")
                .and_then(Value::as_array_mut)
                .expect("上面已确认 messages 是数组");
            let message = &mut messages[index];
            let info = object(message.get("info"));
            let when = iso_ms(
                object(info.get("time"))
                    .get("created")
                    .unwrap_or(&Value::Null),
            )
            .unwrap_or(now);
            let mid = info
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let ordinal = message
                .get("parts")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0);
            let part = task_part(
                templates,
                sid,
                &mid,
                ordinal,
                child,
                target_child,
                edge,
                when,
                None,
            );
            let entries = message
                .as_object_mut()
                .ok_or_else(|| DomainError::internal("OpenCode payload 消息不是对象"))?;
            match entries.get_mut("parts").and_then(Value::as_array_mut) {
                Some(parts) => parts.push(part),
                None => {
                    entries.insert("parts".into(), Value::Array(vec![part]));
                }
            }
            if info.get("role") == Some(&Value::from("assistant")) {
                let completed = iso_ms(
                    object(info.get("time"))
                        .get("completed")
                        .unwrap_or(&Value::Null),
                )
                .unwrap_or(when)
                .max(when);
                let message_info = entries
                    .get_mut("info")
                    .and_then(Value::as_object_mut)
                    .ok_or_else(|| DomainError::internal("OpenCode payload info 不是对象"))?;
                message_info.insert("finish".into(), Value::from("tool-calls"));
                let mut time = object(message_info.get("time"));
                time.insert("completed".into(), Value::from(completed));
                message_info.insert("time".into(), Value::Object(time));
            }
            linked.push(target_child.clone());
            continue;
        }

        // spawn 消息不在 payload 里：合成一条 assistant 消息挂 task part。
        let mid = new_id("msg");
        let cwd = object(payload.get("info"))
            .get("directory")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let provider_id = session
            .model_provider
            .clone()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_PROVIDER.to_string());
        let model_id = session
            .model
            .clone()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_MODEL.to_string());
        if last_user.is_none() {
            let synthetic = new_id("msg");
            let mut user_info = object(templates.get("msg.user"));
            user_info.insert("id".into(), Value::from(synthetic.as_str()));
            user_info.insert("sessionID".into(), Value::from(sid));
            user_info.insert("role".into(), Value::from("user"));
            user_info.insert("time".into(), serde_json::json!({"created": now - 1}));
            user_info.insert("agent".into(), Value::from("build"));
            user_info.insert(
                "model".into(),
                serde_json::json!({"providerID": provider_id, "modelID": model_id}),
            );
            user_info.insert("summary".into(), serde_json::json!({"diffs": []}));
            let mut user_part = object(templates.get("part.text"));
            user_part.insert("id".into(), Value::from(new_ordered_id("prt", 0)));
            user_part.insert("messageID".into(), Value::from(synthetic.as_str()));
            user_part.insert("sessionID".into(), Value::from(sid));
            user_part.insert("type".into(), Value::from("text"));
            user_part.insert("text".into(), Value::from("[Migrated subagent task]"));
            push_message(
                payload,
                serde_json::json!({"info": Value::Object(user_info),
                                   "parts": [Value::Object(user_part)]}),
            );
            last_user = Some(synthetic);
        }
        let mut message_info = object(templates.get("msg.assistant"));
        message_info.insert("id".into(), Value::from(mid.as_str()));
        message_info.insert("sessionID".into(), Value::from(sid));
        message_info.insert(
            "time".into(),
            serde_json::json!({"created": now, "completed": now}),
        );
        message_info.insert("finish".into(), Value::from("tool-calls"));
        message_info.insert("mode".into(), Value::from("build"));
        message_info.insert("agent".into(), Value::from("build"));
        message_info.insert("path".into(), serde_json::json!({"cwd": cwd, "root": cwd}));
        message_info.insert("cost".into(), Value::from(0));
        message_info.insert(
            "tokens".into(),
            serde_json::json!({"total": 0, "input": 0, "output": 0, "reasoning": 0,
                               "cache": {"write": 0, "read": 0}}),
        );
        message_info.insert("modelID".into(), Value::from(model_id.as_str()));
        message_info.insert("providerID".into(), Value::from(provider_id.as_str()));
        match &last_user {
            Some(parent) => {
                message_info.insert("parentID".into(), Value::from(parent.as_str()));
            }
            None => {
                message_info.remove("parentID");
            }
        }
        let part = task_part(
            templates,
            sid,
            &mid,
            0,
            child,
            target_child,
            edge,
            now,
            None,
        );
        push_message(
            payload,
            serde_json::json!({"info": Value::Object(message_info), "parts": [part]}),
        );
        linked.push(target_child.clone());
        now += 1;
    }

    if !session.children.is_empty() {
        if let Some(info) = payload.get_mut("info").and_then(Value::as_object_mut) {
            let mut time = object(info.get("time"));
            time.insert("updated".into(), Value::from(now - 1));
            info.insert("time".into(), Value::Object(time));
        }
    }
    Ok(())
}

fn push_message(payload: &mut Value, message: Value) {
    let Some(entries) = payload.as_object_mut() else {
        return;
    };
    match entries.get_mut("messages").and_then(Value::as_array_mut) {
        Some(messages) => messages.push(message),
        None => {
            entries.insert("messages".into(), Value::Array(vec![message]));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Block, ToolResult, ToolResultStatus};
    use serde_json::json;

    fn templates() -> Map<String, Value> {
        super::super::native_schema::templates()
    }

    fn sid_map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    #[test]
    fn ids_are_lexicographically_ordered_within_a_parent() {
        let ids: Vec<String> = (0..12).map(|index| new_ordered_id("prt", index)).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted);
        assert!(ids[0].starts_with("prt_00000000"));
        // prefix_ + 8 位序号 + 20 位随机 hex。
        assert_eq!(ids[0].len(), "prt_".len() + 8 + 20);
        // new_id: prefix_ + 12 位 hex + 14 位 urlsafe。
        assert_eq!(new_id("ses").len(), "ses_".len() + 12 + 14);
        assert_ne!(new_id("ses"), new_id("ses"));
    }

    #[test]
    fn message_times_are_strictly_increasing() {
        let mut messages = Vec::new();
        for created in [Some(100), Some(100), None, Some(50), Some(400)] {
            let mut message = Message::new("user");
            message.created_at = created.map(Timestamp::Millis);
            messages.push(message);
        }
        assert_eq!(message_times(&messages, 0), [100, 101, 102, 103, 400]);

        // 全部缺失时从 min(known) 回落，known 为空则用 now - len。
        let blank: Vec<Message> = (0..3).map(|_| Message::new("user")).collect();
        assert_eq!(message_times(&blank, 1_000), [997, 998, 999]);
    }

    #[test]
    fn canonical_payload_fills_the_official_import_defaults() {
        let mut session = Session::new("claude", "root", "/src");
        session.messages = vec![
            {
                let mut message = Message::new("user");
                message.blocks = vec![Block::text("question")];
                message.created_at = Some(Timestamp::Millis(100));
                message
            },
            {
                let mut message = Message::new("assistant");
                message.blocks = vec![Block::text("answer")];
                message.created_at = Some(Timestamp::Millis(200));
                message
            },
        ];
        let (payload, loss) = canonical_payload(
            &session,
            "ses_abcdefgh12",
            "/dst",
            None,
            &templates(),
            &BTreeMap::new(),
            None,
        )
        .unwrap();
        assert!(loss.is_empty());
        let info = &payload["info"];
        assert_eq!(info["id"], json!("ses_abcdefgh12"));
        assert_eq!(info["directory"], json!("/dst"));
        assert_eq!(info["slug"], json!("ferry-cdefgh12"));
        assert_eq!(info["projectID"], json!("global"));
        assert_eq!(info["agent"], json!("build"));
        assert_eq!(info["cost"], json!(0));
        assert_eq!(info["time"], json!({"created": 100, "updated": 200}));
        assert!(info.get("parentID").is_none());

        let assistant = &payload["messages"][1]["info"];
        assert_eq!(assistant["finish"], json!("stop"));
        assert_eq!(assistant["time"], json!({"created": 200, "completed": 200}));
        assert_eq!(assistant["modelID"], json!("gpt-5.6-sol"));
        assert_eq!(assistant["providerID"], json!("openai"));
        assert_eq!(assistant["parentID"], payload["messages"][0]["info"]["id"]);
        assert_eq!(
            payload["messages"][0]["info"]["model"],
            json!({"providerID": "openai", "modelID": "gpt-5.6-sol"})
        );
    }

    #[test]
    fn an_orphan_assistant_gets_a_synthetic_parent() {
        let mut session = Session::new("claude", "root", "/src");
        let mut message = Message::new("assistant");
        message.blocks = vec![Block::text("answer")];
        message.created_at = Some(Timestamp::Millis(200));
        session.messages = vec![message];
        let (payload, _) = canonical_payload(
            &session,
            "ses_1",
            "/dst",
            None,
            &templates(),
            &BTreeMap::new(),
            None,
        )
        .unwrap();
        let roles: Vec<&str> = payload["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|message| message["info"]["role"].as_str().unwrap())
            .collect();
        assert_eq!(roles, ["user", "assistant"]);
        assert_eq!(
            payload["messages"][0]["parts"][0]["text"],
            json!("[Migrated subagent task]")
        );
        // 第一条是 assistant → 会话创建时间比首条消息早 1 毫秒。
        assert_eq!(payload["info"]["time"]["created"], json!(199));
    }

    #[test]
    fn tool_blocks_without_a_native_form_degrade_to_narration() {
        let mut session = Session::new("claude", "root", "/src");
        let mut tool = ToolCall::new("Weird", Some("nope".into()), json!({}));
        tool.result = Some(ToolResult::new(ToolResultStatus::Success));
        let mut message = Message::new("assistant");
        let mut block = Block::new(crate::model::BlockKind::Tool);
        block.tool = Some(tool);
        message.blocks = vec![block];
        message.created_at = Some(Timestamp::Millis(10));
        session.messages = vec![message];

        let (payload, loss) = canonical_payload(
            &session,
            "ses_1",
            "/dst",
            None,
            &templates(),
            &BTreeMap::new(),
            None,
        )
        .unwrap();
        assert_eq!(loss.len(), 1);
        assert_eq!(loss[0].code, "migration.tool_degraded");
        assert_eq!(loss[0].params["tool_name"], json!("Weird"));
        let part = &payload["messages"][1]["parts"][0];
        assert_eq!(part["type"], json!("text"));
        assert!(part["text"]
            .as_str()
            .unwrap()
            .contains("History: tool Weird"));
    }

    #[test]
    fn agent_spawn_blocks_become_task_parts_with_both_metadata_ids() {
        let mut session = Session::new("claude", "root", "/src");
        let mut child = Session::new("claude", "child", "/src");
        child.title = "reviewer".into();
        let mut child_message = Message::new("assistant");
        child_message.blocks = vec![Block::text("review complete")];
        child.messages = vec![child_message];
        session.children = vec![child];
        let mut edge = AgentEdge::new("root", "child");
        edge.source_call_id = Some("call-task".into());
        edge.spawn_message_id = Some("spawn".into());
        edge.prompt = "review".into();
        session.agent_edges = vec![edge];

        let mut tool = ToolCall::new("Task", Some(CanonicalOp::AGENT_SPAWN.into()), json!({}));
        tool.source_call_id = Some("call-task".into());
        tool.result = Some(ToolResult::new(ToolResultStatus::Success));
        let mut block = Block::new(crate::model::BlockKind::Tool);
        block.tool = Some(tool);
        let mut message = Message::new("assistant");
        message.blocks = vec![block];
        message.source_id = Some("spawn".into());
        message.created_at = Some(Timestamp::Millis(200));
        session.messages = vec![message];

        let (payload, loss) = canonical_payload(
            &session,
            "ses_root",
            "/dst",
            None,
            &templates(),
            &sid_map(&[("child", "ses_child")]),
            None,
        )
        .unwrap();
        assert!(loss.is_empty());
        let parts = payload["messages"][1]["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0]["tool"], json!("task"));
        assert_eq!(parts[0]["callID"], json!("call-task"));
        assert_eq!(
            parts[0]["state"]["metadata"],
            json!({"parentSessionId": "ses_root", "sessionId": "ses_child"})
        );
        assert_eq!(parts[0]["state"]["output"], json!("review complete"));
        assert_eq!(parts[0]["state"]["input"]["description"], json!("reviewer"));
        assert_eq!(
            payload["messages"][1]["info"]["finish"],
            json!("tool-calls")
        );
    }

    #[test]
    fn remap_makes_tied_message_and_part_order_stable() {
        let task = json!({
            "id": "part-task", "messageID": "m1", "sessionID": "old-session",
            "type": "tool", "tool": "task", "callID": "call-1",
            "state": {"status": "completed", "input": {}, "output": "",
                      "metadata": {"parentSessionId": "old-session",
                                   "sessionId": "old-child"},
                      "time": {"start": 100, "end": 100}}
        });
        let message =
            |mid: &str, role: &str, created: i64, parts: Value, completed: Option<i64>| {
                let mut time = json!({"created": created});
                if let Some(completed) = completed {
                    time["completed"] = json!(completed);
                }
                json!({"info": {"id": mid, "sessionID": "old-session", "role": role,
                            "time": time},
                   "parts": parts})
            };
        let payload = json!({
            "info": {"id": "old-session", "directory": "/old",
                     "time": {"created": 100, "updated": 100}},
            "messages": [
                message("m1", "assistant", 100, json!([
                    {"id": "part-z", "messageID": "m1", "sessionID": "old-session",
                     "type": "text", "text": "first"},
                    task,
                    {"id": "part-a", "messageID": "m1", "sessionID": "old-session",
                     "type": "text", "text": "last"}
                ]), Some(100)),
                message("m2", "user", 100, json!([{"id": "p2", "type": "text",
                                                   "text": "m2"}]), None),
                message("m3", "assistant", 100, json!([{"id": "p3", "type": "text",
                                                        "text": "m3"}]), Some(100)),
            ]
        });
        let remapped = remap_payload(
            &payload,
            "new-session",
            "/new",
            None,
            &sid_map(&[("old-session", "new-session"), ("old-child", "new-child")]),
        );
        let created: Vec<i64> = remapped["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|message| message["info"]["time"]["created"].as_i64().unwrap())
            .collect();
        assert_eq!(created, [100, 101, 102]);
        let completed: Vec<i64> = remapped["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|message| message["info"]["time"].get("completed"))
            .map(|value| value.as_i64().unwrap())
            .collect();
        assert_eq!(completed, [100, 102]);
        let parts = remapped["messages"][0]["parts"].as_array().unwrap();
        let labels: Vec<&str> = parts
            .iter()
            .map(|part| {
                part.get("text")
                    .and_then(Value::as_str)
                    .or_else(|| part.get("tool").and_then(Value::as_str))
                    .unwrap()
            })
            .collect();
        assert_eq!(labels, ["first", "task", "last"]);
        let ids: Vec<&str> = parts
            .iter()
            .map(|part| part["id"].as_str().unwrap())
            .collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(ids, sorted);
        assert_eq!(
            parts[1]["state"]["metadata"],
            json!({"parentSessionId": "new-session", "sessionId": "new-child"})
        );
        assert_eq!(remapped["info"]["time"]["updated"], json!(102));
    }

    #[test]
    fn remap_normalizes_missing_and_non_dict_time_fields() {
        let message = |mid: &str, role: &str, created: Value| {
            json!({"info": {"id": mid, "sessionID": "old-root", "role": role,
                            "time": {"created": created}},
                   "parts": [{"id": format!("old-{mid}"), "type": "text", "text": mid}]})
        };
        let payload = json!({
            "info": {"id": "old-root", "directory": "/old", "time": null},
            "messages": [
                message("m1", "user", Value::Null),
                json!({"info": {"id": "m2", "role": "assistant",
                                "time": {"completed": 10}},
                       "parts": []}),
                message("m3", "user", json!("invalid-time")),
            ]
        });
        let remapped = remap_payload(&payload, "new-root", "/new", None, &BTreeMap::new());
        let created: Vec<i64> = remapped["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|message| message["info"]["time"]["created"].as_i64().unwrap())
            .collect();
        assert_eq!(created, [created[0], created[0] + 1, created[0] + 2]);
        assert_eq!(
            remapped["info"]["time"]["updated"],
            json!(created[created.len() - 1])
        );
    }

    #[test]
    fn an_empty_native_payload_still_gets_a_valid_session_clock() {
        let payload = json!({"info": {"id": "old-root", "directory": "/old", "time": null},
                             "messages": []});
        let remapped = remap_payload(&payload, "new-root", "/new", None, &BTreeMap::new());
        let created = remapped["info"]["time"]["created"].as_i64().unwrap();
        let updated = remapped["info"]["time"]["updated"].as_i64().unwrap();
        assert!(updated >= created);
    }

    #[test]
    fn ensure_task_links_appends_at_the_spawn_message() {
        let mut session = Session::new("opencode", "old-root", "/src");
        let mut child = Session::new("opencode", "old-child", "/src");
        child.title = "child".into();
        session.children = vec![child];
        let mut edge = AgentEdge::new("old-root", "old-child");
        edge.spawn_message_id = Some("spawn".into());
        edge.prompt = "review".into();
        session.agent_edges = vec![edge];

        let mut payload = json!({
            "info": {"id": "old-root", "directory": "/src",
                     "time": {"created": 100, "updated": 300}},
            "messages": [
                {"info": {"id": "u1", "role": "user", "time": {"created": 100}},
                 "parts": []},
                {"info": {"id": "spawn", "role": "assistant",
                          "time": {"created": 200, "completed": 200}},
                 "parts": []},
            ]
        });
        ensure_task_links(
            &mut payload,
            &session,
            "ses_root",
            &sid_map(&[("old-child", "ses_child")]),
            &templates(),
        )
        .unwrap();
        assert_eq!(payload["messages"].as_array().unwrap().len(), 2);
        let parts = payload["messages"][1]["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0]["tool"], json!("task"));
        assert_eq!(
            parts[0]["state"]["metadata"],
            json!({"parentSessionId": "ses_root", "sessionId": "ses_child"})
        );
        assert_eq!(
            payload["messages"][1]["info"]["finish"],
            json!("tool-calls")
        );
    }

    #[test]
    fn ensure_task_links_synthesises_a_message_when_the_spawn_is_missing() {
        let mut session = Session::new("opencode", "root", "/src");
        session.children = vec![Session::new("opencode", "child", "/src")];
        let mut payload = json!({"info": {"id": "root", "directory": "/src", "time": {}},
                                 "messages": []});
        ensure_task_links(
            &mut payload,
            &session,
            "ses_root",
            &sid_map(&[("child", "ses_child")]),
            &templates(),
        )
        .unwrap();
        let roles: Vec<&str> = payload["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|message| message["info"]["role"].as_str().unwrap())
            .collect();
        assert_eq!(roles, ["user", "assistant"]);
        assert_eq!(
            payload["messages"][1]["info"]["parentID"],
            payload["messages"][0]["info"]["id"]
        );
        assert_eq!(payload["messages"][1]["parts"][0]["tool"], json!("task"));
    }

    #[test]
    fn already_linked_children_are_not_duplicated() {
        let mut session = Session::new("opencode", "root", "/src");
        session.children = vec![Session::new("opencode", "child", "/src")];
        let mut payload = json!({
            "info": {"id": "root", "directory": "/src", "time": {}},
            "messages": [{"info": {"id": "m", "role": "assistant"},
                          "parts": [{"id": "p", "tool": "task",
                                     "state": {"metadata": {"sessionId": "child"}}}]}]
        });
        ensure_task_links(
            &mut payload,
            &session,
            "ses_root",
            &sid_map(&[("child", "ses_child")]),
            &templates(),
        )
        .unwrap();
        assert_eq!(payload["messages"].as_array().unwrap().len(), 1);
        assert_eq!(payload["messages"][0]["parts"].as_array().unwrap().len(), 1);
    }
}
