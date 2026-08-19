//! Ferry 支持的唯一一套 Claude Code 原生结构。
//!
//! 语义事实源：`engine/adapters/claude/native_schema.py`。
//!
//! `templates()` 是 writer 生成记录的骨架；键序即写盘时的 JSON 键序，
//! 因此这里用 `json!` 字面量（`serde_json` 开了 `preserve_order`）而不是 Map 拼装。

use serde_json::{json, Map, Value};

use crate::errors::{DomainError, DomainResult};

/// 从一份原生 capture 里抽出 user / assistant 两个模板记录。
///
/// 两者缺一即视为 fixture 不合法（对齐 Python 的 `ValueError`）。
pub fn extract_templates(records: &[Value]) -> DomainResult<Map<String, Value>> {
    let mut templates = Map::new();
    for record in records {
        let kind = record.get("type").and_then(Value::as_str);
        if kind == Some("user")
            && !templates.contains_key("user")
            && record
                .get("message")
                .and_then(|message| message.get("content"))
                .is_some_and(Value::is_string)
        {
            templates.insert("user".into(), record.clone());
        }
        if kind == Some("assistant") && !templates.contains_key("assistant") {
            templates.insert("assistant".into(), record.clone());
        }
    }
    if templates.len() != 2 {
        return Err(DomainError::internal(
            "Claude fixture must contain user and assistant records",
        ));
    }
    Ok(templates)
}

fn current_templates() -> Map<String, Value> {
    let value = json!({
        "user": {
            "parentUuid": null,
            "isSidechain": false,
            "promptId": "fixture-prompt-tools",
            "type": "user",
            "message": {
                "role": "user",
                "content": "Run the fixture shell, write, and read operations."
            },
            "uuid": "fixture-message-user-tools",
            "cwd": "/fixture/claude/tools",
            "sessionId": "fixture-claude-tools",
            "version": "2.1.204"
        },
        "assistant": {
            "parentUuid": "fixture-message-user-tools",
            "isSidechain": false,
            "type": "assistant",
            "message": {
                "type": "message",
                "role": "assistant",
                "content": [
                    {
                        "type": "tool_use",
                        "id": "fixture-tool-shell",
                        "name": "Bash",
                        "input": {"command": "echo format-fixture-shell-test"}
                    }
                ],
                "stop_reason": "tool_use"
            },
            "uuid": "fixture-message-assistant-shell",
            "cwd": "/fixture/claude/tools",
            "sessionId": "fixture-claude-tools",
            "version": "2.1.204"
        }
    });
    value.as_object().cloned().expect("模板字面量是对象")
}

/// 返回一份独立的当前原生记录模板副本。
pub fn templates() -> Map<String, Value> {
    current_templates()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 对齐 `test_current_native_formats` 里的 claude 断言：
    /// 从真实 fixture 抽出的模板必须与内置模板逐字段一致。
    #[test]
    fn extracted_templates_match_the_builtin_ones() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/agent_formats/claude/case-02-tools/session.jsonl");
        let text = std::fs::read_to_string(&fixture).expect("fixture 可读");
        let records: Vec<Value> = text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).expect("fixture 是合法 JSONL"))
            .collect();
        assert_eq!(extract_templates(&records).unwrap(), templates());
    }

    #[test]
    fn missing_roles_are_rejected() {
        let records = vec![json!({"type": "user", "message": {"content": "hi"}})];
        assert!(extract_templates(&records).is_err());
    }
}
