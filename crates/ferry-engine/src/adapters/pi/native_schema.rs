//! Ferry 支持的唯一当前 Pi 会话结构。
//!
//! 这份模板既是文档也是断言：真实 capture 一旦缺少任一必需模板记录，
//! [`extract_templates`] 就会失败，提示 pi 的原生格式已经变了。

use std::sync::LazyLock;

use serde_json::{json, Map, Value};

/// 必须能从 capture 里抽出的模板键。
const REQUIRED: [&str; 10] = [
    "session",
    "message.user",
    "message.assistant",
    "message.toolResult",
    "content.text",
    "content.toolCall",
    "message.bashExecution",
    "content.thinking",
    "content.image",
    "compaction",
];

/// 从一串原生记录里抽取「每种结构各一份」的模板。
pub fn extract_templates(records: &[Value]) -> Result<Map<String, Value>, String> {
    let mut templates: Map<String, Value> = Map::new();
    for record in records {
        let kind = record.get("type").and_then(Value::as_str);
        let mut key = kind.map(str::to_string);
        if kind == Some("message") {
            let empty = Value::Object(Map::new());
            let message = record.get("message").unwrap_or(&empty);
            let role = message.get("role").and_then(Value::as_str);
            key = Some(format!(
                "message.{}",
                role.map_or("None".to_string(), str::to_string)
            ));
            // 注意顺序：content 模板先于 message 模板落库（对齐 Python）。
            for part in message
                .get("content")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default()
            {
                let part_kind = part
                    .get("type")
                    .and_then(Value::as_str)
                    .map_or("None".to_string(), str::to_string);
                templates
                    .entry(format!("content.{part_kind}"))
                    .or_insert_with(|| part.clone());
            }
        }
        if let Some(key) = key {
            templates.entry(key).or_insert_with(|| record.clone());
        }
    }
    let mut missing: Vec<&str> = REQUIRED
        .iter()
        .copied()
        .filter(|key| !templates.contains_key(*key))
        .collect();
    if !missing.is_empty() {
        missing.sort_unstable();
        return Err(format!(
            "Pi fixture is missing template records: {}",
            missing.join(", ")
        ));
    }
    Ok(templates)
}

fn fixture() -> Vec<Value> {
    vec![
        json!({"type": "session", "version": 3, "id": "fixture-pi-tools",
               "timestamp": "2026-07-25T10:00:00.000Z", "cwd": "/fixture/pi/tools"}),
        json!({"type": "message", "id": "u1", "parentId": null,
               "timestamp": "2026-07-25T10:00:01.000Z",
               "message": {"role": "user", "content": [
                   {"type": "text",
                    "text": "Inspect /fixture/pi/tools and token sk-test-fixture."},
                   {"type": "image", "data": "iVBORw0KGgo=", "mimeType": "image/png"}],
                   "timestamp": 1784973601000i64}}),
        json!({"type": "message", "id": "a1", "parentId": "u1",
               "timestamp": "2026-07-25T10:00:02.000Z",
               "message": {"role": "assistant", "content": [
                   {"type": "thinking", "thinking": "Use two tools."},
                   {"type": "text", "text": "I will inspect it."},
                   {"type": "toolCall", "id": "call-1", "name": "read",
                    "arguments": {"path": "/fixture/pi/tools/input.txt"}}],
                   "api": "anthropic-messages", "provider": "fixture",
                   "model": "fixture-model",
                   "usage": {"input": 10, "output": 5, "cacheRead": 2,
                             "cacheWrite": 1, "totalTokens": 18,
                             "cost": {"input": 0, "output": 0, "cacheRead": 0,
                                      "cacheWrite": 0, "total": 0}},
                   "stopReason": "toolUse", "timestamp": 1784973602000i64}}),
        json!({"type": "message", "id": "r1", "parentId": "a1",
               "timestamp": "2026-07-25T10:00:03.000Z",
               "message": {"role": "toolResult", "toolCallId": "call-1",
                   "toolName": "read",
                   "content": [{"type": "text", "text": "fixture output"}],
                   "isError": false, "timestamp": 1784973603000i64}}),
        json!({"type": "message", "id": "b1", "parentId": "r1",
               "timestamp": "2026-07-25T10:00:03.500Z",
               "message": {"role": "bashExecution", "command": "pwd",
                   "output": "/fixture/pi/tools\n", "exitCode": 0,
                   "cancelled": false, "truncated": false,
                   "timestamp": 1784973603500i64}}),
        json!({"type": "compaction", "id": "c1", "parentId": "b1",
               "timestamp": "2026-07-25T10:00:04.000Z",
               "summary": "Fixture summary", "firstKeptEntryId": "u1",
               "tokensBefore": 123}),
    ]
}

static TEMPLATES: LazyLock<Map<String, Value>> =
    LazyLock::new(|| extract_templates(&fixture()).expect("Pi 模板 fixture 必须自洽"));

/// 当前支持的 pi 结构模板（每次返回独立副本）。
pub fn templates() -> Map<String, Value> {
    TEMPLATES.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracting_from_the_fixture_reproduces_the_templates() {
        assert_eq!(extract_templates(&fixture()).unwrap(), templates());
    }

    #[test]
    fn results_are_independent_copies() {
        let mut first = templates();
        first.clear();
        assert!(!templates().is_empty());
    }

    /// 移植 `tests/test_current_native_formats.py::test_pi_current_structure_is_session_v3`。
    #[test]
    fn current_structure_is_session_v3() {
        let current = templates();
        assert_eq!(current["session"]["version"], Value::from(3));
        for key in [
            "message.user",
            "message.assistant",
            "message.toolResult",
            "message.bashExecution",
            "compaction",
        ] {
            assert!(current.contains_key(key), "缺少模板 {key}");
        }
    }

    #[test]
    fn missing_records_are_reported_by_name() {
        let error = extract_templates(&fixture()[..2]).unwrap_err();
        assert!(error.starts_with("Pi fixture is missing template records: "));
        assert!(error.contains("compaction"));
        assert!(error.contains("content.toolCall"));
    }
}
