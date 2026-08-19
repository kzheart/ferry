//! Ferry 支持的唯一一套 Cursor 原生记录结构。
//!
//! 三类记录：`composerHeaders.value`（head）、`composerData:<id>`、
//! `bubbleId:<composerId>:<bubbleId>`。
//!
//! Cursor 每条记录都带 60+ 个字段，绝大多数是恒空的 UI 状态；这里只声明解析用
//! 得到的部分，其余靠 serde 的默认「忽略未知字段」丢掉。`composerData._v` 在
//! 本机同时存在 16 与 17，两版差异全在 UI 状态字段上，核心字段语义一致，所以
//! **不按 _v 分支**：一律 `Option` + `serde(default)` 容错。

use serde::Deserialize;
use serde_json::Value;

/// VS Code 序列化的 URI（`{$mid, scheme, path, fsPath, external, ...}`）。
///
/// `external` 是 percent-encoded 的，中文路径必须走 `fsPath` / `path`。
#[derive(Clone, Debug, Default, Deserialize)]
pub struct NativeUri {
    #[serde(default, rename = "fsPath")]
    pub fs_path: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
}

impl NativeUri {
    pub fn local_path(&self) -> Option<&str> {
        self.fs_path
            .as_deref()
            .or(self.path.as_deref())
            .filter(|value| !value.is_empty())
    }
}

/// 工作区标识。`uri` 整体可缺失——「未打开文件夹」的空窗口没有路径。
#[derive(Clone, Debug, Default, Deserialize)]
pub struct WorkspaceIdentifier {
    #[serde(default)]
    pub uri: Option<NativeUri>,
}

/// 子代理会话回指父会话的那次工具调用。
#[derive(Clone, Debug, Default, Deserialize)]
pub struct SubagentInfo {
    #[serde(default, rename = "parentComposerId")]
    pub parent_composer_id: Option<String>,
    #[serde(default, rename = "subagentTypeName")]
    pub subagent_type_name: Option<String>,
    #[serde(default, rename = "toolCallId")]
    pub tool_call_id: Option<String>,
}

/// `composerHeaders.value`（`type == "head"`）。
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Head {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default, rename = "unifiedMode")]
    pub unified_mode: Option<String>,
    #[serde(default, rename = "workspaceIdentifier")]
    pub workspace_identifier: Option<WorkspaceIdentifier>,
    #[serde(default, rename = "subagentInfo")]
    pub subagent_info: Option<SubagentInfo>,
}

/// `composerData.modelConfig`。
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ModelConfig {
    #[serde(default, rename = "modelName")]
    pub model_name: Option<String>,
}

/// `fullConversationHeadersOnly` 的一条：会话里**唯一权威**的消息顺序与存活标记。
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ConversationHeader {
    #[serde(default, rename = "bubbleId")]
    pub bubble_id: String,
}

/// `composerData:<composerId>`。
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ComposerData {
    #[serde(default, rename = "fullConversationHeadersOnly")]
    pub headers: Vec<ConversationHeader>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, rename = "modelConfig")]
    pub model_config: Option<ModelConfig>,
    #[serde(default, rename = "workspaceIdentifier")]
    pub workspace_identifier: Option<WorkspaceIdentifier>,
    #[serde(default, rename = "contextTokensUsed")]
    pub context_tokens_used: Option<i64>,
    #[serde(default, rename = "subagentComposerIds")]
    pub subagent_composer_ids: Vec<String>,
}

impl ComposerData {
    pub fn model(&self) -> Option<&str> {
        self.model_config
            .as_ref()
            .and_then(|config| config.model_name.as_deref())
            .filter(|value| !value.is_empty())
    }
}

/// `bubble.thinking`：两种落位（`{text, signature}` 或裸字符串）都必须认。
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum Thinking {
    Structured {
        #[serde(default)]
        text: String,
    },
    Text(String),
}

impl Thinking {
    pub fn text(&self) -> &str {
        match self {
            Self::Structured { text } => text,
            Self::Text(text) => text,
        }
    }
}

/// 工具调用与结果同住一条 bubble：Cursor 不做 call/result 配对。
///
/// `raw_args` / `params` / `result` 是**内嵌 JSON 字符串**，需要二次解析；
/// `additional_data` 是真对象。
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ToolFormerData {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default, rename = "toolCallId")]
    pub tool_call_id: Option<String>,
    #[serde(default, rename = "rawArgs")]
    pub raw_args: Option<String>,
    #[serde(default)]
    pub params: Option<String>,
    #[serde(default)]
    pub result: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default, rename = "additionalData")]
    pub additional_data: Value,
}

/// `bubbleId:<composerId>:<bubbleId>`（`_v` 恒为 3）。
///
/// `type`：1 = user，2 = assistant。`capabilityType`：15 工具 / 30 思考 /
/// 22 上下文压缩标记 / 缺失 = 纯文本。
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Bubble {
    #[serde(default, rename = "type")]
    pub kind: i64,
    #[serde(default, rename = "capabilityType")]
    pub capability_type: Option<i64>,
    #[serde(default)]
    pub text: String,
    #[serde(default, rename = "createdAt")]
    pub created_at: Option<String>,
    #[serde(default)]
    pub thinking: Option<Thinking>,
    #[serde(default, rename = "toolFormerData")]
    pub tool_former_data: Option<ToolFormerData>,
    #[serde(default, rename = "errorDetails")]
    pub error_details: Option<Value>,
}

/// 工具 bubble 的 capabilityType。
pub const CAPABILITY_TOOL: i64 = 15;
/// 思考 bubble 的 capabilityType。
pub const CAPABILITY_THINKING: i64 = 30;
/// 上下文压缩标记的 capabilityType。
pub const CAPABILITY_COMPACTION: i64 = 22;

/// 解析内嵌 JSON 字符串；解不开时保留原始字符串而不是丢掉整条记录。
pub fn embedded_json(raw: Option<&str>) -> Option<Value> {
    let text = raw?;
    if text.is_empty() {
        return None;
    }
    Some(serde_json::from_str(text).unwrap_or_else(|_| Value::from(text)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn thinking_accepts_both_native_shapes() {
        let structured: Bubble =
            serde_json::from_value(json!({"type": 2, "thinking": {"text": "a", "signature": "s"}}))
                .unwrap();
        assert_eq!(structured.thinking.unwrap().text(), "a");
        let bare: Bubble = serde_json::from_value(json!({"type": 2, "thinking": "b"})).unwrap();
        assert_eq!(bare.thinking.unwrap().text(), "b");
        let absent: Bubble = serde_json::from_value(json!({"type": 2})).unwrap();
        assert!(absent.thinking.is_none());
    }

    #[test]
    fn v16_and_v17_composer_data_parse_through_the_same_model() {
        for version in [16, 17] {
            let data: ComposerData = serde_json::from_value(json!({
                "_v": version,
                "fullConversationHeadersOnly": [{"bubbleId": "b1", "type": 1}],
                "modelConfig": {"modelName": "model-xxxxxx"},
                "activeCustomMode": null,
                "bestOfNJudgeWinner": {"unknown": true},
            }))
            .unwrap();
            assert_eq!(data.headers.len(), 1);
            assert_eq!(data.model(), Some("model-xxxxxx"));
        }
    }

    #[test]
    fn a_workspace_without_a_folder_has_no_path() {
        let head: Head =
            serde_json::from_value(json!({"workspaceIdentifier": {"id": "1783251917755"}}))
                .unwrap();
        assert!(head
            .workspace_identifier
            .unwrap()
            .uri
            .is_none_or(|uri| uri.local_path().is_none()));
    }

    #[test]
    fn embedded_json_keeps_unparsable_payloads_as_text() {
        assert_eq!(embedded_json(Some("{\"a\": 1}")), Some(json!({"a": 1})));
        assert_eq!(embedded_json(Some("{broken")), Some(json!("{broken")));
        assert_eq!(embedded_json(Some("")), None);
        assert_eq!(embedded_json(None), None);
    }
}
