//! 规范会话读取、树装配与 RPC DTO。
//!
//! 语义事实源：`engine/sessions/read.py`。
//!
//! 树装配复用 `adapters::shared::migration::assemble_tree`（WP-B2），本模块只
//! 提供 `read_tree`（按 tool 名解析 adapter + 缓存）这一层门面。

use serde_json::{Map, Value};

use crate::adapters::contracts::ScanRow;
use crate::errors::{DomainError, DomainResult};
use crate::model::{tool_result_text, Block, BlockKind, Message, Session, Timestamp};

use super::index::SessionPorts;
use crate::tool_ops::CanonicalOp;

/// UI 浏览路径的默认分页大小。
pub const DEFAULT_BROWSER_MESSAGE_LIMIT: i64 = 30;

/// 消息 locator 的签发器（Agent 与 UI 共用同一 `fml_` 键）。
pub type LocatorIssuer<'a> = &'a dyn Fn(&Message, usize) -> DomainResult<String>;

/// `str(value)` 的最小等价物：canonical 模型里 `ToolCall.input` 只可能是
/// dict 或 str，其余形态是契约违例，这里按 Python `str()` 的形状兜底。
fn python_str(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Number(number) => number.to_string(),
        other => other.to_string(),
    }
}

/// Python 的真值判定：`None`/`False`/`0`/`""`/`[]`/`{}` 为假。
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|value| value != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(entries) => !entries.is_empty(),
    }
}

/// `_tool_view`：`tool.invoke` 会把真实工具名与参数藏在 input 里。
fn tool_view(call: &crate::model::ToolCall) -> (String, Value) {
    let mut value = if call.input.is_object() {
        call.input.clone()
    } else {
        Value::from(python_str(&call.input))
    };
    let mut name = call.name.clone();
    if call.op.as_deref() == Some(CanonicalOp::TOOL_INVOKE) {
        if let Some(input) = call.input.as_object() {
            // Python 是 `str(call.input.get("name") or name)`：真值就 `str()` 后
            // 采纳，不要求它已经是字符串。
            if let Some(inner) = input.get("name").filter(|value| truthy(value)) {
                name = python_str(inner);
            }
            if let Some(inner) = input.get("input") {
                value = inner.clone();
            }
        }
    }
    (name, value)
}

fn timestamp_value(value: &Option<Timestamp>) -> Value {
    match value {
        Some(Timestamp::Millis(millis)) => Value::from(*millis),
        Some(Timestamp::Text(text)) => Value::from(text.as_str()),
        None => Value::Null,
    }
}

pub use crate::adapters::shared::migration::assemble_tree;

/// `read_tree(tool, ref, ports)`：按工具名取 adapter 与扫描缓存后装配整棵树。
pub fn read_tree(tool: &str, reference: &str, ports: &dyn SessionPorts) -> DomainResult<Session> {
    let adapter = ports.adapter(tool)?;
    let cache = ports.cache_factory();
    assemble_tree(adapter.require_browser()?, reference, cache.as_ref())
}

fn block_dto(block: &Block) -> Option<Map<String, Value>> {
    let mut entry = Map::new();
    match block.kind {
        BlockKind::Text => {
            entry.insert("kind".into(), Value::from("text"));
            entry.insert("text".into(), Value::from(block.text.as_str()));
            entry.insert("size".into(), Value::from(block.text.chars().count()));
        }
        BlockKind::Tool => {
            let call = block.tool.as_ref()?;
            let (name, value) = tool_view(call);
            let output = tool_result_text(call.result.as_ref());
            entry.insert("kind".into(), Value::from("tool"));
            entry.insert("name".into(), Value::from(name));
            entry.insert(
                "op".into(),
                call.op.as_deref().map(Value::from).unwrap_or(Value::Null),
            );
            entry.insert("input".into(), value);
            entry.insert("output".into(), Value::from(output.as_str()));
            entry.insert("size".into(), Value::from(output.chars().count()));
        }
        BlockKind::Image => {
            let image = block.image.as_ref()?;
            let mut payload = Map::new();
            payload.insert("id".into(), Value::from(image.id.as_str()));
            payload.insert("mime_type".into(), Value::from(image.mime_type.as_str()));
            payload.insert(
                "filename".into(),
                image
                    .filename
                    .as_deref()
                    .map(Value::from)
                    .unwrap_or(Value::Null),
            );
            entry.insert("kind".into(), Value::from("image"));
            entry.insert("image".into(), Value::Object(payload));
        }
        // thinking 不进浏览视图（Python 的 if/elif 链没有这一支）。
        BlockKind::Thinking => return None,
    }
    Some(entry)
}

fn messages_dto(
    messages: &[&Message],
    offset: usize,
    locator_issuer: Option<LocatorIssuer<'_>>,
) -> DomainResult<Vec<Map<String, Value>>> {
    let mut result = Vec::with_capacity(messages.len());
    for (index, message) in messages.iter().enumerate() {
        let blocks: Vec<Value> = message
            .blocks
            .iter()
            .filter_map(block_dto)
            .map(Value::Object)
            .collect();
        let mut entry = Map::new();
        entry.insert("index".into(), Value::from(offset + index));
        entry.insert("role".into(), Value::from(message.role.as_str()));
        entry.insert("blocks".into(), Value::Array(blocks));
        let locator = match locator_issuer {
            Some(issue) => issue(message, offset + index)?,
            None => match message.source_id.as_deref() {
                // 注意：Python 的兜底分支用的是页内序号 `index` 而不是
                // `offset + index`，这里逐字复刻（下游只当不透明串用）。
                Some(source_id) if !source_id.is_empty() => source_id.to_string(),
                _ => format!("index:{index}"),
            },
        };
        entry.insert("locator".into(), Value::from(locator));
        result.push(entry);
    }
    Ok(result)
}

fn context_compactions_dto(session: &Session, messages: &[&Message]) -> Vec<Map<String, Value>> {
    let mut turn = 0i64;
    let mut turn_by_message: Map<String, Value> = Map::new();
    for message in messages {
        if message.role == "user" {
            turn += 1;
        }
        if let Some(source_id) = message.source_id.as_deref() {
            if !source_id.is_empty() {
                turn_by_message.insert(source_id.to_string(), Value::from(turn));
            }
        }
    }
    session
        .context_compactions
        .iter()
        .enumerate()
        .map(|(position, compaction)| {
            let mut entry = Map::new();
            entry.insert("id".into(), Value::from(compaction.id.as_str()));
            entry.insert("source".into(), Value::from(compaction.source.as_str()));
            entry.insert("sequence".into(), Value::from(position as i64 + 1));
            entry.insert(
                "after_turn".into(),
                compaction
                    .after_message_id
                    .as_deref()
                    .and_then(|id| turn_by_message.get(id).cloned())
                    .unwrap_or(Value::from(0)),
            );
            entry.insert(
                "after_message_locator".into(),
                compaction
                    .after_message_id
                    .as_deref()
                    .map(Value::from)
                    .unwrap_or(Value::Null),
            );
            entry.insert(
                "event_locator".into(),
                compaction
                    .event_locator
                    .as_deref()
                    .map(Value::from)
                    .unwrap_or(Value::Null),
            );
            entry.insert("created_at".into(), timestamp_value(&compaction.created_at));
            entry.insert("trigger".into(), Value::from(compaction.trigger.as_str()));
            entry.insert("state".into(), Value::from(compaction.state.as_str()));
            let mut summary = Map::new();
            summary.insert(
                "status".into(),
                Value::from(compaction.summary_status.as_str()),
            );
            summary.insert("text".into(), Value::from(compaction.summary_text.as_str()));
            summary.insert(
                "locator".into(),
                compaction
                    .summary_message_id
                    .as_deref()
                    .map(Value::from)
                    .unwrap_or(Value::Null),
            );
            entry.insert("summary".into(), Value::Object(summary));
            let mut tail = Map::new();
            tail.insert(
                "status".into(),
                Value::from(compaction.tail_status.as_str()),
            );
            tail.insert(
                "start_locator".into(),
                compaction
                    .tail_start_locator
                    .as_deref()
                    .map(Value::from)
                    .unwrap_or(Value::Null),
            );
            tail.insert(
                "start_message_index".into(),
                compaction
                    .tail_start_message_index
                    .map(Value::from)
                    .unwrap_or(Value::Null),
            );
            entry.insert("tail".into(), Value::Object(tail));
            entry.insert("metrics".into(), Value::Object(compaction.metrics.clone()));
            entry
        })
        .collect()
}

fn context_status(compactions: &[Map<String, Value>]) -> Map<String, Value> {
    let mut status = Map::new();
    if compactions.is_empty() {
        status.insert("state".into(), Value::from("full"));
        status.insert("compaction_count".into(), Value::from(0));
        status.insert("summary_status".into(), Value::from("not_applicable"));
        return status;
    }
    let state_of = |name: &str| {
        compactions
            .iter()
            .any(|item| item.get("state").and_then(Value::as_str) == Some(name))
    };
    let state = if state_of("in_progress") {
        "in_progress"
    } else if state_of("incomplete") {
        "incomplete"
    } else {
        "compacted"
    };
    let latest = compactions.last().expect("非空");
    status.insert("state".into(), Value::from(state));
    status.insert("compaction_count".into(), Value::from(compactions.len()));
    status.insert(
        "summary_status".into(),
        latest
            .get("summary")
            .and_then(|summary| summary.get("status"))
            .cloned()
            .unwrap_or(Value::Null),
    );
    status
}

/// 分页只在下一条用户消息前截断，避免把同一轮的 AI 回复拆开。
fn page_messages<'a>(
    messages: &'a [&'a Message],
    start: i64,
    limit: Option<i64>,
) -> (usize, usize, &'a [&'a Message]) {
    // Python `first = max(0, start - 1)`：**不**向上钳到长度，越界时页为空。
    let length = messages.len();
    let first = start.saturating_sub(1).max(0).min(length as i64 + 1) as usize;
    let Some(limit) = limit else {
        let page = messages.get(first..).unwrap_or(&[]);
        return (first, length, page);
    };
    let mut last = length.min(first.saturating_add(limit.max(1) as usize));
    while last < length && messages[last].role != "user" {
        last += 1;
    }
    let page = if first >= last {
        &[][..]
    } else {
        &messages[first..last]
    };
    (first, last, page)
}

/// `session_json` 的可选参数集合；Python 那边是一串关键字参数。
#[derive(Clone, Copy, Debug)]
pub struct SessionJsonOptions {
    pub from_message: i64,
    pub message_limit: Option<i64>,
    pub include_messages: bool,
    pub include_tree: bool,
    pub tree_count: Option<i64>,
    pub child_count: Option<i64>,
    pub total_count: Option<i64>,
}

impl Default for SessionJsonOptions {
    fn default() -> Self {
        Self {
            from_message: 1,
            message_limit: None,
            include_messages: true,
            include_tree: true,
            tree_count: None,
            child_count: None,
            total_count: None,
        }
    }
}

const EDGE_FIELDS: [&str; 12] = [
    "parent_session_id",
    "child_session_id",
    "source_call_id",
    "spawn_message_id",
    "result_message_id",
    "agent_id",
    "agent_path",
    "agent_type",
    "prompt",
    "status",
    "association",
    "confidence",
];

pub fn session_json(
    session: &Session,
    options: SessionJsonOptions,
    locator_issuer: Option<LocatorIssuer<'_>>,
) -> DomainResult<Map<String, Value>> {
    let children: Vec<Map<String, Value>> = if options.include_tree {
        session
            .children
            .iter()
            .map(|child| session_json(child, SessionJsonOptions::default(), None))
            .collect::<DomainResult<_>>()?
    } else {
        Vec::new()
    };
    let edges: Vec<Value> = session
        .agent_edges
        .iter()
        .map(|edge| {
            let value = serde_json::to_value(edge).unwrap_or(Value::Null);
            let mut entry = Map::new();
            for field in EDGE_FIELDS {
                entry.insert(
                    field.into(),
                    value.get(field).cloned().unwrap_or(Value::Null),
                );
            }
            Value::Object(entry)
        })
        .collect();
    // compaction 的 summary 消息是内部产物，不出现在编号里。
    let internal: Vec<&str> = session
        .context_compactions
        .iter()
        .filter_map(|compaction| compaction.summary_message_id.as_deref())
        .filter(|id| !id.is_empty())
        .collect();
    let display_messages: Vec<&Message> = session
        .messages
        .iter()
        .filter(|message| match message.source_id.as_deref() {
            Some(source_id) => !internal.contains(&source_id),
            None => true,
        })
        .collect();
    let (first, last, page) = page_messages(
        &display_messages,
        options.from_message,
        options.message_limit,
    );
    let messages = messages_dto(page, first, locator_issuer)?;
    let compactions = context_compactions_dto(session, &display_messages);

    let turn_offset = display_messages[..first.min(display_messages.len())]
        .iter()
        .filter(|message| message.role == "user")
        .count() as i64;
    let mut turns: Vec<Value> = Vec::new();
    for message in &messages {
        let role = message.get("role").and_then(Value::as_str).unwrap_or("");
        let blocks = message
            .get("blocks")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if role == "user" {
            let mut turn = Map::new();
            turn.insert(
                "turn".into(),
                Value::from(turn_offset + turns.len() as i64 + 1),
            );
            turn.insert("user".into(), Value::Object(message.clone()));
            turn.insert(
                "turn_locator".into(),
                message.get("locator").cloned().unwrap_or(Value::Null),
            );
            let mut reply = Map::new();
            reply.insert("items".into(), Value::Array(Vec::new()));
            turn.insert("assistant_reply".into(), Value::Object(reply));
            turns.push(Value::Object(turn));
        } else if role == "assistant" {
            let Some(current) = turns.last_mut() else {
                continue;
            };
            let items = current["assistant_reply"]["items"]
                .as_array_mut()
                .expect("assistant_reply.items 恒为数组");
            for block in blocks {
                match block.get("kind").and_then(Value::as_str) {
                    Some("text") => {
                        let mut item = Map::new();
                        item.insert("kind".into(), Value::from("text"));
                        item.insert(
                            "text".into(),
                            block.get("text").cloned().unwrap_or(Value::Null),
                        );
                        items.push(Value::Object(item));
                    }
                    Some("tool") => {
                        let mut item = Map::new();
                        item.insert("kind".into(), Value::from("tool"));
                        for field in ["name", "input", "output"] {
                            item.insert(
                                field.into(),
                                block.get(field).cloned().unwrap_or(Value::Null),
                            );
                        }
                        items.push(Value::Object(item));
                    }
                    _ => {}
                }
            }
        }
    }

    let mut payload = Map::new();
    payload.insert("tool".into(), Value::from(session.source_tool.as_str()));
    payload.insert("id".into(), Value::from(session.source_id.as_str()));
    payload.insert("title".into(), Value::from(session.title.as_str()));
    payload.insert("dir".into(), Value::from(session.cwd.as_str()));
    payload.insert(
        "root_id".into(),
        Value::from(
            session
                .root_id
                .as_deref()
                .filter(|value| !value.is_empty())
                .unwrap_or(session.source_id.as_str()),
        ),
    );
    payload.insert(
        "parent_id".into(),
        session
            .parent_id
            .as_deref()
            .map(Value::from)
            .unwrap_or(Value::Null),
    );
    for (key, value) in [
        ("agent_id", &session.agent_id),
        ("agent_path", &session.agent_path),
        ("agent_type", &session.agent_type),
    ] {
        payload.insert(
            key.into(),
            value.as_deref().map(Value::from).unwrap_or(Value::Null),
        );
    }
    payload.insert(
        "count".into(),
        Value::from(options.total_count.unwrap_or(display_messages.len() as i64)),
    );
    payload.insert(
        "root_message_count".into(),
        Value::from(display_messages.len()),
    );
    payload.insert("returned_message_count".into(), Value::from(messages.len()));
    let mut range = Map::new();
    range.insert(
        "from".into(),
        if messages.is_empty() {
            Value::Null
        } else {
            Value::from(first + 1)
        },
    );
    range.insert(
        "to".into(),
        if messages.is_empty() {
            Value::Null
        } else {
            Value::from(last)
        },
    );
    payload.insert("message_range".into(), Value::Object(range));
    payload.insert(
        "next_from_message".into(),
        if last < display_messages.len() {
            Value::from(last + 1)
        } else {
            Value::Null
        },
    );
    payload.insert(
        "context".into(),
        Value::Object(context_status(&compactions)),
    );
    payload.insert(
        "context_compactions".into(),
        Value::Array(compactions.into_iter().map(Value::Object).collect()),
    );
    payload.insert(
        "child_count".into(),
        Value::from(options.child_count.unwrap_or(children.len() as i64)),
    );
    payload.insert(
        "tree_count".into(),
        Value::from(options.tree_count.unwrap_or_else(|| {
            1 + children
                .iter()
                .map(|child| child.get("tree_count").and_then(Value::as_i64).unwrap_or(0))
                .sum::<i64>()
        })),
    );
    payload.insert(
        "loss".into(),
        serde_json::to_value(&session.loss).unwrap_or(Value::Array(Vec::new())),
    );
    payload.insert(
        "messages".into(),
        Value::Array(if options.include_messages {
            messages.into_iter().map(Value::Object).collect()
        } else {
            Vec::new()
        }),
    );
    payload.insert("turns".into(), Value::Array(turns));
    payload.insert(
        "children".into(),
        Value::Array(children.into_iter().map(Value::Object).collect()),
    );
    payload.insert("agent_edges".into(), Value::Array(edges));
    Ok(payload)
}

/// `show` RPC 的默认配置：不带正文、不带子树。
pub fn show_options() -> SessionJsonOptions {
    SessionJsonOptions {
        from_message: 1,
        message_limit: Some(DEFAULT_BROWSER_MESSAGE_LIMIT),
        include_messages: false,
        include_tree: false,
        ..SessionJsonOptions::default()
    }
}

pub fn show(
    session: &Session,
    options: SessionJsonOptions,
    locator_issuer: Option<LocatorIssuer<'_>>,
) -> DomainResult<Map<String, Value>> {
    session_json(
        session,
        SessionJsonOptions {
            include_tree: false,
            ..options
        },
        locator_issuer,
    )
}

/// 按 asset_id 找回图片原数据。
pub fn session_asset(session: &Session, asset_id: &str) -> DomainResult<Map<String, Value>> {
    for node in session.walk() {
        for message in &node.messages {
            for block in &message.blocks {
                let Some(image) = block.image.as_ref() else {
                    continue;
                };
                if block.kind != BlockKind::Image || image.id != asset_id {
                    continue;
                }
                let mut payload = Map::new();
                payload.insert("mime_type".into(), Value::from(image.mime_type.as_str()));
                payload.insert("data".into(), Value::from(image.data.as_str()));
                payload.insert(
                    "filename".into(),
                    image
                        .filename
                        .as_deref()
                        .map(Value::from)
                        .unwrap_or(Value::Null),
                );
                return Ok(payload);
            }
        }
    }
    Err(DomainError::session_asset_not_found(asset_id))
}

/// 扫描行类型 re-export，方便调用方拼装。
pub type Row = ScanRow;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ImageAsset, ToolCall};
    use serde_json::json;

    fn user(text: &str) -> Message {
        let mut message = Message::new("user");
        message.blocks.push(Block::text(text));
        message
    }

    fn assistant(text: &str) -> Message {
        let mut message = Message::new("assistant");
        message.blocks.push(Block::text(text));
        message
    }

    #[test]
    fn paging_never_splits_a_turn() {
        let messages = [
            user("u1"),
            assistant("a1"),
            assistant("a2"),
            user("u2"),
            assistant("a3"),
        ];
        let borrowed: Vec<&Message> = messages.iter().collect();
        // limit=1 但必须一直取到下一条 user 之前。
        let (first, last, page) = page_messages(&borrowed, 1, Some(1));
        assert_eq!((first, last, page.len()), (0, 3, 3));
        let (first, last, page) = page_messages(&borrowed, 4, Some(1));
        assert_eq!((first, last, page.len()), (3, 5, 2));
        // limit=None → 一直到底。
        let (first, last, _) = page_messages(&borrowed, 2, None);
        assert_eq!((first, last), (1, 5));
    }

    #[test]
    fn compaction_summary_messages_are_removed_before_numbering() {
        let mut session = Session::new("claude", "s1", "/tmp");
        let mut summary = assistant("internal summary");
        summary.source_id = Some("sum-1".into());
        let mut visible = user("hello");
        visible.source_id = Some("m-1".into());
        session.messages = vec![summary, visible];
        let mut compaction = crate::model::ContextCompaction::new("c1", "native");
        compaction.summary_message_id = Some("sum-1".into());
        session.context_compactions.push(compaction);

        let payload = session_json(&session, SessionJsonOptions::default(), None).unwrap();
        assert_eq!(payload["root_message_count"], Value::from(1));
        assert_eq!(payload["messages"][0]["index"], Value::from(0));
        assert_eq!(payload["messages"][0]["locator"], Value::from("m-1"));
        assert_eq!(payload["context"]["state"], Value::from("compacted"));
        assert_eq!(payload["context"]["compaction_count"], Value::from(1));
    }

    #[test]
    fn turns_group_assistant_replies_under_the_preceding_user() {
        let mut session = Session::new("claude", "s1", "/tmp");
        session.messages = vec![user("q"), assistant("a"), user("q2")];
        let payload = session_json(&session, SessionJsonOptions::default(), None).unwrap();
        let turns = payload["turns"].as_array().unwrap();
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0]["turn"], Value::from(1));
        assert_eq!(
            turns[0]["assistant_reply"]["items"][0]["text"],
            Value::from("a")
        );
        assert!(turns[1]["assistant_reply"]["items"]
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn tool_view_unwraps_tool_invoke() {
        let call = ToolCall::new(
            "mcp",
            Some(CanonicalOp::TOOL_INVOKE.to_string()),
            json!({"namespace": "n", "name": "real", "input": {"a": 1}}),
        );
        assert_eq!(tool_view(&call), ("real".to_string(), json!({"a": 1})));
        let plain = ToolCall::new("Bash", Some(CanonicalOp::SHELL_EXEC.into()), json!("ls"));
        assert_eq!(tool_view(&plain), ("Bash".to_string(), json!("ls")));
    }

    #[test]
    fn session_asset_walks_the_whole_tree() {
        let mut child = Session::new("claude", "c", "/tmp");
        let mut message = Message::new("assistant");
        let mut block = Block::new(BlockKind::Image);
        block.image = Some(ImageAsset {
            id: "img-1".into(),
            mime_type: "image/png".into(),
            data: "AAA".into(),
            filename: None,
        });
        message.blocks.push(block);
        child.messages.push(message);
        let mut root = Session::new("claude", "r", "/tmp");
        root.children.push(child);

        let asset = session_asset(&root, "img-1").unwrap();
        assert_eq!(asset["mime_type"], Value::from("image/png"));
        assert_eq!(asset["data"], Value::from("AAA"));
        let error = session_asset(&root, "missing").unwrap_err();
        assert_eq!(error.code, "session.asset_not_found");
    }
}
