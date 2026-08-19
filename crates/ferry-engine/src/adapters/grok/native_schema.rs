//! 当前 Grok summary/update/chat v1 的结构模板。
//!
//! 模板是「格式漂移探测器」：抽取器要求 capture 里必须出现当前格式的全部代表性
//! 记录，缺一条就说明 fixture（或 Grok 本身）已经不是当前形态。

use std::collections::BTreeMap;
use std::sync::LazyLock;

use serde_json::{json, Value};

/// 模板表：`summary` / `update.<kind>` / `chat.<type>`。
pub type Templates = BTreeMap<String, Value>;

/// 必须出现的模板键。
const REQUIRED: [&str; 7] = [
    "summary",
    "update.UserMessage",
    "update.AgentMessageChunk",
    "update.ToolCall",
    "update.ToolCallUpdate",
    "chat.user",
    "chat.assistant",
];

/// 从一份原生 capture 抽取结构模板；缺少当前格式记录即报错。
pub fn extract_templates(capture: &Value) -> Result<Templates, String> {
    let mut templates: Templates = BTreeMap::new();
    let summary = capture
        .get("summary")
        .ok_or_else(|| "Grok fixture 缺少 summary".to_string())?;
    templates.insert("summary".into(), summary.clone());
    if let Some(updates) = capture.get("updates").and_then(Value::as_array) {
        for envelope in updates {
            let params = envelope.get("params");
            let meta = params.and_then(|params| params.get("_meta"));
            let kind = meta
                .and_then(|meta| meta.get("updateType"))
                .filter(|value| !value.is_null())
                .or_else(|| {
                    params
                        .and_then(|params| params.get("update"))
                        .and_then(|update| update.get("kind"))
                })
                .map(label)
                .unwrap_or_else(|| "None".to_string());
            templates
                .entry(format!("update.{kind}"))
                .or_insert_with(|| envelope.clone());
        }
    }
    if let Some(rows) = capture.get("chat").and_then(Value::as_array) {
        for row in rows {
            let kind = row.get("type").map(label).unwrap_or_else(|| "None".into());
            templates
                .entry(format!("chat.{kind}"))
                .or_insert_with(|| row.clone());
        }
    }
    let mut missing: Vec<&str> = REQUIRED
        .iter()
        .copied()
        .filter(|key| !templates.contains_key(*key))
        .collect();
    missing.sort_unstable();
    if !missing.is_empty() {
        // 缺失键按字典序列出，文案本身是维护者读的诊断信息。
        return Err(format!(
            "Grok fixture is missing current template records: {}",
            missing.join(", ")
        ));
    }
    Ok(templates)
}

/// Python 的 f-string 插值：字符串取字面量，其余走 `str(value)`。
fn label(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "None".to_string(),
        other => crate::adapters::shared::dialect::python_str(other),
    }
}

static TEMPLATES: LazyLock<Templates> =
    LazyLock::new(|| extract_templates(&capture()).expect("内置 Grok capture 必须覆盖当前格式"));

/// 当前格式模板的副本。
pub fn templates() -> Templates {
    TEMPLATES.clone()
}

/// 内置的当前格式 capture，与 `native_schema.py` 的 `_TEMPLATES` 逐字段一致。
fn capture() -> Value {
    json!({
        "summary": {
            "info": {"id": "fixture-grok-tools", "cwd": "/fixture/grok/tools"},
            "session_summary": "Tools fixture", "generated_title": "Tools fixture",
            "created_at": "2026-07-25T13:00:00Z",
            "updated_at": "2026-07-25T13:00:04Z", "num_messages": 4,
            "num_chat_messages": 4, "current_model_id": "grok-code-fast-1",
            "chat_format_version": 1
        },
        "updates": [
            {"method": "session/update", "params": {
                "sessionId": "fixture-grok-tools",
                "update": {"kind": "user_message", "content": {
                    "type": "text", "text": "Read /fixture/grok/tools/input.txt."
                }},
                "_meta": {"promptId": "p1", "promptIndex": 0,
                          "updateType": "UserMessage"}
            }},
            {"method": "session/update", "params": {
                "sessionId": "fixture-grok-tools",
                "update": {"content": {"type": "text", "text": "Inspecting "}},
                "_meta": {"promptId": "p1", "promptIndex": 0,
                          "updateType": "AgentMessageChunk", "chunkId": "c1"}
            }},
            {"method": "session/update", "params": {
                "sessionId": "fixture-grok-tools",
                "update": {"kind": "read", "rawInput": {
                    "path": "/fixture/grok/tools/input.txt"
                }},
                "_meta": {"promptId": "p1", "promptIndex": 0,
                          "updateType": "ToolCall", "updateParams": {
                              "kind": "read", "status": "Pending",
                              "toolCallId": "tool-1"
                          }}
            }},
            {"method": "session/update", "params": {
                "sessionId": "fixture-grok-tools",
                "update": {"kind": "read", "rawOutput": {
                    "FileContent": {
                        "absolute_path": "/fixture/grok/tools/input.txt",
                        "content": "sk-test-fixture", "total_lines": 1
                    }
                }},
                "_meta": {"promptId": "p1", "promptIndex": 0,
                          "updateType": "ToolCallUpdate", "updateParams": {
                              "kind": "read", "status": "Completed",
                              "toolCallId": "tool-1"
                          }}
            }}
        ],
        "chat": [
            {"type": "user", "id": "u1",
             "content": [{"type": "text",
                          "text": "Read /fixture/grok/tools/input.txt."}]},
            {"type": "assistant", "id": "a1", "content": "Inspecting now.",
             "tool_calls": [{"id": "tool-1", "name": "read",
                             "arguments": {
                                 "path": "/fixture/grok/tools/input.txt"
                             }}]},
            {"type": "tool_result", "id": "r1", "tool_call_id": "tool-1",
             "content": "sk-test-fixture"}
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 移植 `tests/test_current_native_formats.py::
    /// test_grok_current_structure_is_bundle_with_chat_v1`。
    #[test]
    fn the_current_structure_is_a_bundle_with_chat_v1() {
        let current = templates();
        assert_eq!(current["summary"]["chat_format_version"], json!(1));
        for key in [
            "update.UserMessage",
            "update.AgentMessageChunk",
            "update.ToolCall",
            "update.ToolCallUpdate",
            "chat.user",
            "chat.assistant",
        ] {
            assert!(current.contains_key(key), "缺少模板键 {key}");
        }
        // chat.tool_result 也在，但不是断言的一部分。
        assert!(current.contains_key("chat.tool_result"));
    }

    #[test]
    fn a_capture_missing_current_records_is_rejected() {
        let mut capture = capture();
        capture["updates"] = json!([]);
        let error = extract_templates(&capture).unwrap_err();
        assert_eq!(
            error,
            "Grok fixture is missing current template records: \
             update.AgentMessageChunk, update.ToolCall, update.ToolCallUpdate, \
             update.UserMessage"
        );
    }

    #[test]
    fn the_first_record_of_each_kind_wins() {
        let mut capture = capture();
        let updates = capture["updates"].as_array_mut().unwrap();
        let mut duplicate = updates[0].clone();
        duplicate["params"]["update"]["content"]["text"] = json!("second");
        updates.push(duplicate);
        let templates = extract_templates(&capture).unwrap();
        assert_eq!(
            templates["update.UserMessage"]["params"]["update"]["content"]["text"],
            json!("Read /fixture/grok/tools/input.txt.")
        );
    }
}
