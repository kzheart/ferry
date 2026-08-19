//! Ferry 支持的唯一一套 OpenCode export/import 结构。
//!
//! `templates()` 返回的模板是 writer 组装 payload 的骨架：所有原生记录都从
//! 模板 clone 出来再覆盖字段，因此 OpenCode 一旦改结构，只需要换这份模板。

use std::sync::LazyLock;

use serde_json::{json, Map, Value};

use crate::errors::{DomainError, DomainResult};

/// 三张表的 capture 行 → 官方 export 形状。
pub fn export_from_capture(capture: &Value) -> DomainResult<Value> {
    let session = capture
        .get("session")
        .ok_or_else(|| DomainError::internal("OpenCode capture 缺少 session"))?;
    let info = match session.get("data") {
        Some(Value::String(text)) => serde_json::from_str(text).map_err(|error| {
            DomainError::internal(format!("OpenCode capture session 非法: {error}"))
        })?,
        _ => session.clone(),
    };

    let mut parts: Map<String, Value> = Map::new();
    for row in capture
        .get("parts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let part: Value = parse_data(&row)?;
        let message_id = row
            .get("message_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        match parts.get_mut(&message_id) {
            Some(Value::Array(items)) => items.push(part),
            _ => {
                parts.insert(message_id, Value::Array(vec![part]));
            }
        }
    }

    let mut messages = Vec::new();
    for row in capture
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let id = row.get("id").and_then(Value::as_str).unwrap_or_default();
        let mut message = Map::new();
        message.insert("info".into(), parse_data(&row)?);
        message.insert(
            "parts".into(),
            parts
                .get(id)
                .cloned()
                .unwrap_or_else(|| Value::Array(Vec::new())),
        );
        messages.push(Value::Object(message));
    }

    let mut payload = Map::new();
    payload.insert("info".into(), info);
    payload.insert("messages".into(), Value::Array(messages));
    Ok(Value::Object(payload))
}

fn parse_data(row: &Value) -> DomainResult<Value> {
    let text = row
        .get("data")
        .and_then(Value::as_str)
        .ok_or_else(|| DomainError::internal("OpenCode capture 行缺少 data"))?;
    serde_json::from_str(text)
        .map_err(|error| DomainError::internal(format!("OpenCode capture 行非法: {error}")))
}

/// 从 capture 里抽出模板记录；缺任何一类必需模板都是装配缺陷。
pub fn extract_templates(capture: &Value) -> DomainResult<Map<String, Value>> {
    let data = export_from_capture(capture)?;
    let mut templates = Map::new();
    templates.insert("info".into(), data["info"].clone());
    for message in data["messages"].as_array().cloned().unwrap_or_default() {
        let role = message["info"]
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("None")
            .to_string();
        templates
            .entry(format!("msg.{role}"))
            .or_insert_with(|| message["info"].clone());
        for part in message["parts"].as_array().cloned().unwrap_or_default() {
            let kind = part
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("None")
                .to_string();
            templates.entry(format!("part.{kind}")).or_insert(part);
        }
    }
    let mut missing: Vec<&str> = REQUIRED_TEMPLATES
        .iter()
        .copied()
        .filter(|key| !templates.contains_key(*key))
        .collect();
    if !missing.is_empty() {
        missing.sort_unstable();
        return Err(DomainError::internal(format!(
            "OpenCode fixture is missing template records: {}",
            missing.join(", ")
        )));
    }
    Ok(templates)
}

/// `extract_templates` 的必需模板键。
pub const REQUIRED_TEMPLATES: &[&str] = &[
    "info",
    "msg.user",
    "msg.assistant",
    "part.text",
    "part.tool",
];

static TEMPLATES: LazyLock<Map<String, Value>> = LazyLock::new(|| {
    json!({
        "info": {
            "id": "fixture-opencode-tools",
            "directory": "/fixture/opencode/tools",
            "title": "Tools fixture",
            "version": "1.18.3"
        },
        "msg.user": {
            "id": "fixture-message-user-tools",
            "sessionID": "fixture-opencode-tools",
            "role": "user"
        },
        "part.text": {
            "id": "fixture-part-user-tools",
            "messageID": "fixture-message-user-tools",
            "sessionID": "fixture-opencode-tools",
            "type": "text",
            "text": "Run the fixture shell, write, and read operations."
        },
        "msg.assistant": {
            "id": "fixture-message-assistant-tools",
            "sessionID": "fixture-opencode-tools",
            "parentID": "fixture-message-user-tools",
            "role": "assistant",
            "finish": "tool-calls"
        },
        "part.tool": {
            "id": "fixture-part-shell",
            "messageID": "fixture-message-assistant-tools",
            "sessionID": "fixture-opencode-tools",
            "type": "tool",
            "tool": "bash",
            "callID": "fixture-call-shell",
            "state": {
                "status": "completed",
                "input": {"command": "echo format-fixture-shell-test"},
                "output": "format-fixture-shell-test\n",
                "metadata": {"exit": 0}
            }
        }
    })
    .as_object()
    .cloned()
    .expect("模板是常量对象")
});

/// 当前原生记录模板的独立副本。
pub fn templates() -> Map<String, Value> {
    TEMPLATES.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Python 侧的验收断言：从 tools fixture 抽模板必须等于内建模板。
    #[test]
    fn extract_templates_matches_the_builtin_templates() {
        let fixture = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/agent_formats/opencode/case-02-tools/session.json"),
        )
        .expect("tools fixture 可读");
        let capture: Value = serde_json::from_str(&fixture).unwrap();
        assert_eq!(extract_templates(&capture).unwrap(), templates());
    }

    #[test]
    fn missing_required_templates_are_reported_by_name() {
        let capture = json!({
            "session": {"id": "s"},
            "messages": [{"id": "m", "data": "{\"id\":\"m\",\"role\":\"user\"}"}],
            "parts": []
        });
        let error = extract_templates(&capture).unwrap_err();
        assert!(error
            .message()
            .contains("msg.assistant, part.text, part.tool"));
    }
}
