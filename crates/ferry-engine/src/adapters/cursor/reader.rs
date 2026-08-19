//! 把 Cursor 的 bubble 链读成 canonical session。
//!
//! 两条硬纪律，违反任何一条都会产出垃圾数据：
//!
//! 1. **绝不用 `bubbleId:<composerId>:%` 前缀扫描重建会话**。本机 153 789 行
//!    bubble 里有 132 648 行（86%）是没人引用的僵尸——会话被压缩或回滚后
//!    Cursor 重建了新链，旧行不删。唯一权威的存活清单是
//!    `composerData.fullConversationHeadersOnly`，必须逐条点查。
//! 2. **顺序只认数组下标**。`bubble.createdAt` 在 11% 的多消息会话里非单调
//!    （并发工具调用 + 批量重写），不能拿来排序。
//!
//! `conversationState` / `agentKv:blob:` 是「发给模型的 prompt 消息数组」的并行
//! 一层，不是 bubble 的替代（本机 113 个有 blob 链的会话 100% 同时有 bubble
//! 链）。要完整会话内容取 bubble 就够，所以这里不解析它，也就不需要 protobuf。

use rusqlite::Connection;
use serde_json::{Map, Value};

use crate::errors::{DomainError, DomainResult};
use crate::model::{
    AgentEdge, Block, BlockKind, ContextCompaction, Message, Session, Timestamp, ToolCall,
    ToolResult, ToolResultBlock, ToolResultBlockKind, ToolResultStatus,
};
use crate::tool_ops::CanonicalOp;

use super::dialect::DIALECT;
use super::native_schema::{
    embedded_json, Bubble, ComposerData, Head, ToolFormerData, CAPABILITY_COMPACTION,
    CAPABILITY_THINKING, CAPABILITY_TOOL,
};
use super::store;

/// 会话头：header 行的 `value` + `composerData`。
struct Native {
    head: Head,
    data: ComposerData,
}

fn head_value(connection: &Connection, session_id: &str) -> rusqlite::Result<Option<String>> {
    let mut statement =
        connection.prepare("SELECT value FROM composerHeaders WHERE composerId = ?")?;
    let mut rows = statement.query([session_id])?;
    match rows.next()? {
        None => Ok(None),
        Some(row) => Ok(Some(store::text_cell(row.get_ref(0)?))),
    }
}

fn load(connection: &Connection, session_id: &str) -> DomainResult<Native> {
    let head_raw = head_value(connection, session_id).map_err(|error| {
        DomainError::session_store_unavailable("cursor", &format!("读取会话头失败: {error}"))
    })?;
    let data_raw = store::disk_kv(connection, &format!("composerData:{session_id}"))
        .map_err(|error| {
            DomainError::session_store_unavailable("cursor", &format!("读取会话体失败: {error}"))
        })?
        .filter(|text| !text.is_empty());
    if head_raw.is_none() && data_raw.is_none() {
        return Err(DomainError::session_not_found("cursor", session_id));
    }
    Ok(Native {
        head: head_raw
            .as_deref()
            .and_then(|text| serde_json::from_str(text).ok())
            .unwrap_or_default(),
        // header 与 composerData 各有约 10 个单边存在的会话；缺内容时降级为仅元数据。
        data: data_raw
            .as_deref()
            .and_then(|text| serde_json::from_str(text).ok())
            .unwrap_or_default(),
    })
}

fn workspace_path(native: &Native) -> String {
    let from = |identifier: Option<&super::native_schema::WorkspaceIdentifier>| {
        identifier?.uri.as_ref()?.local_path().map(str::to_string)
    };
    from(native.head.workspace_identifier.as_ref())
        .or_else(|| from(native.data.workspace_identifier.as_ref()))
        .unwrap_or_default()
}

/// `completed | error | cancelled | loading` 之外的取值当未知。
///
/// `loading` 不是失败：它表示会话被中断、这次调用从未收敛，`result` 因此缺席，
/// 渲染上必须与 `error` 区别开。
fn tool_status(status: &str) -> ToolResultStatus {
    match status {
        "completed" => ToolResultStatus::Success,
        "error" => ToolResultStatus::Error,
        "cancelled" => ToolResultStatus::Interrupted,
        "loading" => ToolResultStatus::Running,
        _ => ToolResultStatus::Unknown,
    }
}

/// 结果里承载正文的键：命中就产出 text 块，否则整体作为 json 块。
const RESULT_TEXT_KEYS: &[&str] = &[
    "contents", "output", "markdown", "text", "content", "stdout",
];

/// 不承载内容的结果键。
///
/// Cursor 的多数结果里除正文外还有指针与渲染提示：`read_file_v2` 有 73% 只留下
/// `totalLinesInFile`、`edit_file_v2` 只留下指向 `composer.content.<sha256>` 的
/// 两个 id、`task_v2` 只留下子会话 id。把它们投影成工具输出是纯噪声——补丁全文
/// 已在入参的 `raw_patch` 里，子会话 id 已在 agent 边上。全是这类键就不产出块。
const RESULT_METADATA_KEYS: &[&str] = &[
    "totalLinesInFile",
    "beforeContentId",
    "afterContentId",
    "agentId",
    "isBackground",
    "isFinal",
    "rejected",
    "notInterrupted",
    "exitCode",
];

fn result_blocks(value: &Value) -> Vec<ToolResultBlock> {
    match value {
        Value::Null => Vec::new(),
        Value::String(text) if text.is_empty() => Vec::new(),
        Value::String(text) => vec![ToolResultBlock::text(text.as_str())],
        Value::Object(entries) => {
            for key in RESULT_TEXT_KEYS {
                if let Some(Value::String(text)) = entries.get(*key) {
                    return if text.is_empty() {
                        Vec::new()
                    } else {
                        vec![ToolResultBlock::text(text.as_str())]
                    };
                }
            }
            let payload: Map<String, Value> = entries
                .iter()
                .filter(|(key, _)| !RESULT_METADATA_KEYS.contains(&key.as_str()))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            if payload.is_empty() {
                return Vec::new();
            }
            let mut block = ToolResultBlock::new(ToolResultBlockKind::Json);
            block.data = Value::Object(payload);
            vec![block]
        }
        other => {
            let mut block = ToolResultBlock::new(ToolResultBlockKind::Json);
            block.data = other.clone();
            vec![block]
        }
    }
}

/// 原生入参 → `(规范操作, 规范入参)`；方言不认就退回 `tool.invoke`。
fn normalize(name: &str, raw: &Value) -> (Option<String>, Value) {
    match DIALECT.parse(name, raw) {
        Some((op, canonical)) => (Some(op.to_string()), canonical),
        None => {
            let mut fallback = Map::new();
            fallback.insert("namespace".into(), Value::from("cursor"));
            fallback.insert("name".into(), Value::from(name));
            fallback.insert("input".into(), raw.clone());
            (
                Some(CanonicalOp::TOOL_INVOKE.to_string()),
                Value::Object(fallback),
            )
        }
    }
}

fn tool_call(data: &ToolFormerData) -> ToolCall {
    // params 是规范化后的参数，rawArgs 是模型原样输出；前者更完整。
    let raw = embedded_json(data.params.as_deref())
        .or_else(|| embedded_json(data.raw_args.as_deref()))
        .unwrap_or_else(|| Value::Object(Map::new()));
    let (op, input) = normalize(&data.name, &raw);
    let mut call = ToolCall::new(data.name.as_str(), op, input);
    call.source_call_id = data
        .tool_call_id
        .clone()
        .filter(|value| !value.is_empty())
        .or_else(|| Some(data.name.clone()));

    // 结果与调用同住一条 bubble，不存在配对失败；status=loading 时 result 缺席，
    // 用一个空的 running 结果如实表达「会话被中断、调用从未收敛」。
    let payload = embedded_json(data.result.as_deref());
    let mut result = ToolResult::new(tool_status(&data.status));
    result.blocks = payload.as_ref().map(result_blocks).unwrap_or_default();
    if let Some(error) = data.error.as_deref().filter(|text| !text.is_empty()) {
        result.stderr = Some(error.to_string());
        result.status = ToolResultStatus::Error;
    }
    // result 里带 error 信封的（如 ripgrep 的 Path does not exist）也是失败。
    if payload
        .as_ref()
        .and_then(|value| value.get("error"))
        .is_some_and(|value| !value.is_null())
    {
        result.status = ToolResultStatus::Error;
    }
    // 终端结果只在非零时写 exitCode；非零退出把 completed 纠成 error。
    result.exit_code = payload
        .as_ref()
        .and_then(|value| value.get("exitCode"))
        .filter(|value| !value.is_boolean())
        .and_then(Value::as_i64);
    if result.status == ToolResultStatus::Success && result.exit_code.is_some_and(|code| code != 0)
    {
        result.status = ToolResultStatus::Error;
    }
    call.result = Some(result);
    if let Some(started) = data
        .additional_data
        .get("startedAtMs")
        .and_then(Value::as_i64)
    {
        call.started_at = Some(Timestamp::Millis(started));
    }
    call.agent_id = subagent_id(data);
    call
}

/// task_v2 的子会话 composerId（三处冗余落位，任取其一）。
fn subagent_id(data: &ToolFormerData) -> Option<String> {
    let from_additional = data
        .additional_data
        .get("subagentComposerId")
        .and_then(Value::as_str)
        .map(str::to_string);
    from_additional.or_else(|| {
        embedded_json(data.result.as_deref())?
            .get("agentId")?
            .as_str()
            .map(str::to_string)
    })
}

/// `errorDetails` 是 Cursor UI 里真会显示的报错横幅，投影成文本而不是丢掉。
fn error_text(details: &Value) -> Option<String> {
    let inner = details.get("error")?.get("details")?;
    let field = |key: &str| inner.get(key).and_then(Value::as_str).unwrap_or_default();
    let text = [field("title"), field("detail")]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(": ");
    Some(text).filter(|text| !text.is_empty())
}

/// 读一个会话成 canonical session（不装配子会话树）。
pub fn read(session_id: &str) -> DomainResult<Session> {
    let connection = store::open_database()?;
    let native = load(&connection, session_id)?;

    let mut session = Session::new("cursor", session_id, workspace_path(&native));
    session.title = native
        .head
        .name
        .clone()
        .or_else(|| native.data.name.clone())
        .or_else(|| native.head.subtitle.clone())
        .unwrap_or_default();
    session.model = native.data.model().map(str::to_string);
    if let Some(info) = native.head.subagent_info.as_ref() {
        session.parent_id = info
            .parent_composer_id
            .clone()
            .filter(|value| !value.is_empty());
        session.agent_type = info
            .subagent_type_name
            .clone()
            .filter(|value| !value.is_empty());
    }

    let mut statement = connection
        .prepare("SELECT value FROM cursorDiskKV WHERE key = ?")
        .map_err(|error| {
            DomainError::session_store_unavailable("cursor", &format!("读取消息失败: {error}"))
        })?;
    let mut malformed = 0i64;
    let mut dangling = 0i64;
    for (index, header) in native.data.headers.iter().enumerate() {
        if header.bubble_id.is_empty() {
            malformed += 1;
            continue;
        }
        let key = format!("bubbleId:{session_id}:{}", header.bubble_id);
        let raw = match statement.query([&key]) {
            Ok(mut rows) => match rows.next() {
                Ok(Some(row)) => row.get_ref(0).ok().map(store::text_cell),
                _ => None,
            },
            Err(_) => None,
        };
        let Some(raw) = raw else {
            // 本机 21 141 条引用 0 缺失，但 GC 随时可能改变这一点。
            dangling += 1;
            continue;
        };
        let Ok(bubble) = serde_json::from_str::<Bubble>(&raw) else {
            malformed += 1;
            continue;
        };
        append(&mut session, &bubble, &header.bubble_id, index);
    }

    for (reason, count) in [("dangling_bubble", dangling), ("invalid_bubble", malformed)] {
        if count == 0 {
            continue;
        }
        let mut params = Map::new();
        params.insert("source".into(), Value::from("cursor"));
        params.insert("reason".into(), Value::from(reason));
        params.insert("count".into(), Value::from(count));
        session.lose("session.malformed_record", params);
    }
    Ok(session)
}

/// 把一条 bubble 追加成消息。
///
/// user bubble 恒开新一轮（即使正文为空，它仍是 Cursor UI 里的一次提问）；
/// assistant bubble 一条产出一个消息块，没有内容就不产出消息。
fn append(session: &mut Session, bubble: &Bubble, bubble_id: &str, index: usize) {
    let created = bubble
        .created_at
        .clone()
        .filter(|value| !value.is_empty())
        .map(Timestamp::Text);
    // 顺序只认数组下标：locator 用下标而不是 createdAt。
    let source_id = format!("{index}:{bubble_id}");

    if bubble.kind == 1 {
        let mut message = Message::new("user");
        message.blocks.push(Block::text(bubble.text.clone()));
        message.source_id = Some(source_id);
        message.created_at = created;
        session.messages.push(message);
        return;
    }
    if bubble.kind != 2 {
        return;
    }

    let mut blocks: Vec<Block> = Vec::new();
    match bubble.capability_type {
        Some(CAPABILITY_TOOL) => {
            if let Some(data) = bubble.tool_former_data.as_ref() {
                let call = tool_call(data);
                if let Some(child) = call.agent_id.clone() {
                    let mut edge = AgentEdge::new(session.source_id.clone(), child);
                    edge.source_call_id = call.source_call_id.clone();
                    edge.spawn_message_id = Some(source_id.clone());
                    edge.agent_type = call
                        .input
                        .get("subagent_type")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    edge.prompt = call
                        .input
                        .get("prompt")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    session.agent_edges.push(edge);
                }
                let mut block = Block::new(BlockKind::Tool);
                block.tool = Some(call);
                blocks.push(block);
            }
        }
        Some(CAPABILITY_THINKING) => {
            if let Some(text) = bubble
                .thinking
                .as_ref()
                .map(super::native_schema::Thinking::text)
            {
                if !text.is_empty() {
                    let mut block = Block::new(BlockKind::Thinking);
                    block.text = text.to_string();
                    blocks.push(block);
                }
            }
        }
        // 上下文压缩标记：此点之前的内容已被摘要，不再进入模型 prompt。
        // Cursor 不落摘要正文，只有一句 "Chat context summarized."。
        Some(CAPABILITY_COMPACTION) => {
            let mut record = ContextCompaction::new(source_id.clone(), "cursor");
            record.after_message_id = session
                .messages
                .last()
                .and_then(|message| message.source_id.clone());
            record.created_at = created.clone();
            session.context_compactions.push(record);
        }
        _ if !bubble.text.is_empty() => blocks.push(Block::text(bubble.text.clone())),
        _ => {}
    }
    if let Some(text) = bubble.error_details.as_ref().and_then(error_text) {
        blocks.push(Block::text(text));
    }
    if blocks.is_empty() {
        return;
    }
    let mut message = Message::new("assistant");
    message.blocks = blocks;
    message.source_id = Some(source_id);
    message.created_at = created;
    session.messages.push(message);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::cursor::store::tests::{exclusive, materialize};
    use serde_json::json;

    fn tool_bubble(name: &str, params: Value, result: Value, status: &str) -> Value {
        json!({"_v": 3, "type": 2, "capabilityType": 15,
               "toolFormerData": {"tool": 40, "name": name, "status": status,
                                  "toolCallId": "call_00_x",
                                  "params": params.to_string(),
                                  "result": result.to_string(),
                                  "additionalData": {"startedAtMs": 1_787_000_001_000i64}}})
    }

    /// 覆盖 _v16/_v17、缺 workspaceIdentifier、子代理、归档、工具、thinking 两形态、
    /// 压缩标记与僵尸 bubble。
    fn fixture() -> Value {
        json!({"sessions": [
            {
                "id": "s-main",
                "header": {"name": "Explore", "createdAt": 1_787_000_000_000i64,
                           "lastUpdatedAt": 1_787_000_009_000i64,
                           "workspaceIdentifier": {"id": "3d6aae0c", "uri": {
                               "$mid": 1, "scheme": "file", "fsPath": "/w",
                               "path": "/w", "external": "file:///w"}}},
                "composerData": {"_v": 17, "modelConfig": {"modelName": "grok-4.6"},
                    "fullConversationHeadersOnly": [
                        {"bubbleId": "b1", "type": 1},
                        {"bubbleId": "b2", "type": 2, "grouping": {"capabilityType": 30}},
                        {"bubbleId": "b3", "type": 2, "grouping": {"capabilityType": 30}},
                        {"bubbleId": "b4", "type": 2, "grouping": {"capabilityType": 15}},
                        {"bubbleId": "b5", "type": 2, "grouping": {"capabilityType": 15}},
                        {"bubbleId": "b6", "type": 2, "grouping": {"capabilityType": 22}},
                        {"bubbleId": "b7", "type": 2},
                        {"bubbleId": "b8", "type": 2},
                        {"bubbleId": "gone", "type": 2}]},
                "bubbles": {
                    "b1": {"_v": 3, "type": 1, "text": "看看 README",
                           "createdAt": "2026-05-28T11:22:22.424Z",
                           "tokenCount": {"inputTokens": 0, "outputTokens": 0}},
                    "b2": {"_v": 3, "type": 2, "capabilityType": 30,
                           "thinking": {"text": "结构化推理", "signature": "sig"}},
                    "b3": {"_v": 3, "type": 2, "capabilityType": 30, "thinking": "裸串推理"},
                    "b4": tool_bubble("read_file_v2", json!({"targetFile": "/w/README.md"}),
                                      json!({"contents": "# hi", "totalLinesInFile": 1}),
                                      "completed"),
                    "b5": tool_bubble("task_v2",
                                      json!({"description": "explore", "prompt": "look",
                                             "subagentType": "explore"}),
                                      json!({"agentId": "s-sub"}), "completed"),
                    "b6": {"_v": 3, "type": 2, "capabilityType": 22,
                           "text": "Chat context summarized."},
                    "b7": {"_v": 3, "type": 2, "text": "读完了"},
                    "b8": {"_v": 3, "type": 2, "capabilityType": 15,
                           "toolFormerData": {"name": "ripgrep_raw_search", "status": "loading",
                                              "toolCallId": "call_00_y",
                                              "params": "{\"pattern\": \"todo\"}"}},
                    // 僵尸：不在 fullConversationHeadersOnly 里，绝不能被读出来。
                    "zombie": {"_v": 3, "type": 2, "text": "旧链残留"}},
            },
            {
                "id": "s-sub",
                "subagent": true,
                "header": {"name": "explore", "createdAt": 1_787_000_005_000i64,
                           "isArchived": true,
                           "subagentInfo": {"parentComposerId": "s-main",
                                            "subagentTypeName": "explore",
                                            "toolCallId": "call_00_x"}},
                "composerData": {"_v": 16,
                    "fullConversationHeadersOnly": [{"bubbleId": "c1", "type": 2}]},
                "bubbles": {"c1": {"_v": 3, "type": 2,
                                   "errorDetails": {"error": {"error": 57, "details": {
                                       "title": "LLM provider error",
                                       "detail": "400 level not supported"}}}}},
            },
        ]})
    }

    fn with_fixture() -> (tempfile::TempDir, std::sync::MutexGuard<'static, ()>) {
        let guard = exclusive();
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("state.vscdb");
        materialize(&database, &fixture());
        store::set_database_path_override(Some(database));
        (root, guard)
    }

    #[test]
    fn turns_tools_thinking_and_compaction_all_land_in_the_canonical_session() {
        let (_root, _guard) = with_fixture();
        let session = read("s-main").unwrap();
        store::set_database_path_override(None);

        assert_eq!(session.source_tool, "cursor");
        assert_eq!(session.cwd, "/w");
        assert_eq!(session.title, "Explore");
        assert_eq!(session.model.as_deref(), Some("grok-4.6"));
        // 引用了一条已被 GC 的 bubble：记一条损耗，其余消息照读。
        assert_eq!(session.loss.len(), 1);
        assert_eq!(session.loss[0].params["reason"], json!("dangling_bubble"));

        let roles: Vec<&str> = session
            .messages
            .iter()
            .map(|message| message.role.as_str())
            .collect();
        // b6（压缩标记）不产消息；僵尸 bubble 也不该出现。
        assert_eq!(
            roles,
            [
                "user",
                "assistant",
                "assistant",
                "assistant",
                "assistant",
                "assistant",
                "assistant"
            ]
        );
        assert!(!session
            .messages
            .iter()
            .flat_map(|message| &message.blocks)
            .any(|block| block.text.contains("旧链残留")));

        assert_eq!(session.messages[0].blocks[0].text, "看看 README");
        assert_eq!(
            session.messages[0].created_at,
            Some(Timestamp::Text("2026-05-28T11:22:22.424Z".into()))
        );
        // thinking 的两种落位都收成 Thinking 块。
        assert_eq!(session.messages[1].blocks[0].kind, BlockKind::Thinking);
        assert_eq!(session.messages[1].blocks[0].text, "结构化推理");
        assert_eq!(session.messages[2].blocks[0].text, "裸串推理");

        let read_call = session.messages[3].blocks[0].tool.as_ref().unwrap();
        assert_eq!(read_call.op.as_deref(), Some(CanonicalOp::FS_READ));
        assert_eq!(read_call.input, json!({"file_path": "/w/README.md"}));
        let result = read_call.result.as_ref().unwrap();
        assert_eq!(result.status, ToolResultStatus::Success);
        assert_eq!(result.blocks[0].text, "# hi");
        assert_eq!(
            read_call.started_at,
            Some(Timestamp::Millis(1_787_000_001_000))
        );

        // task_v2 同时产出 agent.spawn 与父子边。
        let spawn = session.messages[4].blocks[0].tool.as_ref().unwrap();
        assert_eq!(spawn.op.as_deref(), Some(CanonicalOp::AGENT_SPAWN));
        assert_eq!(spawn.agent_id.as_deref(), Some("s-sub"));
        let edge = &session.agent_edges[0];
        assert_eq!(edge.parent_session_id, "s-main");
        assert_eq!(edge.child_session_id, "s-sub");
        assert_eq!(edge.agent_type.as_deref(), Some("explore"));
        assert_eq!(edge.prompt, "look");

        let compaction = &session.context_compactions[0];
        assert_eq!(compaction.source, "cursor");
        assert_eq!(compaction.summary_status, "missing");
        assert_eq!(
            compaction.after_message_id,
            session.messages[4].source_id.clone()
        );

        assert_eq!(session.messages[5].blocks[0].text, "读完了");
        // status=loading 表示会话被中断，调用从未收敛：结果是 running 且无内容。
        let pending = session.messages[6].blocks[0].tool.as_ref().unwrap();
        assert_eq!(
            pending.result.as_ref().unwrap().status,
            ToolResultStatus::Running
        );
        assert!(pending.result.as_ref().unwrap().blocks.is_empty());
    }

    #[test]
    fn a_subagent_session_carries_its_parent_and_renders_error_banners() {
        let (_root, _guard) = with_fixture();
        let session = read("s-sub").unwrap();
        store::set_database_path_override(None);

        assert_eq!(session.parent_id.as_deref(), Some("s-main"));
        assert_eq!(session.agent_type.as_deref(), Some("explore"));
        assert_eq!(session.cwd, "");
        assert_eq!(
            session.messages[0].blocks[0].text,
            "LLM provider error: 400 level not supported"
        );
    }

    #[test]
    fn unknown_tools_and_broken_payloads_degrade_instead_of_failing() {
        let (root, _guard) = with_fixture();
        let database = root.path().join("other.vscdb");
        materialize(
            &database,
            &json!({"sessions": [{"id": "s-x", "header": {},
                "composerData": {"_v": 16, "fullConversationHeadersOnly": [
                    {"bubbleId": "m1"}, {"bubbleId": "m2"}, {"bubbleId": "absent"}]},
                "bubbles": {
                    "m1": {"_v": 3, "type": 2, "capabilityType": 15,
                           "toolFormerData": {"name": "mcp-ida-pro-decompile", "status": "error",
                                              "toolCallId": "c1",
                                              "params": "{\"ea\": 4096}",
                                              "result": "{\"error\": \"nope\"}"}},
                    "m2": "not-json"}}]}),
        );
        store::set_database_path_override(Some(database));
        let session = read("s-x").unwrap();
        store::set_database_path_override(None);

        let call = session.messages[0].blocks[0].tool.as_ref().unwrap();
        assert_eq!(call.op.as_deref(), Some(CanonicalOp::TOOL_INVOKE));
        assert_eq!(
            call.input,
            json!({"namespace": "cursor", "name": "mcp-ida-pro-decompile",
                   "input": {"ea": 4096}})
        );
        assert_eq!(
            call.result.as_ref().unwrap().status,
            ToolResultStatus::Error
        );
        // 坏 bubble 与失效引用各记一条损耗，不影响其余消息。
        let codes: Vec<(&str, &Value)> = session
            .loss
            .iter()
            .map(|event| (event.code.as_str(), &event.params["reason"]))
            .collect();
        assert_eq!(
            codes,
            [
                ("session.malformed_record", &json!("dangling_bubble")),
                ("session.malformed_record", &json!("invalid_bubble")),
            ]
        );
    }

    #[test]
    fn results_project_to_text_metadata_or_nothing() {
        // 只剩指针与渲染提示的结果不产出任何块（本机 73% 的 read_file_v2、
        // 全部 edit_file_v2 与 task_v2 都是这一类）。
        for metadata in [
            json!({"totalLinesInFile": 642}),
            json!({"beforeContentId": "composer.content.a", "afterContentId": "composer.content.b"}),
            json!({"agentId": "sub", "isBackground": false}),
            json!({}),
        ] {
            assert!(result_blocks(&metadata).is_empty(), "{metadata}");
        }
        assert_eq!(
            result_blocks(&json!({"contents": "# hi", "totalLinesInFile": 1}))[0].text,
            "# hi"
        );
        assert_eq!(
            result_blocks(&json!({"markdown": "page", "url": "https://x"}))[0].text,
            "page"
        );
        let structured = result_blocks(&json!({"directories": [], "isFinal": true}));
        assert_eq!(structured[0].kind, ToolResultBlockKind::Json);
        assert_eq!(structured[0].data, json!({"directories": []}));
    }

    #[test]
    fn a_non_zero_shell_exit_turns_a_completed_call_into_an_error() {
        let data: ToolFormerData = serde_json::from_value(json!({
            "name": "run_terminal_command_v2", "status": "completed",
            "toolCallId": "call_1",
            "params": "{\"command\": \"false\"}",
            "result": "{\"output\": \"boom\", \"exitCode\": 1, \"rejected\": false}",
        }))
        .unwrap();
        let call = tool_call(&data);
        assert_eq!(call.op.as_deref(), Some(CanonicalOp::SHELL_EXEC));
        let result = call.result.unwrap();
        assert_eq!(result.exit_code, Some(1));
        assert_eq!(result.status, ToolResultStatus::Error);
        assert_eq!(result.blocks[0].text, "boom");
    }

    #[test]
    fn an_unknown_session_is_not_found() {
        let (_root, _guard) = with_fixture();
        let error = read("nope").unwrap_err();
        store::set_database_path_override(None);
        assert_eq!(error.code, "session.not_found");
    }
}
