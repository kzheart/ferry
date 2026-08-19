//! 会话规范化中间格式（canonical model）。
//!
//! Canonical Model 只保存 Ferry 使用的明确语义；原生记录由各 Adapter 在边界内
//! 处理，无法表达的内容通过迁移损失报告（`Session::lose`）。

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::events::Event;

/// 结构化工具结果的状态。对齐 `TOOL_RESULT_STATUSES`：非法值不可构造。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolResultStatus {
    Success,
    Error,
    Interrupted,
    Running,
    Pending,
    #[default]
    Unknown,
}

/// 工具结果里的一个结构化块。对齐 `TOOL_RESULT_BLOCK_KINDS`。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolResultBlockKind {
    Text,
    Json,
    Image,
    File,
    ToolReference,
}

/// 消息块的种类：text | thinking | tool | image。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind {
    Text,
    Thinking,
    Tool,
    Image,
}

/// 原生时间戳：字符串或 epoch 毫秒整数（Python `str | int | None`）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Timestamp {
    Millis(i64),
    Text(String),
}

/// One structured block emitted by a tool result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolResultBlock {
    pub kind: ToolResultBlockKind,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub data: Value,
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default)]
    pub uri: Option<String>,
}

impl ToolResultBlock {
    pub fn new(kind: ToolResultBlockKind) -> Self {
        Self {
            kind,
            text: String::new(),
            data: Value::Null,
            mime_type: None,
            filename: None,
            uri: None,
        }
    }

    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::new(ToolResultBlockKind::Text)
        }
    }

    /// Project a block to text for text-only consumers.
    ///
    /// json 块的兜底投影用 `ensure_ascii=False` + 紧凑分隔符，但**不排序 key**
    /// （与 canonical_json 不同，这里保留插入序）。
    pub fn text_projection(&self) -> String {
        if !self.text.is_empty() {
            return self.text.clone();
        }
        if self.kind == ToolResultBlockKind::Json && !self.data.is_null() {
            return serde_json::to_string(&self.data).unwrap_or_default();
        }
        String::new()
    }
}

/// Canonical result with explicit status, streams and non-text blocks.
///
/// `exit_code` 在 Python 侧显式拒绝 bool；Rust 的 `Option<i64>` 反序列化天然
/// 拒绝 JSON `true/false`（invalid type: boolean），语义等价。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    #[serde(default)]
    pub status: ToolResultStatus,
    #[serde(default)]
    pub blocks: Vec<ToolResultBlock>,
    #[serde(default)]
    pub stdout: Option<String>,
    #[serde(default)]
    pub stderr: Option<String>,
    #[serde(default)]
    pub exit_code: Option<i64>,
    #[serde(default)]
    pub truncated: Option<bool>,
    #[serde(default)]
    pub attachments: Vec<Value>,
}

impl ToolResult {
    pub fn new(status: ToolResultStatus) -> Self {
        Self {
            status,
            ..Self::default()
        }
    }
}

/// Project a structured result to text without creating another data source.
pub fn tool_result_text(result: Option<&ToolResult>) -> String {
    let Some(result) = result else {
        return String::new();
    };
    result
        .blocks
        .iter()
        .map(ToolResultBlock::text_projection)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Build a structured result for native tools that emit plain text.
pub fn text_tool_result(text: &str, status: ToolResultStatus) -> ToolResult {
    ToolResult {
        status,
        blocks: if text.is_empty() {
            Vec::new()
        } else {
            vec![ToolResultBlock::text(text)]
        },
        ..ToolResult::default()
    }
}

/// 一次工具调用。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    /// 源工具名（Bash / exec / bash ...）。
    pub name: String,
    /// 规范操作（`shell.exec` 等）；None = 无映射，降级处理。
    #[serde(default)]
    pub op: Option<String>,
    /// 源参数（已解析）。Python 标注是 `dict | str`，但运行期不校验，
    /// 为了 golden 对照的逐字段保真这里用 `Value` 原样承载。
    pub input: Value,
    #[serde(default)]
    pub result: Option<ToolResult>,
    #[serde(default)]
    pub source_call_id: Option<String>,
    #[serde(default)]
    pub source_result_id: Option<String>,
    #[serde(default)]
    pub source_message_id: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub started_at: Option<Timestamp>,
    #[serde(default)]
    pub ended_at: Option<Timestamp>,
}

impl ToolCall {
    pub fn new(name: impl Into<String>, op: Option<String>, input: Value) -> Self {
        Self {
            name: name.into(),
            op,
            input,
            result: None,
            source_call_id: None,
            source_result_id: None,
            source_message_id: None,
            agent_id: None,
            started_at: None,
            ended_at: None,
        }
    }
}

/// 规范图片块的私有源数据；DTO 仅暴露 id 与元数据。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageAsset {
    pub id: String,
    pub mime_type: String,
    pub data: String,
    #[serde(default)]
    pub filename: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Block {
    pub kind: BlockKind,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub tool: Option<ToolCall>,
    #[serde(default)]
    pub image: Option<ImageAsset>,
}

impl Block {
    pub fn new(kind: BlockKind) -> Self {
        Self {
            kind,
            text: String::new(),
            tool: None,
            image: None,
        }
    }

    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::new(BlockKind::Text)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    /// user | assistant（Python 侧不做运行期校验，这里同样保持开放）。
    pub role: String,
    #[serde(default)]
    pub blocks: Vec<Block>,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub parent_ids: Vec<String>,
    #[serde(default)]
    pub turn_id: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub created_at: Option<Timestamp>,
}

impl Message {
    pub fn new(role: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            blocks: Vec::new(),
            source_id: None,
            parent_ids: Vec::new(),
            turn_id: None,
            agent_id: None,
            created_at: None,
        }
    }
}

/// 消息在原生会话里的定位串；没有 source_id 时退回序号。
pub fn native_locator(message: &Message, index: usize) -> String {
    match &message.source_id {
        Some(source_id) if !source_id.is_empty() => source_id.clone(),
        _ => format!("index:{index}"),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContextCompaction {
    pub id: String,
    pub source: String,
    #[serde(default)]
    pub after_message_id: Option<String>,
    #[serde(default)]
    pub event_locator: Option<String>,
    #[serde(default)]
    pub created_at: Option<Timestamp>,
    #[serde(default = "unknown_marker")]
    pub trigger: String,
    #[serde(default = "completed_marker")]
    pub state: String,
    #[serde(default = "missing_marker")]
    pub summary_status: String,
    #[serde(default)]
    pub summary_text: String,
    #[serde(default)]
    pub summary_message_id: Option<String>,
    #[serde(default = "unknown_marker")]
    pub tail_status: String,
    #[serde(default)]
    pub tail_start_locator: Option<String>,
    #[serde(default)]
    pub tail_start_message_index: Option<i64>,
    #[serde(default)]
    pub metrics: Map<String, Value>,
    #[serde(default)]
    pub source_meta: Map<String, Value>,
}

fn unknown_marker() -> String {
    "unknown".to_string()
}

fn completed_marker() -> String {
    "completed".to_string()
}

fn missing_marker() -> String {
    "missing".to_string()
}

impl ContextCompaction {
    pub fn new(id: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            source: source.into(),
            after_message_id: None,
            event_locator: None,
            created_at: None,
            trigger: unknown_marker(),
            state: completed_marker(),
            summary_status: missing_marker(),
            summary_text: String::new(),
            summary_message_id: None,
            tail_status: unknown_marker(),
            tail_start_locator: None,
            tail_start_message_index: None,
            metrics: Map::new(),
            source_meta: Map::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentEdge {
    pub parent_session_id: String,
    pub child_session_id: String,
    #[serde(default)]
    pub source_call_id: Option<String>,
    #[serde(default)]
    pub spawn_message_id: Option<String>,
    #[serde(default)]
    pub result_message_id: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub agent_path: Option<String>,
    #[serde(default)]
    pub agent_type: Option<String>,
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default = "explicit_marker")]
    pub association: String,
    #[serde(default = "full_confidence")]
    pub confidence: f64,
}

fn explicit_marker() -> String {
    "explicit".to_string()
}

fn full_confidence() -> f64 {
    1.0
}

impl AgentEdge {
    /// `__post_init__`：confidence 必须落在 [0, 1]。
    pub fn new(parent_session_id: impl Into<String>, child_session_id: impl Into<String>) -> Self {
        Self {
            parent_session_id: parent_session_id.into(),
            child_session_id: child_session_id.into(),
            source_call_id: None,
            spawn_message_id: None,
            result_message_id: None,
            agent_id: None,
            agent_path: None,
            agent_type: None,
            prompt: String::new(),
            status: None,
            association: explicit_marker(),
            confidence: full_confidence(),
        }
    }

    /// 校验 confidence 区间；对齐 Python 的 `__post_init__` 断言。
    pub fn validate(&self) -> Result<(), &'static str> {
        if (0.0..=1.0).contains(&self.confidence) {
            Ok(())
        } else {
            Err("agent edge confidence must be between 0 and 1")
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub source_tool: String,
    pub source_id: String,
    pub cwd: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub messages: Vec<Message>,
    #[serde(default)]
    pub loss: Vec<Event>,
    #[serde(default)]
    pub root_id: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub forked_from_id: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub agent_path: Option<String>,
    #[serde(default)]
    pub agent_type: Option<String>,
    #[serde(default)]
    pub agent_nickname: Option<String>,
    #[serde(default)]
    pub agent_role: Option<String>,
    #[serde(default)]
    pub model_provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub depth: Option<i64>,
    #[serde(default)]
    pub parent_association: Option<String>,
    #[serde(default)]
    pub children: Vec<Session>,
    #[serde(default)]
    pub agent_edges: Vec<AgentEdge>,
    #[serde(default)]
    pub context_compactions: Vec<ContextCompaction>,
}

impl Session {
    pub fn new(
        source_tool: impl Into<String>,
        source_id: impl Into<String>,
        cwd: impl Into<String>,
    ) -> Self {
        Self {
            source_tool: source_tool.into(),
            source_id: source_id.into(),
            cwd: cwd.into(),
            title: String::new(),
            messages: Vec::new(),
            loss: Vec::new(),
            root_id: None,
            parent_id: None,
            forked_from_id: None,
            agent_id: None,
            agent_path: None,
            agent_type: None,
            agent_nickname: None,
            agent_role: None,
            model_provider: None,
            model: None,
            depth: None,
            parent_association: None,
            children: Vec::new(),
            agent_edges: Vec::new(),
            context_compactions: Vec::new(),
        }
    }

    /// 记一条保真度损耗（对应 `Session.lose`）。
    pub fn lose(&mut self, code: &str, params: Map<String, Value>) {
        self.loss.push(Event::new(code, params));
    }

    /// 前序遍历自身与全部子会话，顺序与 Python `walk()` 逐条一致。
    pub fn walk(&self) -> Vec<&Session> {
        let mut visited = Vec::new();
        let mut stack = vec![self];
        while let Some(session) = stack.pop() {
            visited.push(session);
            stack.extend(session.children.iter().rev());
        }
        visited
    }

    pub fn message_count(&self) -> usize {
        self.walk()
            .iter()
            .map(|session| session.messages.len())
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_result_text_joins_non_empty_projections() {
        let mut result = ToolResult::new(ToolResultStatus::Success);
        result.blocks.push(ToolResultBlock::text("hello"));
        result
            .blocks
            .push(ToolResultBlock::new(ToolResultBlockKind::Image));
        let mut json_block = ToolResultBlock::new(ToolResultBlockKind::Json);
        json_block.data = json!({"b": 1, "a": "中文"});
        result.blocks.push(json_block);
        assert_eq!(
            tool_result_text(Some(&result)),
            "hello\n{\"b\":1,\"a\":\"中文\"}"
        );
        assert_eq!(tool_result_text(None), "");
    }

    #[test]
    fn exit_code_rejects_booleans_on_deserialize() {
        let ok: ToolResult = serde_json::from_value(json!({"exit_code": 0})).unwrap();
        assert_eq!(ok.exit_code, Some(0));
        assert!(serde_json::from_value::<ToolResult>(json!({"exit_code": true})).is_err());
    }

    #[test]
    fn strict_enums_reject_unknown_values() {
        assert!(serde_json::from_value::<ToolResult>(json!({"status": "weird"})).is_err());
        assert!(serde_json::from_value::<Block>(json!({"kind": "weird"})).is_err());
        let block: ToolResultBlock =
            serde_json::from_value(json!({"kind": "tool_reference"})).unwrap();
        assert_eq!(block.kind, ToolResultBlockKind::ToolReference);
    }

    #[test]
    fn walk_is_preorder_and_counts_every_message() {
        let mut root = Session::new("claude", "root", "/tmp");
        root.messages.push(Message::new("user"));
        let mut first = Session::new("claude", "a", "/tmp");
        first.messages.push(Message::new("assistant"));
        let mut grandchild = Session::new("claude", "a1", "/tmp");
        grandchild.messages.push(Message::new("user"));
        first.children.push(grandchild);
        let second = Session::new("claude", "b", "/tmp");
        root.children.push(first);
        root.children.push(second);

        let ids: Vec<&str> = root
            .walk()
            .iter()
            .map(|session| session.source_id.as_str())
            .collect();
        assert_eq!(ids, ["root", "a", "a1", "b"]);
        assert_eq!(root.message_count(), 3);
    }

    #[test]
    fn native_locator_falls_back_to_the_index() {
        let mut message = Message::new("user");
        assert_eq!(native_locator(&message, 4), "index:4");
        message.source_id = Some(String::new());
        assert_eq!(native_locator(&message, 4), "index:4");
        message.source_id = Some("uuid-1".into());
        assert_eq!(native_locator(&message, 4), "uuid-1");
    }

    #[test]
    fn agent_edge_confidence_stays_in_range() {
        let mut edge = AgentEdge::new("parent", "child");
        assert!(edge.validate().is_ok());
        edge.confidence = 1.5;
        assert!(edge.validate().is_err());
    }

    #[test]
    fn timestamps_round_trip_both_shapes() {
        let message: Message =
            serde_json::from_value(json!({"role": "user", "created_at": 1700000000000i64}))
                .unwrap();
        assert_eq!(message.created_at, Some(Timestamp::Millis(1700000000000)));
        let textual: Message =
            serde_json::from_value(json!({"role": "user", "created_at": "2024-01-01"})).unwrap();
        assert_eq!(
            textual.created_at,
            Some(Timestamp::Text("2024-01-01".into()))
        );
    }
}
