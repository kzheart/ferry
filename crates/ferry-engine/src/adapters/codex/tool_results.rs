//! Codex 工具结果包络解析。

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use crate::adapters::shared::dialect::python_str;
use crate::model::{ToolResult, ToolResultBlock, ToolResultBlockKind, ToolResultStatus};

/// unified-exec（exec_command/write_stdin）的分块文本头，携带真实退出状态：
///
/// ```text
/// Chunk ID: 1448c0
/// Wall time: 0.0000 seconds
/// Process exited with code 128 | Process running with session ID x
/// ```
static UNIFIED_EXEC_HEAD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"\AChunk ID: \S+\nWall time: [\d.]+ seconds\n",
        r"(?:Process exited with code (-?\d+)",
        r"|Process running with session ID \S+)\n",
    ))
    .expect("unified-exec 头部正则是常量")
});

/// 结构化信封里出现任一键即判定为「Codex 输出包络」。
const ENVELOPE_KEYS: [&str; 7] = [
    "output",
    "stdout",
    "stderr",
    "exit_code",
    "status",
    "truncated",
    "attachments",
];

/// `Script completed` 包装块的前缀。
const WRAPPER_PREFIX: &str = "Script completed\nWall time ";

fn result_status(value: Option<&Value>) -> ToolResultStatus {
    match value.and_then(Value::as_str) {
        Some("success") | Some("completed") => ToolResultStatus::Success,
        Some("error") => ToolResultStatus::Error,
        Some("interrupted") => ToolResultStatus::Interrupted,
        Some("running") => ToolResultStatus::Running,
        Some("pending") => ToolResultStatus::Pending,
        _ => ToolResultStatus::Unknown,
    }
}

/// 把原生 output 归一成待遍历的块序列，对齐 Python 的三段兜底。
fn native_blocks(raw: &Value) -> Vec<Value> {
    // `raw if isinstance(raw, list) else json.loads(raw)`；
    // json.loads 只接受字符串，其余类型抛 TypeError 并被兜底成原值。
    let decoded = match raw {
        Value::Array(_) => raw.clone(),
        Value::String(text) => serde_json::from_str::<Value>(text).unwrap_or_else(|_| raw.clone()),
        other => other.clone(),
    };
    match decoded {
        Value::Array(items) => items,
        Value::Object(_) => vec![decoded],
        Value::String(text) => vec![text_envelope(&text)],
        other => vec![text_envelope(&python_str(&other))],
    }
}

fn text_envelope(text: &str) -> Value {
    let mut entry = serde_json::Map::new();
    entry.insert("type".into(), Value::from("input_text"));
    entry.insert("text".into(), Value::from(text));
    Value::Object(entry)
}

/// 解码 Codex 的输出包络，既不压平 status 也不丢弃富块。
pub fn parse_result(raw: &Value) -> ToolResult {
    let mut blocks: Vec<ToolResultBlock> = Vec::new();
    // 与 blocks 等长：标记「Script completed」包装块，结构化信封出现时整体剔除。
    let mut wrapper_flags: Vec<bool> = Vec::new();
    let mut stdout: Option<String> = None;
    let mut stderr: Option<String> = None;
    let mut exit_code: Option<i64> = None;
    let mut truncated: Option<bool> = None;
    let mut attachments: Vec<Value> = Vec::new();
    let mut explicit_status: Option<Value> = None;
    let mut structured_envelope = false;

    macro_rules! push {
        ($block:expr) => {{
            blocks.push($block);
            wrapper_flags.push(false);
        }};
        ($block:expr, $wrapper:expr) => {{
            blocks.push($block);
            wrapper_flags.push($wrapper);
        }};
    }

    for native_block in native_blocks(raw) {
        let Some(entry) = native_block.as_object() else {
            let mut block = ToolResultBlock::new(ToolResultBlockKind::Json);
            block.data = native_block.clone();
            push!(block);
            continue;
        };
        let kind = entry.get("type").and_then(Value::as_str).unwrap_or("");
        match kind {
            "input_text" | "output_text" | "text" => {
                let text = entry
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let inner = serde_json::from_str::<Value>(&text).ok();
                let envelope = inner
                    .as_ref()
                    .and_then(Value::as_object)
                    .filter(|inner| ENVELOPE_KEYS.iter().any(|key| inner.contains_key(*key)));
                if let Some(inner) = envelope {
                    structured_envelope = true;
                    let output = inner.get("output");
                    let stdout_value = inner.get("stdout").or(output);
                    if let Some(text) = stdout_value.and_then(Value::as_str) {
                        stdout = Some(text.to_string());
                    }
                    match output {
                        Some(Value::String(text)) if !text.is_empty() => {
                            push!(ToolResultBlock::text(text.as_str()));
                        }
                        // 缺席与 JSON null 都是 Python 的 None：不产出块。
                        None | Some(Value::Null) => {}
                        // 空串走 `elif output is not None` 分支，落成空 json 块。
                        Some(other) => {
                            let mut block = ToolResultBlock::new(ToolResultBlockKind::Json);
                            block.data = other.clone();
                            push!(block);
                        }
                    }
                    if let Some(text) = inner.get("stderr").and_then(Value::as_str) {
                        stderr = Some(text.to_string());
                    }
                    if let Some(code) = inner.get("exit_code") {
                        // Python 显式拒绝 bool；serde 的 bool 与 number 天然分离。
                        if let Some(code) = code.as_i64() {
                            exit_code = Some(code);
                        }
                    }
                    if let Some(flag) = inner.get("truncated").and_then(Value::as_bool) {
                        truncated = Some(flag);
                    }
                    if let Some(items) = inner.get("attachments").and_then(Value::as_array) {
                        attachments = items.clone();
                    }
                    explicit_status = inner.get("status").cloned();
                } else if !text.is_empty() {
                    let wrapper = text.starts_with(WRAPPER_PREFIX);
                    push!(ToolResultBlock::text(text.as_str()), wrapper);
                }
            }
            "input_image" | "output_image" | "image" => {
                let mut block = ToolResultBlock::new(ToolResultBlockKind::Image);
                block.uri = entry
                    .get("image_url")
                    .and_then(Value::as_str)
                    .or_else(|| entry.get("url").and_then(Value::as_str))
                    .map(str::to_string);
                block.data = entry.get("data").cloned().unwrap_or(Value::Null);
                block.mime_type = entry
                    .get("mime_type")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                push!(block);
            }
            "file" => {
                let mut block = ToolResultBlock::new(ToolResultBlockKind::File);
                block.uri = entry.get("url").and_then(Value::as_str).map(str::to_string);
                block.filename = entry
                    .get("filename")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                block.mime_type = entry
                    .get("mime_type")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                push!(block);
            }
            _ => {
                let mut block = ToolResultBlock::new(ToolResultBlockKind::Json);
                block.data = native_block.clone();
                push!(block);
            }
        }
    }

    let mut status = result_status(explicit_status.as_ref());
    if structured_envelope {
        blocks = blocks
            .into_iter()
            .zip(wrapper_flags.iter())
            .filter(|(_, wrapper)| !**wrapper)
            .map(|(block, _)| block)
            .collect();
    }
    if status == ToolResultStatus::Unknown
        && exit_code.is_none()
        && blocks.len() == 1
        && blocks[0].kind == ToolResultBlockKind::Text
    {
        if let Some(head) = UNIFIED_EXEC_HEAD.captures(&blocks[0].text) {
            match head.get(1) {
                Some(code) => exit_code = code.as_str().parse::<i64>().ok(),
                None => status = ToolResultStatus::Running,
            }
        }
    }
    if status == ToolResultStatus::Unknown {
        if let Some(code) = exit_code {
            status = if code == 0 {
                ToolResultStatus::Success
            } else {
                ToolResultStatus::Error
            };
        }
    }
    if stderr.as_ref().is_some_and(|text| !text.is_empty()) && status == ToolResultStatus::Unknown {
        status = ToolResultStatus::Error;
    }
    ToolResult {
        status,
        blocks,
        stdout,
        stderr,
        exit_code,
        truncated,
        attachments,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn structured_envelopes_populate_streams_and_status() {
        let result = parse_result(&json!([{
            "type": "input_text",
            "text": "{\"exit_code\":0,\"output\":\"hi\\n\"}",
        }]));
        assert_eq!(result.status, ToolResultStatus::Success);
        assert_eq!(result.exit_code, Some(0));
        assert_eq!(result.stdout.as_deref(), Some("hi\n"));
        assert_eq!(result.blocks.len(), 1);
        assert_eq!(result.blocks[0].text, "hi\n");
    }

    #[test]
    fn explicit_status_wins_over_exit_code() {
        let result = parse_result(&json!([{
            "type": "input_text",
            "text": "{\"status\":\"interrupted\",\"exit_code\":1}",
        }]));
        assert_eq!(result.status, ToolResultStatus::Interrupted);
        assert_eq!(result.exit_code, Some(1));
        // 未知 status 字面量降级成 unknown，随后走 exit_code 推导。
        let unknown = parse_result(&json!([{
            "type": "input_text",
            "text": "{\"status\":\"weird\",\"exit_code\":3}",
        }]));
        assert_eq!(unknown.status, ToolResultStatus::Error);
    }

    #[test]
    fn script_completed_wrappers_drop_only_with_a_structured_envelope() {
        let wrapped = json!([
            {"type": "input_text", "text": "Script completed\nWall time 0.1 seconds\nOutput:\n"},
            {"type": "input_text", "text": "{\"output\":\"done\"}"},
        ]);
        let result = parse_result(&wrapped);
        assert_eq!(result.blocks.len(), 1);
        assert_eq!(result.blocks[0].text, "done");

        // 没有信封时包装块保留。
        let plain = json!([
            {"type": "input_text", "text": "Script completed\nWall time 0.1 seconds\nOutput:\n"},
        ]);
        assert_eq!(parse_result(&plain).blocks.len(), 1);
    }

    #[test]
    fn unified_exec_headers_recover_exit_code_and_running() {
        let exited = parse_result(&json!(
            "Chunk ID: abc\nWall time: 0.0000 seconds\nProcess exited with code 128\nboom"
        ));
        assert_eq!(exited.exit_code, Some(128));
        assert_eq!(exited.status, ToolResultStatus::Error);

        let running = parse_result(&json!(
            "Chunk ID: abc\nWall time: 0.5 seconds\nProcess running with session ID s1\n"
        ));
        assert_eq!(running.status, ToolResultStatus::Running);
        assert_eq!(running.exit_code, None);
    }

    #[test]
    fn stderr_alone_implies_an_error() {
        let result = parse_result(&json!([{
            "type": "input_text",
            "text": "{\"stderr\":\"boom\"}",
        }]));
        assert_eq!(result.status, ToolResultStatus::Error);
        assert_eq!(result.stderr.as_deref(), Some("boom"));
        assert!(result.blocks.is_empty());
    }

    #[test]
    fn rich_blocks_survive_without_flattening() {
        let result = parse_result(&json!([
            {"type": "image", "image_url": "data:image/png;base64,QQ==", "mime_type": "image/png"},
            {"type": "file", "url": "file:///a", "filename": "a.txt"},
            {"type": "weird", "x": 1},
            42,
        ]));
        let kinds: Vec<ToolResultBlockKind> =
            result.blocks.iter().map(|block| block.kind).collect();
        assert_eq!(
            kinds,
            [
                ToolResultBlockKind::Image,
                ToolResultBlockKind::File,
                ToolResultBlockKind::Json,
                ToolResultBlockKind::Json,
            ]
        );
        assert_eq!(result.status, ToolResultStatus::Unknown);
    }

    #[test]
    fn plain_strings_become_a_single_text_block() {
        let result = parse_result(&json!("/fixture/codex/tools"));
        assert_eq!(result.blocks.len(), 1);
        assert_eq!(result.blocks[0].text, "/fixture/codex/tools");
        assert_eq!(result.status, ToolResultStatus::Unknown);
    }

    #[test]
    fn non_string_outputs_become_json_blocks() {
        let result = parse_result(&json!([{
            "type": "input_text",
            "text": "{\"output\":{\"a\":1},\"status\":\"success\"}",
        }]));
        assert_eq!(result.blocks.len(), 1);
        assert_eq!(result.blocks[0].kind, ToolResultBlockKind::Json);
        assert_eq!(result.blocks[0].data, json!({"a": 1}));
    }
}
