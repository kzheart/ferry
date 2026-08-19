//! Ferry 支持的唯一一种 Codex rollout 结构。
//!
//! 语义事实源：`engine/adapters/codex/native_schema.py`。

use serde_json::{json, Map, Value};

/// 从真实 rollout 记录里抽取模板；缺少必需模板即报错。
///
/// 对齐 `extract_templates`：`templates.setdefault` 保留**首次**出现的记录。
pub fn extract_templates(records: &[Value]) -> Result<Map<String, Value>, String> {
    let mut templates = Map::new();
    for record in records {
        // Python 用 `record["type"]`：缺 type 直接 KeyError，这里同样视为非法。
        let record_type = record
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| "Codex fixture record 缺少 type".to_string())?;
        let payload_type = record
            .get("payload")
            .and_then(Value::as_object)
            .and_then(|payload| payload.get("type"))
            .and_then(Value::as_str);
        let key = match payload_type {
            Some(payload_type) => format!("{record_type}.{payload_type}"),
            None => record_type.to_string(),
        };
        templates
            .entry(key.clone())
            .or_insert_with(|| record.clone());
        if key == "response_item.message" {
            let role = record
                .get("payload")
                .and_then(Value::as_object)
                .and_then(|payload| payload.get("role"))
                .and_then(Value::as_str)
                .ok_or_else(|| "Codex fixture message 缺少 role".to_string())?;
            templates
                .entry(format!("message.{role}"))
                .or_insert_with(|| record.clone());
        }
    }
    let required = [
        "session_meta",
        "response_item.custom_tool_call",
        "response_item.custom_tool_call_output",
        "response_item.function_call",
        "response_item.function_call_output",
        "message.user",
        "message.assistant",
    ];
    let mut missing: Vec<&str> = required
        .iter()
        .copied()
        .filter(|key| !templates.contains_key(*key))
        .collect();
    if !missing.is_empty() {
        missing.sort_unstable();
        return Err(format!(
            "Codex fixture is missing template records: {}",
            missing.join(", ")
        ));
    }
    Ok(templates)
}

fn current_templates() -> Map<String, Value> {
    let value = json!({
        "session_meta": {
            "type": "session_meta",
            "payload": {
                "id": "fixture-codex-tools",
                "session_id": "fixture-codex-tools",
                "cwd": "/fixture/codex/tools",
                "cli_version": "0.144.0",
            },
        },
        "turn_context": {
            "type": "turn_context",
            "payload": {
                "turn_id": "fixture-turn-tools",
                "cwd": "/fixture/codex/tools",
            },
        },
        "response_item.message": {
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "user",
                "content": [
                    {
                        "type": "input_text",
                        "text": "Run the fixture shell, write, and read operations.",
                    }
                ],
            },
        },
        "response_item.custom_tool_call": {
            "type": "response_item",
            "payload": {
                "type": "custom_tool_call",
                "id": "fixture-custom-call-shell",
                "status": "completed",
                "call_id": "fixture-call-shell",
                "name": "exec",
                "input": concat!(
                    "const r = await tools.exec_command({\"cmd\":\"echo ",
                    "format-fixture-shell-test\",\"workdir\":\"/fixture/codex/tools\"});\n",
                    "text(JSON.stringify(r));\n"
                ),
            },
        },
        "response_item.custom_tool_call_output": {
            "type": "response_item",
            "payload": {
                "type": "custom_tool_call_output",
                "call_id": "fixture-call-shell",
                "output": [
                    {
                        "type": "input_text",
                        "text": "{\"exit_code\":0,\"output\":\"format-fixture-shell-test\\n\"}",
                    }
                ],
            },
        },
        "response_item.function_call": {
            "type": "response_item",
            "payload": {
                "type": "function_call",
                "id": "fixture-function-call-shell",
                "status": "completed",
                "call_id": "fixture-function-shell",
                "name": "exec_command",
                "arguments": "{\"cmd\":\"pwd\",\"workdir\":\"/fixture/codex/tools\"}",
            },
        },
        "response_item.function_call_output": {
            "type": "response_item",
            "payload": {
                "type": "function_call_output",
                "call_id": "fixture-function-shell",
                "output": "/fixture/codex/tools",
            },
        },
        "message.user": {
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "user",
                "content": [
                    {
                        "type": "input_text",
                        "text": "Run the fixture shell, write, and read operations.",
                    }
                ],
            },
        },
        "message.assistant": {
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": "fixture-message-assistant-tools",
                "role": "assistant",
                "content": [
                    {
                        "type": "output_text",
                        "text": "Fixture operations completed.",
                    }
                ],
                "phase": "final_answer",
            },
        },
    });
    value.as_object().cloned().expect("模板常量是对象")
}

/// 返回一份独立的当前原生记录模板（等价 Python 的 `deepcopy`）。
pub fn templates() -> Map<String, Value> {
    current_templates()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 语义事实源里的 `extract_templates(fixture) == templates()` 断言移植。
    #[test]
    fn extracted_templates_match_the_declared_ones() {
        let declared = templates();
        // 用声明式模板自身反推 fixture 记录流：先 message.user 再 message.assistant，
        // 保证 `response_item.message` 命中 user 那条（与真实 fixture 顺序一致）。
        let records: Vec<Value> = [
            "session_meta",
            "turn_context",
            "message.user",
            "response_item.custom_tool_call",
            "response_item.custom_tool_call_output",
            "response_item.function_call",
            "response_item.function_call_output",
            "message.assistant",
        ]
        .iter()
        .map(|key| declared[*key].clone())
        .collect();
        let extracted = extract_templates(&records).unwrap();
        for (key, value) in &declared {
            assert_eq!(extracted.get(key), Some(value), "模板 {key} 不一致");
        }
        assert_eq!(extracted.len(), declared.len());
    }

    #[test]
    fn missing_templates_are_reported_by_name() {
        let error = extract_templates(&[templates()["session_meta"].clone()]).unwrap_err();
        assert!(error.starts_with("Codex fixture is missing template records: "));
        assert!(error.contains("message.assistant"));
    }
}
