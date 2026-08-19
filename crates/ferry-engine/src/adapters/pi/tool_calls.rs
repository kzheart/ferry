//! Pi 工具调用归一与结果配对。
//!
//! 语义事实源：`engine/adapters/pi/tool_calls.py`。

use serde_json::{Map, Value};

use crate::adapters::shared::dialect::python_str;
use crate::model::{ToolCall, ToolResult, ToolResultBlock, ToolResultBlockKind, ToolResultStatus};
use crate::tool_ops::CanonicalOp;

use super::dialect::DIALECT;

/// 原生入参 → `(规范操作, 规范入参)`；方言归一失败时退回 `tool.invoke` 私有调用，
/// 原始参数全量保留在 `input` 里。
pub fn normalize_input(name: &str, value: &Value) -> (String, Value) {
    match DIALECT.parse(name, value) {
        Some((op, canonical)) => (op.to_string(), canonical),
        None => {
            let mut fallback = Map::new();
            fallback.insert("namespace".into(), Value::from("pi"));
            fallback.insert("name".into(), Value::from(name));
            fallback.insert("input".into(), value.clone());
            (
                CanonicalOp::TOOL_INVOKE.to_string(),
                Value::Object(fallback),
            )
        }
    }
}

/// assistant content 里的一个 `toolCall` part → 规范 `ToolCall`。
pub fn call_from_part(part: &Value, message_id: &str) -> ToolCall {
    // Python 的 `str(part.get("name") or "")`：falsy 一律成空串。
    let name = match part.get("name") {
        Some(value) if truthy(value) => python_str(value),
        _ => String::new(),
    };
    // `part.get("arguments") or {}`：缺席/falsy 都当空 dict。
    let arguments = match part.get("arguments") {
        Some(value) if truthy(value) => value.clone(),
        _ => Value::Object(Map::new()),
    };
    let (op, input) = normalize_input(&name, &arguments);
    let mut call = ToolCall::new(name, Some(op), input);
    call.source_call_id = part.get("id").and_then(Value::as_str).map(str::to_string);
    call.source_message_id = Some(message_id.to_string());
    call
}

/// `role == "toolResult"` 的原生消息 → 规范 `ToolResult`。
pub fn result_from_message(message: &Value) -> ToolResult {
    let mut blocks = Vec::new();
    for part in message
        .get("content")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        match part.get("type").and_then(Value::as_str) {
            Some("text") => {
                let text = match part.get("text") {
                    Some(value) if truthy(value) => python_str(value),
                    _ => String::new(),
                };
                blocks.push(ToolResultBlock::text(text));
            }
            Some("image") => {
                let mut block = ToolResultBlock::new(ToolResultBlockKind::Image);
                block.data = part.get("data").cloned().unwrap_or(Value::Null);
                block.mime_type = part
                    .get("mimeType")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                blocks.push(block);
            }
            _ => {
                let mut block = ToolResultBlock::new(ToolResultBlockKind::Json);
                block.data = part.clone();
                blocks.push(block);
            }
        }
    }
    // `details` 既可能是单个对象也可能是数组；缺席时不产生附件。
    let attachments = match message.get("details") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items.clone(),
        Some(other) => vec![other.clone()],
    };
    ToolResult {
        status: if message.get("isError").is_some_and(truthy) {
            ToolResultStatus::Error
        } else {
            ToolResultStatus::Success
        },
        blocks,
        attachments,
        ..ToolResult::default()
    }
}

/// Python `bool(value)` 的 JSON 等价。
pub(super) fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|float| float != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(entries) => !entries.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unknown_tools_fall_back_to_tool_invoke_with_the_raw_input() {
        let (op, input) = normalize_input("mystery", &json!({"a": 1}));
        assert_eq!(op, CanonicalOp::TOOL_INVOKE);
        assert_eq!(
            input,
            json!({"namespace": "pi", "name": "mystery", "input": {"a": 1}})
        );
    }

    #[test]
    fn call_from_part_keeps_source_ids() {
        let call = call_from_part(
            &json!({"type": "toolCall", "id": "c1", "name": "read",
                    "arguments": {"path": "/a.txt"}}),
            "a1",
        );
        assert_eq!(call.name, "read");
        assert_eq!(call.op.as_deref(), Some(CanonicalOp::FS_READ));
        assert_eq!(call.input, json!({"file_path": "/a.txt"}));
        assert_eq!(call.source_call_id.as_deref(), Some("c1"));
        assert_eq!(call.source_message_id.as_deref(), Some("a1"));
        assert!(call.result.is_none());
    }

    #[test]
    fn missing_name_and_arguments_degrade_to_an_empty_invocation() {
        let call = call_from_part(&json!({"type": "toolCall"}), "a1");
        assert_eq!(call.name, "");
        assert_eq!(
            call.input,
            json!({"namespace": "pi", "name": "", "input": {}})
        );
    }

    #[test]
    fn result_blocks_cover_text_image_and_json_fallbacks() {
        let result = result_from_message(&json!({
            "role": "toolResult", "toolCallId": "c1",
            "content": [
                {"type": "text", "text": "out"},
                {"type": "image", "data": "AA==", "mimeType": "image/png"},
                {"type": "weird", "payload": 1},
            ],
            "isError": true,
            "details": {"path": "/tmp/full.txt"},
        }));
        assert_eq!(result.status, ToolResultStatus::Error);
        let kinds: Vec<ToolResultBlockKind> = result.blocks.iter().map(|b| b.kind).collect();
        assert_eq!(
            kinds,
            [
                ToolResultBlockKind::Text,
                ToolResultBlockKind::Image,
                ToolResultBlockKind::Json
            ]
        );
        assert_eq!(result.blocks[1].mime_type.as_deref(), Some("image/png"));
        assert_eq!(
            result.blocks[2].data,
            json!({"type": "weird", "payload": 1})
        );
        // 单个 details 会被包成单元素附件列表。
        assert_eq!(result.attachments, vec![json!({"path": "/tmp/full.txt"})]);
    }

    #[test]
    fn list_details_stay_a_list_and_missing_details_produce_nothing() {
        let listed = result_from_message(&json!({"details": [{"a": 1}, {"b": 2}]}));
        assert_eq!(listed.attachments.len(), 2);
        let bare = result_from_message(&json!({"content": []}));
        assert!(bare.attachments.is_empty());
        assert_eq!(bare.status, ToolResultStatus::Success);
    }
}
