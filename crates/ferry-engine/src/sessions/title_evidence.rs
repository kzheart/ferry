//! AI 重置标题的证据采集。
//!
//! 只走既有读取路径（`read_indexed_session`），只取**可见文本**：工具调用保留
//! 输入里的文件名，正文与工具输出一概不进证据——生成标题不需要它们，带上去只会
//! 把提示词撑大。单个会话失败进 `errors[]`，不让一次批量整体失败。

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::{json, Map, Value};

use crate::errors::{DomainError, DomainResult};
use crate::model::{BlockKind, Session};
use crate::operations::metadata_store::metadata_key;
use crate::operations::types::EngineResult;
use crate::storage::database::state_database;

use super::agent_read::read_indexed_session;
use super::index::AgentSessionIndex;
use super::safety::record_session_id;
use super::title_style;

/// 一次请求的会话数上限。
pub const MAX_SESSIONS: usize = 50;
/// 用户消息取前 3 条，各截 400 字符。
const USER_MESSAGES: usize = 3;
const USER_MESSAGE_CHARS: usize = 400;
/// 最后一条助手可见文本截 600 字符。
const ASSISTANT_CHARS: usize = 600;
/// 文件名去重后最多 20 个。
const MAX_FILES: usize = 20;

/// 工具调用输入里被认作「文件」的键。
const FILE_KEYS: &[&str] = &["file_path", "path", "filePath", "notebook_path"];

fn invalid(message: impl Into<String>) -> DomainError {
    DomainError::agent_request_invalid(message)
}

/// Agent 塞进用户轮次的系统性文本（AGENTS.md 指令、插件推荐、附件清单等），
/// 不是用户在说话，占掉证据名额只会把标题带偏。
fn is_injected_user_text(text: &str) -> bool {
    let head = text.trim_start();
    head.starts_with('<')
        || head.starts_with("# AGENTS.md")
        || head.starts_with("# Files mentioned by the user")
}

fn clip(text: &str, max: usize) -> String {
    let flattened = text.split_whitespace().collect::<Vec<_>>().join(" ");
    flattened.chars().take(max).collect()
}

/// `{"sessions": [{"tool","ref"}]}` → 去重后的 `(tool, ref)` 清单。
pub fn parse_targets(params: &Value) -> EngineResult<Vec<(String, String)>> {
    let items = params
        .get("sessions")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("title_evidence 需要 sessions 数组"))?;
    if items.is_empty() || items.len() > MAX_SESSIONS {
        return Err(invalid(format!("sessions 长度须在 1..{MAX_SESSIONS}")).into());
    }
    let mut targets: Vec<(String, String)> = Vec::new();
    for item in items {
        let entry = item
            .as_object()
            .ok_or_else(|| invalid("sessions[] 必须是 object"))?;
        let tool = entry
            .get("tool")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .ok_or_else(|| invalid("sessions[].tool 必须是非空字符串"))?
            .to_string();
        let reference = entry
            .get("ref")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .ok_or_else(|| invalid("sessions[].ref 必须是非空字符串"))?
            .to_string();
        let key = (tool, reference);
        if !targets.contains(&key) {
            targets.push(key);
        }
    }
    Ok(targets)
}

/// 从 canonical Session 抽出提示词需要的那几段。
pub fn extract(session: &Session) -> Map<String, Value> {
    let mut user_messages: Vec<Value> = Vec::new();
    let mut last_assistant: Option<String> = None;
    let mut files: BTreeSet<String> = BTreeSet::new();
    let mut ordered_files: Vec<String> = Vec::new();

    for message in &session.messages {
        let mut visible = String::new();
        for block in &message.blocks {
            match block.kind {
                BlockKind::Text => {
                    if !block.text.trim().is_empty() {
                        if !visible.is_empty() {
                            visible.push('\n');
                        }
                        visible.push_str(&block.text);
                    }
                }
                BlockKind::Tool => {
                    let Some(call) = block.tool.as_ref() else {
                        continue;
                    };
                    for key in FILE_KEYS {
                        let Some(name) = call.input.get(*key).and_then(Value::as_str) else {
                            continue;
                        };
                        let name = name.trim();
                        if name.is_empty() || ordered_files.len() >= MAX_FILES {
                            continue;
                        }
                        if files.insert(name.to_string()) {
                            ordered_files.push(name.to_string());
                        }
                    }
                }
                _ => {}
            }
        }
        if visible.trim().is_empty() {
            continue;
        }
        if message.role == "user" {
            if user_messages.len() < USER_MESSAGES && !is_injected_user_text(&visible) {
                user_messages.push(Value::from(clip(&visible, USER_MESSAGE_CHARS)));
            }
        } else if message.role == "assistant" {
            last_assistant = Some(clip(&visible, ASSISTANT_CHARS));
        }
    }

    let turn_count = session
        .messages
        .iter()
        .filter(|message| message.role == "user")
        .count();

    let mut result = Map::new();
    result.insert("message_count".into(), Value::from(session.messages.len()));
    result.insert("turn_count".into(), Value::from(turn_count));
    result.insert("user_messages".into(), Value::Array(user_messages));
    result.insert(
        "last_assistant_message".into(),
        last_assistant.map_or(Value::Null, Value::from),
    );
    result.insert(
        "files".into(),
        Value::Array(ordered_files.into_iter().map(Value::from).collect()),
    );
    result
}

fn one(index: &AgentSessionIndex, tool: &str, reference: &str) -> DomainResult<Map<String, Value>> {
    let record = index.resolve(tool, reference, true)?;
    let session = read_indexed_session(index, &record, true)?;
    let rename_capability = index
        .ports()
        .adapter(tool)
        .map(|adapter| adapter.require_renamer().is_ok())
        .unwrap_or(false);

    let mut item = Map::new();
    item.insert("tool".into(), Value::from(tool));
    item.insert("ref".into(), Value::from(reference));
    item.insert(
        "session_id".into(),
        Value::from(record_session_id(&record.row, Some(&session.source_id))),
    );
    item.insert("revision".into(), Value::from(record.revision.as_str()));
    item.insert("title".into(), Value::from(session.title.as_str()));
    item.insert(
        "title_source".into(),
        Value::from(
            record
                .row
                .get("title_source")
                .and_then(Value::as_str)
                .unwrap_or(""),
        ),
    );
    item.insert("project".into(), Value::from(session.cwd.as_str()));
    item.insert(
        "updated".into(),
        record.row.get("updated").cloned().unwrap_or(Value::Null),
    );
    item.extend(extract(&session));
    item.insert("rename_capability".into(), Value::Bool(rename_capability));
    Ok(item)
}

/// 本地名称是用户明确设置的覆盖，预览与手动标题保护均以它为准。
fn overlay_local_title(item: &mut Map<String, Value>, metadata: &Map<String, Value>) {
    let key = metadata_key(
        item.get("tool").and_then(Value::as_str).unwrap_or_default(),
        item.get("session_id")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    if let Some(name) = metadata
        .get(&key)
        .and_then(|entry| entry.get("name"))
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
    {
        item.insert("title".into(), Value::from(name));
        item.insert("title_source".into(), Value::from("manual"));
    }
}

/// `title_evidence` RPC。
pub fn collect(
    params: &Value,
    index: &AgentSessionIndex,
    state_dir: impl AsRef<Path>,
) -> EngineResult<Value> {
    let targets = parse_targets(params)?;
    // 读取失败时停止取证，避免将未能检查的手动标题送去覆盖。
    let metadata = state_database(state_dir.as_ref())?.metadata.list_all()?;
    let mut sessions: Vec<Value> = Vec::new();
    let mut errors: Vec<Value> = Vec::new();
    for (tool, reference) in targets {
        match one(index, &tool, &reference) {
            Ok(mut item) => {
                overlay_local_title(&mut item, &metadata);
                sessions.push(Value::Object(item));
            }
            Err(error) => errors.push(json!({
                "tool": tool, "ref": reference, "error": error.payload(),
            })),
        }
    }
    Ok(json!({
        "sessions": sessions,
        "errors": errors,
        "style": title_style::get(state_dir),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Block, Message, ToolCall};

    #[test]
    fn local_names_override_native_titles_and_clearing_restores_the_native_source() {
        let dir = tempfile::tempdir().unwrap();
        let database = state_database(dir.path()).unwrap();
        let original = json!({"tool": "cursor", "session_id": "sid",
            "title": "原生标题", "title_source": "native"});
        database
            .metadata
            .set(
                "cursor",
                "sid",
                json!({"name": "用户的本地名称"}).as_object().unwrap(),
                1,
            )
            .unwrap();
        // 同 id 不同来源的名称不能串到本会话。
        database
            .metadata
            .set(
                "claude",
                "sid",
                json!({"name": "另一个会话"}).as_object().unwrap(),
                1,
            )
            .unwrap();
        let mut item = original.as_object().unwrap().clone();
        overlay_local_title(&mut item, &database.metadata.list_all().unwrap());
        assert_eq!(item["title"], json!("用户的本地名称"));
        assert_eq!(item["title_source"], json!("manual"));

        database
            .metadata
            .set("cursor", "sid", json!({"name": ""}).as_object().unwrap(), 2)
            .unwrap();
        let mut item = original.as_object().unwrap().clone();
        overlay_local_title(&mut item, &database.metadata.list_all().unwrap());
        assert_eq!(Value::Object(item), original);
    }

    fn text_message(role: &str, text: &str) -> Message {
        let mut message = Message::new(role);
        let mut block = Block::new(BlockKind::Text);
        block.text = text.to_string();
        message.blocks.push(block);
        message
    }

    fn tool_message(input: Value) -> Message {
        let mut message = Message::new("assistant");
        let mut block = Block::new(BlockKind::Tool);
        block.tool = Some(ToolCall::new("Read", None, input));
        message.blocks.push(block);
        message
    }

    fn fixture() -> Session {
        let mut session = Session::new("claude", "sid", "/repo");
        session.title = "旧标题".into();
        session.messages = vec![
            text_message("user", "第一条 提问"),
            text_message("assistant", "第一条回答"),
            tool_message(json!({"file_path": "src/a.rs"})),
            text_message("user", "第二条提问"),
            tool_message(json!({"path": "src/b.rs", "notebook_path": "nb.ipynb"})),
            text_message("assistant", "最后一条回答"),
            text_message("user", "第三条提问"),
            text_message("user", "第四条提问"),
        ];
        session
    }

    #[test]
    fn injected_agent_text_is_not_user_evidence() {
        assert!(is_injected_user_text("<recommended_plugins> …"));
        assert!(is_injected_user_text(
            "# AGENTS.md instructions <INSTRUCTIONS>"
        ));
        assert!(is_injected_user_text(
            "  # Files mentioned by the user: a.png"
        ));
        assert!(!is_injected_user_text("# 需求\n给附魔分离加等级限制"));
        assert!(!is_injected_user_text("git pull 然后写使用文档"));
    }

    #[test]
    fn extraction_takes_three_user_texts_the_last_assistant_text_and_tool_files() {
        let evidence = extract(&fixture());
        assert_eq!(
            evidence["user_messages"],
            json!(["第一条 提问", "第二条提问", "第三条提问"])
        );
        assert_eq!(evidence["last_assistant_message"], json!("最后一条回答"));
        assert_eq!(
            evidence["files"],
            json!(["src/a.rs", "src/b.rs", "nb.ipynb"])
        );
        assert_eq!(evidence["message_count"], json!(8));
        assert_eq!(evidence["turn_count"], json!(4));
    }

    #[test]
    fn text_is_clipped_and_a_session_without_assistant_text_reports_null() {
        let mut session = Session::new("claude", "sid", "/repo");
        session.messages = vec![
            text_message("user", &"字".repeat(500)),
            tool_message(json!({"file_path": "only-tool.rs"})),
        ];
        let evidence = extract(&session);
        assert_eq!(
            evidence["user_messages"][0]
                .as_str()
                .unwrap()
                .chars()
                .count(),
            400
        );
        assert_eq!(evidence["last_assistant_message"], Value::Null);
        assert_eq!(evidence["files"], json!(["only-tool.rs"]));
    }

    #[test]
    fn files_are_deduplicated_and_capped_at_twenty() {
        let mut session = Session::new("claude", "sid", "/repo");
        session.messages = (0..30)
            .map(|index| tool_message(json!({"file_path": format!("f{index}.rs")})))
            .chain(std::iter::once(tool_message(json!({"file_path": "f0.rs"}))))
            .collect();
        let files = extract(&session)["files"].as_array().unwrap().len();
        assert_eq!(files, MAX_FILES);
    }

    #[test]
    fn targets_are_validated_and_deduplicated() {
        let params = json!({"sessions": [
            {"tool": "claude", "ref": "fsr_a"},
            {"tool": "claude", "ref": "fsr_a"},
            {"tool": "codex", "ref": "fsr_b"},
        ]});
        assert_eq!(
            parse_targets(&params).unwrap(),
            vec![
                ("claude".to_string(), "fsr_a".to_string()),
                ("codex".to_string(), "fsr_b".to_string()),
            ]
        );
        for bad in [
            json!({}),
            json!({"sessions": []}),
            json!({"sessions": {}}),
            json!({"sessions": ["claude"]}),
            json!({"sessions": [{"tool": "claude"}]}),
            json!({"sessions": [{"tool": "", "ref": "fsr_a"}]}),
        ] {
            assert!(parse_targets(&bad).is_err(), "应当拒绝: {bad}");
        }
        let too_many: Vec<Value> = (0..=MAX_SESSIONS)
            .map(|index| json!({"tool": "claude", "ref": format!("fsr_{index}")}))
            .collect();
        assert!(parse_targets(&json!({"sessions": too_many})).is_err());
    }
}
