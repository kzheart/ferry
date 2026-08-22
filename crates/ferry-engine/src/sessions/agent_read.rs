//! 供 Ferry Agent 使用的限量会话读取。
//!
//! 预算口径：DTO 上限 64 KiB，默认上下文预算 24 KiB；`take` 按 **UTF-8 字节**
//! 计数（与 `safety::truncate_text` 的字符口径相对）。
//!
//! `fit_context_result` 的游标语义是分页正确性的关键：弹掉尾部消息时必须把
//! `next_from_message` **下调**到被弹消息的编号，否则调用方按游标翻页会在同一
//! 条消息上死循环；只剩一条时改为砍掉最大 text block 的一半而不是弹空。

use serde_json::{Map, Value};

use crate::adapters::contracts::NativeSessionReference;
use crate::errors::{DomainError, DomainResult};
use crate::model::{native_locator, tool_result_text, BlockKind, Message, Session};

use super::index::{AgentSessionIndex, IndexedSession};
use super::safety::{
    bounded_int, bounded_json, finalize_dto, python_json, python_json_len, record_session_id,
    string_set, truncate_text, MAX_AGENT_DTO_BYTES,
};

pub const MAX_CONTENT_SEARCH_RESULTS: i64 = 50;
pub const MAX_CONTEXT_MESSAGES: i64 = 50;
pub const MAX_CONTEXT_BYTES: i64 = 64 * 1024;
pub const DEFAULT_CONTEXT_BYTES: i64 = 24 * 1024;

/// `_take`：按 UTF-8 字节裁剪；返回 (裁剪后文本, 剩余预算, 是否被裁)。
fn take(text: &str, remaining: usize) -> (String, usize, bool) {
    let encoded = text.as_bytes();
    if encoded.len() <= remaining {
        return (text.to_string(), remaining - encoded.len(), false);
    }
    // `decode(errors="ignore")` 等价：丢掉尾部不完整的码点。
    let mut boundary = remaining.min(encoded.len());
    while boundary > 0 && !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    (text[..boundary].to_string(), 0, true)
}

/// `json.dumps({"truncated": True})` 的字节数。
const TRUNCATED_MARKER_BYTES: usize = 19;

/// `_take_json`。
fn take_json(value: &Value, remaining: usize) -> (Value, usize, bool) {
    if remaining < 32 {
        return (Value::Object(Map::new()), remaining.saturating_sub(2), true);
    }
    let bounded = bounded_json(value, 128.max(remaining.min(12 * 1024)));
    let encoded = python_json_len(&bounded);
    if encoded <= remaining {
        let changed = bounded != *value;
        return (bounded, remaining - encoded, changed);
    }
    if TRUNCATED_MARKER_BYTES <= remaining {
        let mut marker = Map::new();
        marker.insert("truncated".into(), Value::Bool(true));
        return (
            Value::Object(marker),
            remaining - TRUNCATED_MARKER_BYTES,
            true,
        );
    }
    (Value::Object(Map::new()), remaining.saturating_sub(2), true)
}

/// 读取前后各做一次 `validate_read_scope`，中间夹一次 `resolve` 的钉内容校验。
///
/// 三明治结构是编辑安全的基础：读之前确认引用在 Agent 根内，读之后再确认一次
/// 引用没有被替换掉（符号链接换指向等）。
pub fn read_indexed_session(
    index: &AgentSessionIndex,
    record: &IndexedSession,
    pin_content: bool,
) -> DomainResult<Session> {
    let browser = index.ports().adapter(&record.tool)?.require_browser()?;
    let native_ref = NativeSessionReference::new(
        record.canonical_ref.clone(),
        record.root.clone(),
        record.storage_kind,
    )
    .map_err(DomainError::agent_reference_invalid)?;
    browser.validate_read_scope(&native_ref)?;
    let session = browser.read_agent(&record.canonical_ref)?;
    index.resolve(&record.tool, &record.opaque_ref, pin_content)?;
    browser.validate_read_scope(&native_ref)?;
    Ok(session)
}

fn message_is_rewritable(message: &Message) -> bool {
    message
        .blocks
        .iter()
        .any(|block| block.kind == BlockKind::Text)
}

/// UI 浏览路径的 locator 签发器：与 Agent 读取共用同一 `(ref, 原生定位, role)`
/// 键，保证两条路径对同一条消息拿到同一个 `fml_` 引用。
pub fn browser_locator_issuer<'a>(
    index: &'a AgentSessionIndex,
    record: &'a IndexedSession,
) -> impl Fn(&Message, usize) -> DomainResult<String> + 'a {
    move |message: &Message, message_index: usize| {
        index.issue_message_locator(
            record,
            &native_locator(message, message_index),
            &message.role,
            message_is_rewritable(message),
        )
    }
}

/// 仅剩一条消息仍超预算时继续压小它；无可再压返回 `false`。
fn shrink_sole_message(item: &mut Value, truncation: &mut Value) -> bool {
    let Some(blocks) = item.get_mut("blocks").and_then(Value::as_array_mut) else {
        return false;
    };
    // Python `max(texts, key=...)` 取**首个**最大值。
    let mut largest: Option<usize> = None;
    let mut largest_len = 0usize;
    for (position, block) in blocks.iter().enumerate() {
        if block.get("kind").and_then(Value::as_str) != Some("text") {
            continue;
        }
        let Some(text) = block.get("text").and_then(Value::as_str) else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        if largest.is_none() || text.len() > largest_len {
            largest = Some(position);
            largest_len = text.len();
        }
    }
    if let Some(position) = largest {
        let text = blocks[position]["text"].as_str().unwrap_or("").to_string();
        let mut half = text.len() / 2;
        while half > 0 && !text.is_char_boundary(half) {
            half -= 1;
        }
        let clipped = &text[..half];
        bump(
            truncation,
            "omitted_bytes",
            (text.len() - clipped.len()) as i64,
        );
        blocks[position]["text"] = Value::from(clipped);
    } else if !blocks.is_empty() {
        blocks.pop();
        bump(truncation, "omitted_blocks", 1);
    } else {
        return false;
    }
    item["complete"] = Value::Bool(false);
    truncation["truncated"] = Value::Bool(true);
    true
}

fn bump(target: &mut Value, key: &str, delta: i64) {
    let current = target.get(key).and_then(Value::as_i64).unwrap_or(0);
    target[key] = Value::from(current + delta);
}

/// 逐步收缩直到落进预算；游标语义见模块文档。
fn fit_context_result(mut result: Value, budget: usize) -> Map<String, Value> {
    while python_json_len(&result) > budget {
        let length = result["messages"].as_array().map(Vec::len).unwrap_or(0);
        if length > 1 {
            let removed = result["messages"]
                .as_array_mut()
                .expect("messages 恒为数组")
                .pop()
                .expect("length > 1");
            let next_message = removed.get("message").cloned().unwrap_or(Value::Null);
            let current_next = result
                .get("next_from_message")
                .cloned()
                .unwrap_or(Value::Null);
            result["next_from_message"] = match (current_next.as_i64(), next_message.as_i64()) {
                (Some(current), Some(next)) => Value::from(current.min(next)),
                _ => next_message,
            };
            let blocks = removed
                .get("blocks")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0);
            let truncation = &mut result["truncation"];
            bump(truncation, "omitted_blocks", blocks as i64);
            truncation["truncated"] = Value::Bool(true);
        } else if length == 1 {
            let mut sole = result["messages"][0].take();
            let mut truncation = result["truncation"].take();
            let shrunk = shrink_sole_message(&mut sole, &mut truncation);
            result["messages"][0] = sole;
            result["truncation"] = truncation;
            if shrunk {
                continue;
            }
            result["title"] = Value::from("");
            break;
        } else {
            result["title"] = Value::from("");
            break;
        }
    }
    let remaining = result["messages"].as_array().cloned().unwrap_or_default();
    result["returned_message_count"] = Value::from(remaining.len());
    result["message_range"]["to"] = remaining
        .last()
        .and_then(|item| item.get("message").cloned())
        .unwrap_or(Value::Null);
    result.as_object().cloned().unwrap_or_default()
}

/// `session_read` 的 context 档位。
///
/// `inert=true` 时按 [`super::inert`] 的常量表剥离源 agent 的脚手架：整条被剥空的
/// 消息丢弃并计入 `truncation.stripped_messages`，但**消息编号与分页游标仍按原始
/// 序号**，避免两种模式下 `--from` 的语义漂移。
#[allow(clippy::too_many_arguments)]
pub fn get_session_context(
    tool: &str,
    opaque_ref: &str,
    from_message: Option<&Value>,
    limit: Option<&Value>,
    include_tool_outputs: bool,
    max_bytes: Option<&Value>,
    inert: bool,
    index: &AgentSessionIndex,
) -> DomainResult<Map<String, Value>> {
    let record = index.resolve(tool, opaque_ref, true)?;
    let first = bounded_int(from_message, 1, 1, 1_000_000, "from_message")?;
    let count = bounded_int(limit, 20, 1, MAX_CONTEXT_MESSAGES, "limit")?;
    let budget = bounded_int(
        max_bytes,
        DEFAULT_CONTEXT_BYTES,
        1024,
        MAX_CONTEXT_BYTES,
        "max_bytes",
    )? as usize;
    let session = read_indexed_session(index, &record, true)?;
    let total_turns = session
        .messages
        .iter()
        .filter(|message| message.role == "user")
        .count() as i64;

    let mut messages: Vec<Map<String, Value>> = Vec::new();
    let mut current_turn = 0i64;
    let mut remaining = budget;
    let mut omitted_blocks = 0i64;
    let mut omitted_bytes = 0i64;
    let mut stripped_messages = 0i64;
    let mut exhausted = false;
    let selected_until = (session.messages.len() as i64).min(first - 1 + count);
    // 游标锚在「本页扫过的最后一条」而不是「返回的最后一条」：惰性模式下整页
    // 都可能被剥空，用返回值当游标会让调用方卡在同一页上。
    let mut last_scanned = first - 1;

    for (message_index, message) in session.messages.iter().enumerate() {
        if message.role == "user" {
            current_turn += 1;
        }
        let message_number = message_index as i64 + 1;
        if message_number < first || message_number > selected_until {
            continue;
        }
        last_scanned = message_number;
        let rendered: super::inert::InertBlocks<'_> = if inert {
            match super::inert::message_blocks(message) {
                Some(blocks) => blocks,
                None => {
                    stripped_messages += 1;
                    continue;
                }
            }
        } else {
            message.blocks.iter().map(|block| (block, None)).collect()
        };
        let mut blocks: Vec<Value> = Vec::new();
        let mut message_clipped = false;
        for (block, replacement) in &rendered {
            let source_text = replacement.as_deref().unwrap_or(block.text.as_str());
            let item: Option<Map<String, Value>> = match block.kind {
                BlockKind::Text => {
                    let (value, left, clipped) = take(source_text, remaining);
                    remaining = left;
                    if clipped {
                        message_clipped = true;
                        omitted_bytes += (source_text.len() - value.len()) as i64;
                    }
                    let mut entry = Map::new();
                    entry.insert("kind".into(), Value::from("text"));
                    entry.insert("text".into(), Value::from(value));
                    Some(entry)
                }
                BlockKind::Tool if block.tool.is_some() => {
                    let call = block.tool.as_ref().expect("已判定存在");
                    let (tool_input, left, input_clipped) = take_json(&call.input, remaining);
                    remaining = left;
                    let mut entry = Map::new();
                    entry.insert("kind".into(), Value::from("tool"));
                    entry.insert("name".into(), Value::from(truncate_text(&call.name, 120).0));
                    entry.insert(
                        "op".into(),
                        call.op
                            .as_deref()
                            .map(|op| Value::from(truncate_text(op, 120).0))
                            .unwrap_or(Value::Null),
                    );
                    entry.insert(
                        "status".into(),
                        call.result
                            .as_ref()
                            .map(|result| {
                                Value::from(truncate_text(status_text(result.status), 80).0)
                            })
                            .unwrap_or(Value::Null),
                    );
                    entry.insert("input".into(), tool_input);
                    entry.insert("output".into(), Value::from("[omitted]"));
                    let mut clipped = input_clipped;
                    if include_tool_outputs && remaining > 0 {
                        let output = tool_result_text(call.result.as_ref());
                        let (value, left, output_clipped) = take(&output, remaining);
                        remaining = left;
                        entry.insert("output".into(), Value::from(value));
                        clipped = clipped || output_clipped;
                    }
                    if clipped {
                        message_clipped = true;
                        omitted_blocks += 1;
                    }
                    Some(entry)
                }
                BlockKind::Image if block.image.is_some() => {
                    let image = block.image.as_ref().expect("已判定存在");
                    let mut entry = Map::new();
                    entry.insert("kind".into(), Value::from("image"));
                    entry.insert("id".into(), Value::from(truncate_text(&image.id, 200).0));
                    entry.insert(
                        "mime_type".into(),
                        Value::from(truncate_text(&image.mime_type, 120).0),
                    );
                    entry.insert(
                        "filename".into(),
                        image
                            .filename
                            .as_deref()
                            .filter(|value| !value.is_empty())
                            .map(|value| Value::from(truncate_text(value, 1024).0))
                            .unwrap_or(Value::Null),
                    );
                    entry.insert("data".into(), Value::from("[omitted]"));
                    Some(entry)
                }
                _ => {
                    omitted_blocks += 1;
                    None
                }
            };
            if let Some(entry) = item {
                blocks.push(Value::Object(entry));
            }
            if remaining == 0 {
                exhausted = true;
                break;
            }
        }
        let editable = message_is_rewritable(message);
        let mut entry = Map::new();
        entry.insert("message".into(), Value::from(message_number));
        entry.insert("turn".into(), Value::from(current_turn));
        entry.insert("role".into(), Value::from(message.role.as_str()));
        entry.insert("blocks".into(), Value::Array(blocks));
        entry.insert("editable".into(), Value::Bool(editable));
        entry.insert("complete".into(), Value::Bool(!message_clipped));
        if inert {
            entry.insert("inert".into(), Value::Bool(true));
        }
        entry.insert(
            "locator".into(),
            Value::from(index.issue_message_locator(
                &record,
                &native_locator(message, message_index),
                &message.role,
                editable,
            )?),
        );
        messages.push(entry);
        if exhausted {
            break;
        }
    }

    let last_returned = messages
        .last()
        .and_then(|item| item.get("message").and_then(Value::as_i64))
        .unwrap_or(first - 1);
    let has_more = last_scanned < session.messages.len() as i64;
    // 标题取自会话第一条可见消息，同样可能整段是脚手架：惰性模式下一并剥掉。
    let title_source = if inert {
        super::inert::strip_text(&session.title)
    } else {
        session.title.clone()
    };
    let (title, title_truncated) = truncate_text(&title_source, 200);
    let (project, project_truncated) = truncate_text(&session.cwd, 1024);

    let mut result = Map::new();
    result.insert("tool".into(), Value::from(tool));
    result.insert("ref".into(), Value::from(opaque_ref));
    result.insert(
        "session_id".into(),
        Value::from(record_session_id(&record.row, Some(&session.source_id))),
    );
    result.insert("title".into(), Value::from(title));
    result.insert("project".into(), Value::from(project));
    result.insert("title_truncated".into(), Value::Bool(title_truncated));
    result.insert("project_truncated".into(), Value::Bool(project_truncated));
    result.insert("revision".into(), Value::from(record.revision.as_str()));
    result.insert("message_count".into(), Value::from(session.messages.len()));
    result.insert("turn_count".into(), Value::from(total_turns));
    result.insert("returned_message_count".into(), Value::from(messages.len()));
    let mut range = Map::new();
    range.insert("from".into(), Value::from(first));
    range.insert(
        "to".into(),
        if messages.is_empty() {
            Value::Null
        } else {
            Value::from(last_returned)
        },
    );
    result.insert("message_range".into(), Value::Object(range));
    result.insert(
        "next_from_message".into(),
        if has_more {
            Value::from(last_scanned + 1)
        } else {
            Value::Null
        },
    );
    if inert {
        result.insert("inert".into(), Value::Bool(true));
    }
    result.insert(
        "messages".into(),
        Value::Array(messages.into_iter().map(Value::Object).collect()),
    );
    let mut truncation = Map::new();
    truncation.insert(
        "truncated".into(),
        Value::Bool(exhausted || omitted_blocks > 0),
    );
    truncation.insert("omitted_blocks".into(), Value::from(omitted_blocks));
    truncation.insert("omitted_bytes".into(), Value::from(omitted_bytes));
    if inert {
        truncation.insert("stripped_messages".into(), Value::from(stripped_messages));
    }
    truncation.insert("budget_bytes".into(), Value::from(budget));
    result.insert("truncation".into(), Value::Object(truncation));
    Ok(fit_context_result(Value::Object(result), budget))
}

fn status_text(status: crate::model::ToolResultStatus) -> &'static str {
    use crate::model::ToolResultStatus::*;
    match status {
        Success => "success",
        Error => "error",
        Interrupted => "interrupted",
        Running => "running",
        Pending => "pending",
        Unknown => "unknown",
    }
}

/// 检索用文本：调用方要求带工具输出时一并纳入范围。
///
/// `inert=true` 时文本先过一遍剥离，检索与 snippet 都不会命中源 agent 的
/// system prompt——否则 `--inert --terms` 会是个静默无效的组合。
fn searchable_text(message: &Message, include_tool_outputs: bool, inert: bool) -> String {
    let mut parts: Vec<String> = Vec::new();
    for block in &message.blocks {
        if block.kind == BlockKind::Text && !block.text.is_empty() {
            let text = if inert {
                super::inert::strip_text(&block.text)
            } else {
                block.text.clone()
            };
            if !text.is_empty() {
                parts.push(text);
            }
        } else if include_tool_outputs && block.kind == BlockKind::Tool {
            let Some(call) = block.tool.as_ref() else {
                continue;
            };
            parts.push(format!("[tool {}]", call.name));
            let output = tool_result_text(call.result.as_ref());
            if !output.is_empty() {
                parts.push(output);
            }
        }
    }
    parts.join("\n")
}

/// 以字符（不是字节）为单位取子串，对齐 Python 的切片语义。
pub fn char_slice(text: &str, start: usize, end: usize) -> String {
    text.chars()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect()
}

/// 以字符为单位查找子串位置。
pub fn char_find(haystack: &str, needle: &str) -> Option<usize> {
    haystack
        .find(needle)
        .map(|byte| haystack[..byte].chars().count())
}

/// `session_read` 的 search 档位。
#[allow(clippy::too_many_arguments)]
pub fn search_session_content(
    tool: &str,
    opaque_ref: &str,
    terms: Option<&Value>,
    roles: Option<&Value>,
    limit: Option<&Value>,
    include_tool_outputs: bool,
    inert: bool,
    index: &AgentSessionIndex,
) -> DomainResult<Map<String, Value>> {
    let record = index.resolve(tool, opaque_ref, true)?;
    let wanted = string_set(terms, "terms", 20, 100)?;
    if wanted.is_empty() {
        let mut params = Map::new();
        params.insert("field".into(), Value::from("terms"));
        return Err(DomainError::new(
            "agent.request_invalid",
            "AgentRequestError",
            "terms 至少包含一个检索词",
            params,
        ));
    }
    let allowed_roles = string_set(roles, "roles", 2, 16)?;
    if allowed_roles
        .iter()
        .any(|role| role != "user" && role != "assistant")
    {
        let mut params = Map::new();
        params.insert("field".into(), Value::from("roles"));
        return Err(DomainError::new(
            "agent.request_invalid",
            "AgentRequestError",
            "roles 仅允许 user/assistant",
            params,
        ));
    }
    let maximum = bounded_int(limit, 20, 1, MAX_CONTENT_SEARCH_RESULTS, "limit")? as usize;
    let mut sorted_terms = wanted;
    sorted_terms.sort();
    let normalized: Vec<(String, String)> = sorted_terms
        .iter()
        .map(|term| (term.clone(), super::usage::casefold(term)))
        .collect();

    let session = read_indexed_session(index, &record, true)?;
    let total_turns = session
        .messages
        .iter()
        .filter(|message| message.role == "user")
        .count() as i64;
    let mut matches: Vec<Value> = Vec::new();
    let mut current_turn = 0i64;
    let mut total_matches = 0i64;
    let mut byte_limited = false;
    let mut stripped_messages = 0i64;

    for (message_index, message) in session.messages.iter().enumerate() {
        if message.role == "user" {
            current_turn += 1;
        }
        if !allowed_roles.is_empty() && !allowed_roles.contains(&message.role) {
            continue;
        }
        if inert && super::inert::drops_role(&message.role) {
            stripped_messages += 1;
            continue;
        }
        let text = searchable_text(message, include_tool_outputs, inert);
        if inert && text.is_empty() {
            stripped_messages += 1;
            continue;
        }
        let folded = super::usage::casefold(&text);
        let hits: Vec<&(String, String)> = normalized
            .iter()
            .filter(|(_, folded_term)| folded.contains(folded_term.as_str()))
            .collect();
        if hits.is_empty() {
            continue;
        }
        total_matches += 1;
        if matches.len() >= maximum {
            continue;
        }
        let first_hit = hits
            .iter()
            .filter_map(|(_, folded_term)| char_find(&folded, folded_term))
            .min()
            .unwrap_or(0);
        let text_chars = text.chars().count();
        let start = first_hit.saturating_sub(240);
        let end = text_chars.min(first_hit + 560);
        let snippet = format!(
            "{}{}{}",
            if start > 0 { "…" } else { "" },
            char_slice(&text, start, end),
            if end < text_chars { "…" } else { "" }
        );
        let editable = message_is_rewritable(message);
        let mut item = Map::new();
        item.insert("message".into(), Value::from(message_index as i64 + 1));
        item.insert("turn".into(), Value::from(current_turn));
        item.insert("role".into(), Value::from(message.role.as_str()));
        item.insert("editable".into(), Value::Bool(editable));
        item.insert(
            "locator".into(),
            Value::from(index.issue_message_locator(
                &record,
                &native_locator(message, message_index),
                &message.role,
                editable,
            )?),
        );
        item.insert(
            "matched_terms".into(),
            Value::Array(
                hits.iter()
                    .map(|(term, _)| Value::from(term.as_str()))
                    .collect(),
            ),
        );
        item.insert(
            "snippet".into(),
            Value::from(truncate_text(&snippet, 900).0),
        );
        item.insert(
            "complete".into(),
            Value::Bool(start == 0 && end == text_chars),
        );
        if inert {
            item.insert("inert".into(), Value::Bool(true));
        }
        let item = Value::Object(item);

        let mut candidate = Map::new();
        let mut probe = matches.clone();
        probe.push(item.clone());
        candidate.insert("matches".into(), Value::Array(probe));
        candidate.insert("message_count".into(), Value::from(session.messages.len()));
        candidate.insert("turn_count".into(), Value::from(total_turns));
        candidate.insert("total_matches".into(), Value::from(total_matches));
        if python_json_len(&Value::Object(candidate)) > MAX_AGENT_DTO_BYTES - 2048 {
            byte_limited = true;
            continue;
        }
        matches.push(item);
    }

    let has_more = total_matches > matches.len() as i64;
    let mut result = Map::new();
    result.insert("tool".into(), Value::from(tool));
    result.insert("ref".into(), Value::from(opaque_ref));
    result.insert(
        "session_id".into(),
        Value::from(record_session_id(&record.row, Some(&session.source_id))),
    );
    result.insert("revision".into(), Value::from(record.revision.as_str()));
    result.insert("message_count".into(), Value::from(session.messages.len()));
    result.insert("turn_count".into(), Value::from(total_turns));
    if inert {
        result.insert("inert".into(), Value::Bool(true));
    }
    // 键序与 Python 的字面量顺序一致：DTO 字节数判定依赖它。
    let returned = matches.len();
    result.insert("matches".into(), Value::Array(matches));
    result.insert("returned".into(), Value::from(returned));
    result.insert("total_matches".into(), Value::from(total_matches));
    result.insert("has_more".into(), Value::Bool(has_more));
    result.insert(
        "searched_scope".into(),
        Value::from(if include_tool_outputs {
            "visible_text_and_tool_outputs"
        } else {
            "visible_text_only"
        }),
    );
    let mut truncation = Map::new();
    truncation.insert("truncated".into(), Value::Bool(has_more));
    truncation.insert(
        "reason".into(),
        if byte_limited {
            Value::from("byte_budget")
        } else if has_more {
            Value::from("result_limit")
        } else {
            Value::Null
        },
    );
    if inert {
        truncation.insert("stripped_messages".into(), Value::from(stripped_messages));
    }
    truncation.insert("budget_bytes".into(), Value::from(MAX_AGENT_DTO_BYTES));
    result.insert("truncation".into(), Value::Object(truncation));
    finalize_dto(result)
}

/// `session_read` 分发：给了 `terms` 走内容检索，否则走上下文分页。
#[allow(clippy::too_many_arguments)]
pub fn session_read(
    tool: &str,
    reference: Option<&str>,
    terms: Option<&Value>,
    roles: Option<&Value>,
    from_message: Option<&Value>,
    limit: Option<&Value>,
    include_tool_outputs: Option<&Value>,
    max_bytes: Option<&Value>,
    inert: Option<&Value>,
    index: &AgentSessionIndex,
) -> DomainResult<Map<String, Value>> {
    let Some(reference) = reference.filter(|value| !value.is_empty()) else {
        let mut params = Map::new();
        params.insert("field".into(), Value::from("ref"));
        return Err(DomainError::new(
            "agent.request_invalid",
            "AgentRequestError",
            "必须提供 Engine 签发的 ref",
            params,
        ));
    };
    // 显式 `null` 同样报错：分发层 `p.get("include_tool_outputs", False)` 只在
    // 缺键时给默认值，`isinstance(None, bool)` 为假（`agent_read.py:400-401`）。
    let outputs = match include_tool_outputs {
        None => false,
        Some(Value::Bool(flag)) => *flag,
        Some(_) => {
            return Err(DomainError::agent_request_invalid(
                "include_tool_outputs 必须是 boolean",
            ))
        }
    };
    // `inert` 与 `include_tool_outputs` 同口径：缺键默认 false，显式 null 报错。
    let lazy = match inert {
        None => false,
        Some(Value::Bool(flag)) => *flag,
        Some(_) => return Err(DomainError::agent_request_invalid("inert 必须是 boolean")),
    };
    let mut result = if terms.is_some_and(|value| !value.is_null()) {
        let mut payload =
            search_session_content(tool, reference, terms, roles, limit, outputs, lazy, index)?;
        payload.insert("mode".into(), Value::from("search"));
        payload
    } else {
        let mut payload = get_session_context(
            tool,
            reference,
            from_message,
            limit,
            outputs,
            max_bytes,
            lazy,
            index,
        )?;
        payload.insert("mode".into(), Value::from("context"));
        payload
    };
    // `mode` 是最后追加的键（Python 的 `result["mode"] = ...`）。
    let mode = result.remove("mode").expect("上一步刚插入");
    result.insert("mode".into(), mode);
    Ok(result)
}

/// 供调试与测试查看 DTO 字节数。
pub fn dto_bytes(value: &Value) -> usize {
    python_json(value, false).len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn take_counts_utf8_bytes_and_never_splits_a_code_point() {
        assert_eq!(take("abc", 10), ("abc".into(), 7, false));
        assert_eq!(take("abc", 2), ("ab".into(), 0, true));
        // 中文一个字 3 字节：预算 4 只能装下一个。
        assert_eq!(take("中文", 4), ("中".into(), 0, true));
        assert_eq!(take("中文", 6), ("中文".into(), 0, false));
        assert_eq!(take("", 0), (String::new(), 0, false));
    }

    #[test]
    fn take_json_degrades_to_a_marker_then_to_nothing() {
        let (value, remaining, clipped) = take_json(&json!({"a": 1}), 1024);
        assert_eq!(value, json!({"a": 1}));
        assert!(!clipped);
        assert_eq!(remaining, 1024 - dto_bytes(&json!({"a": 1})));
        // 预算 < 32 直接返回空 object。
        assert_eq!(take_json(&json!({"a": 1}), 10), (json!({}), 8, true));
        // 预算够写 marker 但装不下内容。
        let big = json!({"a": "x".repeat(500)});
        let (value, _, clipped) = take_json(&big, 40);
        assert_eq!(value, json!({"truncated": true}));
        assert!(clipped);
    }

    /// `session_read` 的分发默认值只在**缺键**时生效；键在而值为 `null`
    /// 会走到 `isinstance(None, bool)` 的假分支（`agent_read.py:400-401`）。
    #[test]
    fn session_read_rejects_a_non_boolean_include_tool_outputs() {
        let harness = crate::sessions::index::golden_tests::harness();
        let call = |flag: Option<&Value>| {
            session_read(
                "claude",
                Some("fsr_0000000000000000000000"),
                None,
                None,
                None,
                None,
                flag,
                None,
                None,
                &harness.index,
            )
        };
        for bad in [Value::Null, json!(0), json!("true")] {
            let error = call(Some(&bad)).unwrap_err();
            assert_eq!(error.message(), "include_tool_outputs 必须是 boolean");
            assert_eq!(error.code, "agent.request_invalid");
        }
        // 缺键与合法 boolean 都能穿过这道校验（后续才因未知 ref 失败）。
        for good in [None, Some(&Value::Bool(true)), Some(&Value::Bool(false))] {
            let error = call(good).unwrap_err();
            assert_ne!(error.message(), "include_tool_outputs 必须是 boolean");
        }
        // ref 校验排在 include_tool_outputs 之前。
        let error = session_read(
            "claude",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &harness.index,
        )
        .unwrap_err();
        assert_eq!(error.message(), "必须提供 Engine 签发的 ref");
    }

    #[test]
    fn truncated_marker_size_matches_python() {
        assert_eq!(
            TRUNCATED_MARKER_BYTES,
            // Python `json.dumps({"truncated": True})` 用默认分隔符：`{"truncated": true}`。
            super::python_json(&json!({"truncated": true}), false).len()
        );
    }

    #[test]
    fn fit_pops_the_tail_and_lowers_the_cursor() {
        let result = json!({
            "title": "t",
            "next_from_message": 9,
            "returned_message_count": 2,
            "message_range": {"from": 1, "to": 2},
            "messages": [
                {"message": 1, "blocks": [{"kind": "text", "text": "a"}], "complete": true},
                {"message": 2, "blocks": [{"kind": "text", "text": "x".repeat(400)}],
                 "complete": true}
            ],
            "truncation": {"truncated": false, "omitted_blocks": 0, "omitted_bytes": 0,
                           "budget_bytes": 200},
        });
        let fitted = fit_context_result(result, 200);
        assert_eq!(fitted["messages"].as_array().unwrap().len(), 1);
        // 游标下调到被弹消息的编号，而不是停在 9。
        assert_eq!(fitted["next_from_message"], Value::from(2));
        assert_eq!(fitted["returned_message_count"], Value::from(1));
        assert_eq!(fitted["message_range"]["to"], Value::from(1));
        assert_eq!(fitted["truncation"]["truncated"], Value::Bool(true));
        // 弹掉尾部消息记 1 个 block；剩下的独苗继续压缩还会再记。
        assert!(fitted["truncation"]["omitted_blocks"].as_i64().unwrap() >= 1);
    }

    #[test]
    fn fit_halves_the_largest_text_when_a_single_message_remains() {
        let result = json!({
            "title": "t",
            "next_from_message": null,
            "returned_message_count": 1,
            "message_range": {"from": 1, "to": 1},
            "messages": [
                {"message": 1, "complete": true, "blocks": [
                    {"kind": "text", "text": "y".repeat(20)},
                    {"kind": "text", "text": "x".repeat(400)}
                ]}
            ],
            "truncation": {"truncated": false, "omitted_blocks": 0, "omitted_bytes": 0,
                           "budget_bytes": 200},
        });
        let fitted = fit_context_result(result, 600);
        let blocks = fitted["messages"][0]["blocks"].as_array().unwrap();
        // 最大的那块被反复砍半，短的那块不动。
        assert_eq!(blocks[0]["text"].as_str().unwrap().len(), 20);
        assert!(blocks[1]["text"].as_str().unwrap().len() < 400);
        assert_eq!(fitted["messages"][0]["complete"], Value::Bool(false));
        assert!(fitted["truncation"]["omitted_bytes"].as_i64().unwrap() > 0);
    }

    #[test]
    fn fit_gives_up_by_clearing_the_title() {
        let result = json!({
            "title": "t".repeat(500),
            "next_from_message": null,
            "returned_message_count": 0,
            "message_range": {"from": 1, "to": null},
            "messages": [],
            "truncation": {"truncated": false, "omitted_blocks": 0, "omitted_bytes": 0,
                           "budget_bytes": 10},
        });
        let fitted = fit_context_result(result, 10);
        assert_eq!(fitted["title"], Value::from(""));
        assert_eq!(fitted["message_range"]["to"], Value::Null);
    }

    #[test]
    fn char_helpers_use_code_point_indices() {
        assert_eq!(char_slice("中文测试", 1, 3), "文测");
        assert_eq!(char_find("中文测试", "测"), Some(2));
        assert_eq!(char_find("abc", "z"), None);
    }

    #[test]
    fn searchable_text_includes_tool_output_only_on_request() {
        let mut message = Message::new("assistant");
        message.blocks.push(crate::model::Block::text("hello"));
        let mut tool_block = crate::model::Block::new(BlockKind::Tool);
        let mut call = crate::model::ToolCall::new("Bash", None, json!({}));
        call.result = Some(crate::model::text_tool_result(
            "output",
            crate::model::ToolResultStatus::Success,
        ));
        tool_block.tool = Some(call);
        message.blocks.push(tool_block);
        assert_eq!(searchable_text(&message, false, false), "hello");
        assert_eq!(
            searchable_text(&message, true, false),
            "hello\n[tool Bash]\noutput"
        );
    }
}
