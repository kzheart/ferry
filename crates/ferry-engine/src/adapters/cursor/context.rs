//! 上下文层：`agentKv:blob:<sha256hex>` 里那串「模型真正看到的」消息。
//!
//! 展示层（bubble）决定 UI 里看到什么，上下文层决定模型看到什么，两层互相独立
//! （`docs/cursor-migration-target.md` §1）。只写展示层的会话在 UI 上完美，但模型
//! 会说「这是我们第一次对话」——迁移必须两层都写。
//!
//! 三条硬约束：
//! 1. blob 是**内容寻址**的：键 = `sha256(value 原始字节)` 的小写 hex，写什么字节
//!    就对什么字节算摘要。`serde_json::to_vec` 的紧凑、非 ASCII 不转义形态即所需。
//! 2. **不构造 system / user_info blob**：Cursor 续聊时会自动前置当前模型的
//!    system prompt，自己造只会重复或过时（§5.2 V2 实测）。
//! 3. tool-call 与 tool-result 必须成对且 `toolCallId` 一致，顺序为
//!    `assistant(tool-call)` → `tool(tool-result)`。

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::ids;

/// 上下文层里 Shell 工具对模型暴露的名字。
///
/// 与展示层的内部工具名 `run_terminal_command_v2` 不同，两处不可互换（§4.6）。
pub const SHELL_TOOL_NAME: &str = "Shell";

/// assistant 消息的 `id` 字段，实测常量 `"1"` 可用。
const ASSISTANT_ID: &str = "1";

/// 一条待落库的内容寻址 blob。
#[derive(Clone, Debug)]
pub struct Blob {
    /// `agentKv:blob:<sha256hex>`。
    pub key: String,
    /// 写进 `value` 列的原始字节。
    pub bytes: Vec<u8>,
    /// `conversationState` 的 f1 用的 32 字节摘要。
    pub digest: [u8; 32],
}

/// 把一条消息编成 blob。
pub fn blob(message: &Value) -> Blob {
    // 紧凑分隔符 + 非 ASCII 原样输出，正是 serde_json 的默认形态。
    let bytes = serde_json::to_vec(message).expect("消息 JSON 可序列化");
    let digest: [u8; 32] = Sha256::digest(&bytes).into();
    Blob {
        key: format!("agentKv:blob:{}", ids::hex_lower(&digest)),
        bytes,
        digest,
    }
}

fn text_content(text: &str) -> Value {
    let mut block = Map::new();
    block.insert("type".into(), Value::from("text"));
    block.insert("text".into(), Value::from(text));
    Value::Array(vec![Value::Object(block)])
}

/// `anthropicNativeContent` 是 content 数组的 JSON **字符串**（双重编码）。
fn native_content(blocks: &Value) -> Value {
    Value::from(serde_json::to_string(blocks).expect("原生内容可序列化"))
}

fn provider_options(entries: Map<String, Value>) -> Value {
    let mut cursor = Map::new();
    cursor.insert("cursor".into(), Value::Object(entries));
    Value::Object(cursor)
}

/// 历史 user 消息。`timestamp` 是人读文案，包装形态照抄 Cursor 的约定。
pub fn user_message(text: &str, timestamp: &str) -> Value {
    let wrapped =
        format!("<timestamp>{timestamp}</timestamp>\n<user_query>\n{text}\n</user_query>");
    let mut options = Map::new();
    options.insert("requestId".into(), Value::from(ids::uuid4()));
    let mut message = Map::new();
    message.insert("role".into(), Value::from("user"));
    message.insert("content".into(), text_content(&wrapped));
    message.insert("providerOptions".into(), provider_options(options));
    Value::Object(message)
}

/// 纯文本 assistant 消息。
pub fn assistant_text(text: &str) -> Value {
    let content = text_content(text);
    let mut options = Map::new();
    options.insert("anthropicNativeContent".into(), native_content(&content));
    let mut message = Map::new();
    message.insert("role".into(), Value::from("assistant"));
    message.insert("content".into(), content);
    message.insert("id".into(), Value::from(ASSISTANT_ID));
    message.insert("providerOptions".into(), provider_options(options));
    Value::Object(message)
}

/// assistant 发起的工具调用。
pub fn assistant_tool_call(tool_call_id: &str, tool_name: &str, args: &Value) -> Value {
    let mut block = Map::new();
    block.insert("type".into(), Value::from("tool-call"));
    block.insert("toolCallId".into(), Value::from(tool_call_id));
    block.insert("toolName".into(), Value::from(tool_name));
    block.insert("args".into(), args.clone());
    let content = Value::Array(vec![Value::Object(block)]);

    let mut native = Map::new();
    native.insert("type".into(), Value::from("tool_use"));
    native.insert("id".into(), Value::from(tool_call_id));
    native.insert("name".into(), Value::from(tool_name));
    native.insert("input".into(), args.clone());
    let mut caller = Map::new();
    caller.insert("type".into(), Value::from("direct"));
    native.insert("caller".into(), Value::Object(caller));

    let mut options = Map::new();
    options.insert(
        "anthropicNativeContent".into(),
        native_content(&Value::Array(vec![Value::Object(native)])),
    );
    let mut message = Map::new();
    message.insert("role".into(), Value::from("assistant"));
    message.insert("content".into(), content);
    message.insert("id".into(), Value::from(ASSISTANT_ID));
    message.insert("providerOptions".into(), provider_options(options));
    Value::Object(message)
}

/// 工具返回。`id` 字段等于 `toolCallId`（不是 assistant 的 `"1"`）。
///
/// `highLevelToolCallResult` 是工具专属的结构化数据，实测只填 `output.success`
/// 的几个字段模型也能正确读到结果；失败的调用同样走这条形态，失败信息在
/// `result` 正文里（展示层另有 `status: "error"`）。
pub fn tool_result(tool_call_id: &str, tool_name: &str, command: &str, output: &str) -> Value {
    let mut block = Map::new();
    block.insert("type".into(), Value::from("tool-result"));
    block.insert("toolCallId".into(), Value::from(tool_call_id));
    block.insert("toolName".into(), Value::from(tool_name));
    block.insert("result".into(), Value::from(output));
    block.insert("experimental_content".into(), text_content(output));

    let mut success = Map::new();
    success.insert("command".into(), Value::from(command));
    success.insert("stdout".into(), Value::from(output));
    success.insert("executionTime".into(), Value::from(0));
    success.insert("interleavedOutput".into(), Value::from(output));
    success.insert("localExecutionTimeMs".into(), Value::from(0));
    let mut outcome = Map::new();
    outcome.insert("success".into(), Value::Object(success));
    let mut high_level = Map::new();
    high_level.insert("output".into(), Value::Object(outcome));
    let mut options = Map::new();
    options.insert("highLevelToolCallResult".into(), Value::Object(high_level));

    let mut message = Map::new();
    message.insert("role".into(), Value::from("tool"));
    message.insert("content".into(), Value::Array(vec![Value::Object(block)]));
    message.insert("id".into(), Value::from(tool_call_id));
    message.insert("providerOptions".into(), provider_options(options));
    Value::Object(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn blob_keys_are_the_sha256_of_the_bytes_actually_written() {
        let value = json!({"role": "user", "content": "中文原样"});
        let encoded = blob(&value);
        // 非 ASCII 不转义、分隔符无空格。
        let text = String::from_utf8(encoded.bytes.clone()).unwrap();
        assert_eq!(text, "{\"role\":\"user\",\"content\":\"中文原样\"}");
        let expected: [u8; 32] = Sha256::digest(text.as_bytes()).into();
        assert_eq!(encoded.digest, expected);
        assert_eq!(
            encoded.key,
            format!("agentKv:blob:{}", ids::hex_lower(&expected))
        );
        // 内容寻址：同样的内容必然同样的键。
        assert_eq!(blob(&value).key, encoded.key);
    }

    #[test]
    fn user_messages_keep_the_native_timestamp_and_query_wrapper() {
        let message = user_message("看看 README", "Wednesday, Aug 19, 2026, 11:10 PM (UTC+8)");
        let text = message["content"][0]["text"].as_str().unwrap();
        assert_eq!(
            text,
            "<timestamp>Wednesday, Aug 19, 2026, 11:10 PM (UTC+8)</timestamp>\n\
             <user_query>\n看看 README\n</user_query>"
        );
        assert_eq!(message["role"], json!("user"));
        assert_eq!(
            message["providerOptions"]["cursor"]["requestId"]
                .as_str()
                .unwrap()
                .len(),
            36
        );
    }

    #[test]
    fn assistant_native_content_is_a_doubly_encoded_string() {
        let message = assistant_text("回复");
        let native = message["providerOptions"]["cursor"]["anthropicNativeContent"]
            .as_str()
            .expect("必须是字符串");
        assert_eq!(native, "[{\"type\":\"text\",\"text\":\"回复\"}]");
        assert_eq!(message["id"], json!("1"));
    }

    #[test]
    fn tool_calls_and_results_pair_on_the_same_id() {
        let call = assistant_tool_call("toolu_x", SHELL_TOOL_NAME, &json!({"command": "ls"}));
        assert_eq!(call["content"][0]["type"], json!("tool-call"));
        assert_eq!(call["content"][0]["toolName"], json!("Shell"));
        let native: Value = serde_json::from_str(
            call["providerOptions"]["cursor"]["anthropicNativeContent"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(native[0]["type"], json!("tool_use"));
        assert_eq!(native[0]["id"], json!("toolu_x"));
        assert_eq!(native[0]["caller"], json!({"type": "direct"}));

        let result = tool_result("toolu_x", SHELL_TOOL_NAME, "ls", "a\nb");
        assert_eq!(result["role"], json!("tool"));
        // 结果消息的 id 是 toolCallId 而不是 "1"。
        assert_eq!(result["id"], json!("toolu_x"));
        assert_eq!(result["content"][0]["result"], json!("a\nb"));
        assert_eq!(
            result["content"][0]["experimental_content"],
            json!([{"type": "text", "text": "a\nb"}])
        );
        assert_eq!(
            result["providerOptions"]["cursor"]["highLevelToolCallResult"]["output"]["success"]
                ["command"],
            json!("ls")
        );
    }
}
