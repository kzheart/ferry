//! 供 Ferry Agent 使用的限量会话读取。
//!
//! 预算口径：DTO 上限 64 KiB，默认上下文预算 24 KiB，按完整 JSON 的 UTF-8 字节计数。
//!
//! 消息级和 block 分片均使用绑定读取参数与内容快照的 next_cursor。

use serde_json::{json, Map, Value};

use crate::adapters::contracts::NativeSessionReference;
use crate::errors::{DomainError, DomainResult};
use crate::model::{native_locator, tool_result_text, BlockKind, Message, Session};

use super::index::{AgentSessionIndex, IndexedSession};
use super::safety::{
    bounded_int, python_json, python_json_len, record_session_id, string_set, truncate_text,
    MAX_AGENT_DTO_BYTES,
};

pub const MAX_CONTENT_SEARCH_RESULTS: i64 = 50;
pub const MAX_CONTEXT_MESSAGES: i64 = 50;
pub const MAX_CONTEXT_BYTES: i64 = 64 * 1024;
pub const DEFAULT_CONTEXT_BYTES: i64 = 24 * 1024;

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

use super::read_cursor::{self, Cursor, Position};

/// 返回一个完整的可见 block；大 block 由分页层按其 JSON UTF-8 字节分片。
fn visible_block(
    block: &crate::model::Block,
    replacement: Option<&str>,
    outputs: bool,
    number: usize,
) -> Option<Value> {
    let mut value = match block.kind {
        BlockKind::Text => json!({"kind": "text", "text": replacement.unwrap_or(&block.text)}),
        BlockKind::Tool => {
            let call = block.tool.as_ref()?;
            json!({"kind": "tool", "name": call.name, "op": call.op,
                "status": call.result.as_ref().map(|result| status_text(result.status)),
                "input": call.input,
                "output": if outputs { tool_result_text(call.result.as_ref()) } else { "[omitted]".into() }})
        }
        BlockKind::Image => {
            let image = block.image.as_ref()?;
            json!({"kind": "image", "id": image.id, "mime_type": image.mime_type,
                "filename": image.filename, "data": "[omitted]"})
        }
        _ => return None,
    };
    value["block"] = json!(number);
    Some(value)
}

/// 原始 block 号保持不变，即使 inert 剥掉了前面的 block。
fn visible_blocks(
    message: &Message,
    inert: bool,
    outputs: bool,
) -> Option<(Vec<Value>, usize, usize)> {
    let rendered = if inert {
        super::inert::message_blocks(message)?
    } else {
        message.blocks.iter().map(|block| (block, None)).collect()
    };
    let mut visible = Vec::new();
    let mut omitted_blocks = 0;
    let mut omitted_bytes = 0;
    for (block, replacement) in rendered {
        let number = message
            .blocks
            .iter()
            .position(|original| std::ptr::eq(original, block))?
            + 1;
        if let Some(value) = visible_block(block, replacement.as_deref(), outputs, number) {
            visible.push(value);
        } else {
            omitted_blocks += 1;
            omitted_bytes += serde_json::to_vec(block).expect("Block 可编码").len();
        }
    }
    Some((visible, omitted_blocks, omitted_bytes))
}

fn cursor_read_error(error: DomainError, continuing: bool) -> DomainError {
    if continuing && error.params().get("reason").and_then(Value::as_str) == Some("session_changed")
    {
        read_cursor::error("cursor_stale", "会话已变化，请重新读取第一页")
    } else {
        error
    }
}

fn read_record(
    index: &AgentSessionIndex,
    tool: &str,
    reference: &str,
    continuing: bool,
) -> DomainResult<IndexedSession> {
    index
        .resolve(tool, reference, true)
        .map_err(|error| cursor_read_error(error, continuing))
}

/// context 游标定位 (原消息、可见 block、block JSON 字节偏移)。分片的 text 按序
/// 拼接后解析 JSON 即恢复原 block；工具 input/output 也不会被有损摘要替代。
#[allow(clippy::too_many_arguments)]
pub fn get_session_context(
    tool: &str,
    opaque_ref: &str,
    from_message: Option<&Value>,
    limit: Option<&Value>,
    include_tool_outputs: bool,
    max_bytes: Option<&Value>,
    inert: bool,
    cursor: Option<&Value>,
    index: &AgentSessionIndex,
) -> DomainResult<Map<String, Value>> {
    let first = bounded_int(from_message, 1, 1, 1_000_000, "from_message")?;
    let count = bounded_int(limit, 20, 1, MAX_CONTEXT_MESSAGES, "limit")? as usize;
    let budget = bounded_int(
        max_bytes,
        DEFAULT_CONTEXT_BYTES,
        1024,
        MAX_CONTEXT_BYTES,
        "max_bytes",
    )? as usize;
    let supplied = Cursor::decode(cursor)?;
    let continuing = supplied.is_some();
    let record = read_record(index, tool, opaque_ref, continuing)?;
    let session = read_indexed_session(index, &record, true)
        .map_err(|error| cursor_read_error(error, continuing))?;
    let binding = read_cursor::digest(&json!([
        "context",
        tool,
        opaque_ref,
        first,
        inert,
        include_tool_outputs
    ]));
    let snapshot = read_cursor::digest(&json!([record.revision, session]));
    let start = Cursor::resume(
        supplied,
        &binding,
        &snapshot,
        Position {
            message: first as usize - 1,
            block: 0,
            offset: 0,
        },
    )?;
    if continuing && start.message >= session.messages.len() {
        return Err(read_cursor::error("cursor_invalid", "游标消息位置无效"));
    }
    let total_turns = session
        .messages
        .iter()
        .filter(|message| message.role == "user")
        .count();
    let title_source = if inert {
        super::inert::strip_text(&session.title)
    } else {
        session.title.clone()
    };
    let (title, title_truncated) = truncate_text(&title_source, 200);
    let (project, project_truncated) = truncate_text(&session.cwd, 1024);
    let base = json!({"tool": tool, "ref": opaque_ref,
        "session_id": record_session_id(&record.row, Some(&session.source_id)),
        "revision": record.revision, "title": title, "project": project,
        "title_source": record.row.get("title_source").and_then(Value::as_str).unwrap_or(""),
        "title_truncated": title_truncated, "project_truncated": project_truncated,
        "message_count": session.messages.len(), "turn_count": total_turns,
        "mode": "context"});
    context_page(
        &session,
        base,
        start,
        count,
        budget,
        inert,
        include_tool_outputs,
        &binding,
        &snapshot,
        |message, message_index| {
            index.issue_message_locator(
                &record,
                &native_locator(message, message_index),
                &message.role,
                message_is_rewritable(message),
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn context_page(
    session: &Session,
    mut base: Value,
    start: Position,
    count: usize,
    budget: usize,
    inert: bool,
    outputs: bool,
    binding: &str,
    snapshot: &str,
    locator: impl Fn(&Message, usize) -> DomainResult<String>,
) -> DomainResult<Map<String, Value>> {
    let total = session.messages.len();
    if start.message > total && (start.block > 0 || start.offset > 0) {
        return Err(read_cursor::error("cursor_invalid", "游标消息位置无效"));
    }
    let mut position = start;
    let mut messages = Vec::<Value>::new();
    let mut stripped = 0usize;
    let mut scanned = 0usize;
    let omissions = std::cell::Cell::new((0usize, 0usize));
    let mut byte_limited = false;
    let mut current_turn = session
        .messages
        .iter()
        .take(start.message)
        .filter(|message| message.role == "user")
        .count();
    if inert {
        base["inert"] = json!(true);
    }
    // 为低预算保留真正的消息载荷；元数据缩短必须明确标记。
    for field in ["title", "project"] {
        if python_json_len(&base) > budget / 3 {
            base[field] = json!("");
            base[format!("{field}_truncated")] = json!(true);
        }
    }
    let finish = |messages: &[Value],
                  next: Position,
                  stripped: usize,
                  byte_limited: bool|
     -> Value {
        let mut result = base.clone();
        let has_more = next.message < total;
        result["messages"] = json!(messages);
        result["returned_message_count"] = json!(messages.len());
        result["message_range"] = json!({"from": start.message + 1, "to": messages.last().map(|message| &message["message"])});
        result["next_cursor"] = if has_more {
            json!(Cursor::new(binding, snapshot, next).encode())
        } else {
            Value::Null
        };
        result["has_more"] = json!(has_more);
        // 只有整条读完才可使用旧的消息级跳转；分片续读必须使用 next_cursor。
        result["next_from_message"] = if has_more && next.block == 0 && next.offset == 0 {
            json!(next.message + 1)
        } else {
            Value::Null
        };
        let (omitted_blocks, omitted_bytes) = omissions.get();
        result["truncation"] = json!({"truncated": has_more || omitted_blocks > 0,
            "omitted_blocks": omitted_blocks, "omitted_bytes": omitted_bytes,
            "omission_scope": "unsupported_blocks_in_scanned_messages",
            "budget_bytes": budget, "reason": if byte_limited { Some("byte_budget") } else if has_more { Some("message_limit") } else if omitted_blocks > 0 { Some("unsupported_blocks") } else { None }});
        if inert {
            result["truncation"]["stripped_messages"] = json!(stripped);
        }
        result
    };
    'messages: while position.message < total && scanned < count {
        let message_index = position.message;
        let message = &session.messages[message_index];
        current_turn += usize::from(message.role == "user");
        scanned += 1;
        let Some((blocks, omitted_blocks, omitted_bytes)) = visible_blocks(message, inert, outputs)
        else {
            if position.block != 0 || position.offset != 0 {
                return Err(read_cursor::error("cursor_invalid", "游标指向已剥离消息"));
            }
            stripped += 1;
            position = Position {
                message: message_index + 1,
                block: 0,
                offset: 0,
            };
            continue;
        };
        let (previous_blocks, previous_bytes) = omissions.get();
        omissions.set((
            previous_blocks + omitted_blocks,
            previous_bytes + omitted_bytes,
        ));
        if (!blocks.is_empty() && position.block >= blocks.len())
            || (blocks.is_empty() && (position.block != 0 || position.offset != 0))
        {
            return Err(read_cursor::error("cursor_invalid", "游标 block 位置无效"));
        }
        let mut item = json!({"message": message_index + 1, "turn": current_turn, "role": message.role,
            "editable": message_is_rewritable(message), "locator": locator(message, message_index)?,
            "blocks": [], "complete": false,
            "origin": super::inert::message_origin(message), "duplicate_key": super::inert::duplicate_key(message)});
        if inert {
            item["inert"] = json!(true);
        }
        let mut returned_blocks = Vec::<Value>::new();
        while position.block < blocks.len() {
            let block = &blocks[position.block];
            let encoded = serde_json::to_string(block).expect("Value 可编码");
            if position.offset >= encoded.len() || !encoded.is_char_boundary(position.offset) {
                return Err(read_cursor::error("cursor_invalid", "游标字节位置无效"));
            }
            let next_block = if position.block + 1 == blocks.len() {
                Position {
                    message: message_index + 1,
                    block: 0,
                    offset: 0,
                }
            } else {
                Position {
                    block: position.block + 1,
                    offset: 0,
                    ..position
                }
            };
            let candidate = |value: Value, next: Position| {
                let mut item = item.clone();
                let mut page = messages.clone();
                let mut trial = returned_blocks.clone();
                trial.push(value);
                item["blocks"] = json!(trial);
                item["complete"] = json!(next.message > message_index);
                page.push(item);
                finish(&page, next, stripped, next.offset > 0)
            };
            if position.offset == 0
                && python_json_len(&candidate(block.clone(), next_block)) <= budget
            {
                returned_blocks.push(block.clone());
                position = next_block;
                if position.message > message_index {
                    break;
                }
                continue;
            }
            // 按完整 DTO 的实际编码大小二分，而非将正文长度误当成输出预算。
            let fragment = |end: usize| {
                json!({"kind": "fragment", "block": block["block"],
                "fragment": {"encoding": "json", "offset_bytes": position.offset,
                    "total_bytes": encoded.len(), "text": &encoded[position.offset..end], "complete": end == encoded.len()}})
            };
            let mut low = position.offset;
            let mut high = encoded.len();
            while low < high {
                let mut end = low + (high - low).div_ceil(2);
                while end > low && !encoded.is_char_boundary(end) {
                    end -= 1;
                }
                if end == low {
                    end = encoded[low..]
                        .char_indices()
                        .nth(1)
                        .map(|(n, _)| low + n)
                        .unwrap_or(encoded.len());
                    if end > high {
                        break;
                    }
                }
                let next = if end == encoded.len() {
                    next_block
                } else {
                    Position {
                        offset: end,
                        ..position
                    }
                };
                if python_json_len(&candidate(fragment(end), next)) <= budget {
                    low = end;
                } else {
                    high = end - 1;
                }
            }
            if low == position.offset {
                byte_limited = true;
                if !returned_blocks.is_empty() {
                    item["blocks"] = json!(returned_blocks);
                    messages.push(item);
                }
                break 'messages;
            }
            returned_blocks.push(fragment(low));
            position = if low == encoded.len() {
                next_block
            } else {
                Position {
                    offset: low,
                    ..position
                }
            };
            if position.offset > 0 {
                byte_limited = true;
                break;
            }
            if position.message > message_index {
                break;
            }
        }
        if blocks.is_empty() {
            position = Position {
                message: message_index + 1,
                block: 0,
                offset: 0,
            };
        }
        item["blocks"] = json!(returned_blocks);
        item["complete"] = json!(position.message > message_index);
        let mut trial = messages.clone();
        trial.push(item);
        if python_json_len(&finish(&trial, position, stripped, byte_limited)) > budget {
            position = Position {
                message: message_index,
                block: 0,
                offset: 0,
            };
            byte_limited = true;
            break;
        }
        messages = trial;
        if byte_limited {
            break;
        }
    }
    if byte_limited && messages.is_empty() && position == start {
        return Err(read_cursor::error(
            "byte_budget_too_small",
            "预算无法容纳消息及续页信息，请增大 max_bytes",
        ));
    }
    let result = finish(&messages, position, stripped, byte_limited);
    if python_json_len(&result) > budget {
        return Err(read_cursor::error(
            "byte_budget_too_small",
            "预算无法容纳元数据，请增大 max_bytes",
        ));
    }
    Ok(result.as_object().expect("object").clone())
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
    max_bytes: Option<&Value>,
    cursor: Option<&Value>,
    index: &AgentSessionIndex,
) -> DomainResult<Map<String, Value>> {
    let supplied = Cursor::decode(cursor)?;
    let record = read_record(index, tool, opaque_ref, supplied.is_some())?;
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
    let mut allowed_roles = string_set(roles, "roles", 2, 16)?;
    allowed_roles.sort();
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

    let session = read_indexed_session(index, &record, true)
        .map_err(|error| cursor_read_error(error, supplied.is_some()))?;
    let budget = bounded_int(
        max_bytes,
        MAX_AGENT_DTO_BYTES as i64,
        1024,
        MAX_CONTEXT_BYTES,
        "max_bytes",
    )? as usize;
    let binding = read_cursor::digest(&json!([
        "search",
        tool,
        opaque_ref,
        sorted_terms,
        allowed_roles,
        inert,
        include_tool_outputs
    ]));
    let snapshot = read_cursor::digest(&json!([record.revision, session]));
    let start = Cursor::resume(supplied, &binding, &snapshot, Position::default())?;
    if start.block != 0 || start.offset != 0 || start.message > session.messages.len() {
        return Err(read_cursor::error("cursor_invalid", "搜索游标位置无效"));
    }
    let total_turns = session
        .messages
        .iter()
        .filter(|message| message.role == "user")
        .count() as i64;
    let mut matches: Vec<Value> = Vec::new();
    let mut current_turn = 0i64;
    let mut total_matches = 0i64;
    let mut stripped_messages = 0i64;
    let mut next_message = None;

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
        if message_index < start.message {
            continue;
        }
        if matches.len() >= maximum {
            next_message.get_or_insert(message_index);
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
        item.insert(
            "origin".into(),
            json!(super::inert::message_origin(message)),
        );
        item.insert(
            "duplicate_key".into(),
            json!(super::inert::duplicate_key(message)),
        );
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

        matches.push(item);
    }

    let has_more = next_message.is_some();

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
        "next_cursor".into(),
        next_message
            .map(|message| {
                json!(Cursor::new(
                    &binding,
                    &snapshot,
                    Position {
                        message,
                        block: 0,
                        offset: 0
                    }
                )
                .encode())
            })
            .unwrap_or(Value::Null),
    );
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
        if has_more {
            Value::from("result_limit")
        } else {
            Value::Null
        },
    );
    if inert {
        truncation.insert("stripped_messages".into(), Value::from(stripped_messages));
    }
    truncation.insert("budget_bytes".into(), Value::from(budget));
    result.insert("truncation".into(), Value::Object(truncation));
    result.insert("mode".into(), json!("search"));
    while python_json_len(&Value::Object(result.clone())) > budget {
        let items = result
            .get_mut("matches")
            .and_then(Value::as_array_mut)
            .expect("matches array");
        if items.len() > 1 {
            let removed = items.pop().expect("nonempty");
            let next = removed["message"].as_u64().expect("message number") as usize - 1;
            result.insert(
                "next_cursor".into(),
                json!(Cursor::new(
                    &binding,
                    &snapshot,
                    Position {
                        message: next,
                        block: 0,
                        offset: 0
                    }
                )
                .encode()),
            );
            result.insert("has_more".into(), json!(true));
        } else if let Some(item) = items.first_mut() {
            let snippet = item["snippet"].as_str().expect("snippet");
            if snippet.is_empty() {
                return Err(read_cursor::error(
                    "byte_budget_too_small",
                    "预算无法容纳搜索命中，请增大 max_bytes",
                ));
            }
            let mut boundary = snippet.len() / 2;
            while !snippet.is_char_boundary(boundary) {
                boundary -= 1;
            }
            item["snippet"] = json!(&snippet[..boundary]);
            item["complete"] = json!(false);
        } else {
            return Err(read_cursor::error(
                "byte_budget_too_small",
                "预算无法容纳搜索元数据，请增大 max_bytes",
            ));
        }
        result["returned"] = json!(result["matches"].as_array().expect("array").len());
        result["truncation"]["truncated"] = json!(true);
        result["truncation"]["reason"] = json!("byte_budget");
    }
    Ok(result)
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
    cursor: Option<&Value>,
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
        let mut payload = search_session_content(
            tool, reference, terms, roles, limit, outputs, lazy, max_bytes, cursor, index,
        )?;
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
            cursor,
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
            None,
            &harness.index,
        )
        .unwrap_err();
        assert_eq!(error.message(), "必须提供 Engine 签发的 ref");
    }

    #[test]
    fn canonical_thinking_omissions_are_reported_without_hiding_visible_bold_text() {
        let mut session = Session::new("fixture", "thinking", "/fixture");
        let mut message = Message::new("assistant");
        let mut thinking = crate::model::Block::new(BlockKind::Thinking);
        thinking.text = "fixture reasoning".into();
        message.blocks.push(thinking);
        message
            .blocks
            .push(crate::model::Block::text("**用户可见结论**"));
        session.messages.push(message);
        for inert in [false, true] {
            let result = context_page(
                &session,
                json!({"mode": "context"}),
                Position::default(),
                20,
                24576,
                inert,
                false,
                "fixture",
                "snapshot",
                |_, _| Ok("fml_fixture".into()),
            )
            .unwrap();
            assert_eq!(result["truncation"]["omitted_blocks"], 1);
            assert!(result["truncation"]["omitted_bytes"].as_u64().unwrap() > 0);
            assert_eq!(
                result["truncation"]["omission_scope"],
                "unsupported_blocks_in_scanned_messages"
            );
            assert_eq!(result["truncation"]["truncated"], true);
            assert_eq!(result["has_more"], false);
            let blocks = result["messages"][0]["blocks"].as_array().unwrap();
            assert_eq!(blocks.len(), 1);
            assert_eq!(blocks[0]["text"], "**用户可见结论**");
            assert_eq!(blocks[0]["block"], 2);
        }
    }

    #[test]
    fn fully_stripped_pages_still_advance_the_original_message_cursor() {
        let mut session = Session::new("fixture", "stripped", "/fixture");
        for text in ["<INSTRUCTIONS>scaffold</INSTRUCTIONS>", "actual request"] {
            let mut message = Message::new("user");
            message.blocks.push(crate::model::Block::text(text));
            session.messages.push(message);
        }
        let binding = read_cursor::digest(&json!("query"));
        let snapshot = read_cursor::digest(&json!("snapshot"));
        let first = context_page(
            &session,
            json!({"mode": "context"}),
            Position::default(),
            1,
            24576,
            true,
            false,
            &binding,
            &snapshot,
            |_, _| Ok("fml_fixture".into()),
        )
        .unwrap();
        assert_eq!(first["messages"], json!([]));
        assert_eq!(first["next_from_message"], 2);
        assert_eq!(first["truncation"]["stripped_messages"], 1);
        let cursor = Cursor::decode(first.get("next_cursor")).unwrap();
        let position = Cursor::resume(cursor, &binding, &snapshot, Position::default()).unwrap();
        let second = context_page(
            &session,
            json!({"mode": "context"}),
            position,
            1,
            24576,
            true,
            false,
            &binding,
            &snapshot,
            |_, _| Ok("fml_fixture".into()),
        )
        .unwrap();
        assert_eq!(second["messages"][0]["message"], 2);
        assert_eq!(second["messages"][0]["blocks"][0]["text"], "actual request");
        assert_eq!(second["next_cursor"], Value::Null);
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
