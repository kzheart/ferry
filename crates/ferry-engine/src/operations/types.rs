//! 会话编辑操作类型 + operations 能力包对外的窄端口。
//!
//! operations 不直接吃运行上下文那种胖对象：它需要的能力收窄成本文件里的两个
//! trait，只声明「我需要什么」，具体实现由组合根接线。

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{Map, Value};

use crate::adapters::contracts::AgentAdapter;
use crate::errors::{DomainError, DomainResult};
use crate::model::Session;

// ---------------------------------------------------------------------------
// 错误
// ---------------------------------------------------------------------------

/// Engine 内部结果类型。
///
/// Python 里除 `DomainError` 外还会抛裸 `RuntimeError`/`KeyError`/`sqlite3.*`，
/// 这些异常有两条可观察的语义：
/// 1. RPC 层折成 `internal.unexpected`（`FERRY_DEBUG` 时才带 `类名: 文本`）；
/// 2. operation 失败时 `error_type` 落库的就是**异常类名字符串**（§2.4 第 22 条）。
///
/// 所以 Rust 侧保留一个 `error_type` 字段，与 Python 类名逐字对齐。
#[derive(Clone, Debug, PartialEq)]
pub enum EngineError {
    Domain(DomainError),
    /// 非 DomainError 的内部异常；`error_type` 是 Python 异常类名。
    Internal {
        error_type: &'static str,
        message: String,
    },
}

pub type EngineResult<T> = Result<T, EngineError>;

impl EngineError {
    /// 等价 Python 的 `raise RuntimeError(message)`。
    pub fn runtime(message: impl Into<String>) -> Self {
        Self::Internal {
            error_type: "RuntimeError",
            message: message.into(),
        }
    }

    /// 深层 `KeyError`：调用方缺陷而非参数错误（§2.1 第 6 条）。
    pub fn key_error(key: impl Into<String>) -> Self {
        Self::Internal {
            error_type: "KeyError",
            message: key.into(),
        }
    }

    /// `json.dumps(allow_nan=False)` 的 `ValueError`。
    pub fn value_error(message: impl Into<String>) -> Self {
        Self::Internal {
            error_type: "ValueError",
            message: message.into(),
        }
    }

    /// 落库用的异常类名（`OperationStore::fail` 的 `error_type` 列）。
    pub fn error_type(&self) -> &str {
        match self {
            Self::Domain(error) => error.error_type,
            Self::Internal { error_type, .. } => error_type,
        }
    }

    /// 等价 Python 的 `str(error)`。
    pub fn message(&self) -> &str {
        match self {
            Self::Domain(error) => error.message(),
            Self::Internal { message, .. } => message,
        }
    }

    pub fn as_domain(&self) -> Option<&DomainError> {
        match self {
            Self::Domain(error) => Some(error),
            Self::Internal { .. } => None,
        }
    }

    /// 是否是 `ConcurrentModificationError`（编辑事务的快照还原分支要区分它）。
    pub fn is_concurrent_modification(&self) -> bool {
        self.error_type() == "ConcurrentModificationError"
    }
}

impl From<DomainError> for EngineError {
    fn from(error: DomainError) -> Self {
        Self::Domain(error)
    }
}

impl From<rusqlite::Error> for EngineError {
    fn from(error: rusqlite::Error) -> Self {
        // Python 侧对应 `sqlite3.OperationalError` 家族；这里统一取最常见的类名。
        Self::Internal {
            error_type: "OperationalError",
            message: error.to_string(),
        }
    }
}

impl From<crate::jsonutil::CanonicalJsonError> for EngineError {
    fn from(error: crate::jsonutil::CanonicalJsonError) -> Self {
        Self::value_error(error.to_string())
    }
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for EngineError {}

// ---------------------------------------------------------------------------
// AssistantReply 家族
// ---------------------------------------------------------------------------

/// 回复条目。等价 Python 的 `TextItem | ToolItem`。
#[derive(Clone, Debug, PartialEq)]
pub enum ReplyItem {
    Text {
        text: String,
    },
    Tool {
        name: String,
        /// `dict | str`，原样搬运。
        input: Value,
        output: String,
    },
}

impl ReplyItem {
    pub fn to_value(&self) -> Value {
        let mut payload = Map::new();
        match self {
            Self::Text { text } => {
                payload.insert("kind".into(), Value::from("text"));
                payload.insert("text".into(), Value::from(text.as_str()));
            }
            Self::Tool {
                name,
                input,
                output,
            } => {
                payload.insert("kind".into(), Value::from("tool"));
                payload.insert("name".into(), Value::from(name.as_str()));
                payload.insert("input".into(), input.clone());
                payload.insert("output".into(), Value::from(output.as_str()));
            }
        }
        Value::Object(payload)
    }
}

/// 一次助手回复的完整内容。
#[derive(Clone, Debug, PartialEq)]
pub struct AssistantReply {
    pub items: Vec<ReplyItem>,
}

fn keys_equal(object: &Map<String, Value>, expected: &[&str]) -> bool {
    object.len() == expected.len() && expected.iter().all(|key| object.contains_key(*key))
}

impl AssistantReply {
    /// 等价 `AssistantReply.from_dict`：文案与索引提示逐字保留。
    pub fn from_value(value: &Value) -> DomainResult<Self> {
        let object = value
            .as_object()
            .filter(|object| keys_equal(object, &["items"]))
            .ok_or_else(|| DomainError::invalid_reply("reply 必须且只能包含 items"))?;
        let raw_items = object["items"]
            .as_array()
            .filter(|items| !items.is_empty())
            .ok_or_else(|| DomainError::invalid_reply("reply.items 必须是非空数组"))?;
        let mut items = Vec::with_capacity(raw_items.len());
        for (index, raw) in raw_items.iter().enumerate() {
            let entry = raw.as_object().ok_or_else(|| {
                DomainError::invalid_reply(format!("reply.items[{index}] 必须是对象"))
            })?;
            match entry.get("kind").and_then(Value::as_str) {
                Some("text") => {
                    let text = entry
                        .get("text")
                        .and_then(Value::as_str)
                        .filter(|_| keys_equal(entry, &["kind", "text"]))
                        .ok_or_else(|| {
                            DomainError::invalid_reply(format!(
                                "reply.items[{index}] text 结构非法"
                            ))
                        })?;
                    if text.is_empty() {
                        return Err(DomainError::invalid_reply(format!(
                            "reply.items[{index}].text 不可为空"
                        )));
                    }
                    items.push(ReplyItem::Text {
                        text: text.to_string(),
                    });
                }
                Some("tool") => {
                    if !keys_equal(entry, &["kind", "name", "input", "output"]) {
                        return Err(DomainError::invalid_reply(format!(
                            "reply.items[{index}] tool 结构非法"
                        )));
                    }
                    let name = entry
                        .get("name")
                        .and_then(Value::as_str)
                        .filter(|name| !name.is_empty())
                        .ok_or_else(|| {
                            DomainError::invalid_reply(format!(
                                "reply.items[{index}].name 必须是非空字符串"
                            ))
                        })?;
                    let input = entry.get("input").cloned().unwrap_or(Value::Null);
                    if !input.is_object() && !input.is_string() {
                        return Err(DomainError::invalid_reply(format!(
                            "reply.items[{index}].input 必须是对象或字符串"
                        )));
                    }
                    let output = entry.get("output").and_then(Value::as_str).ok_or_else(|| {
                        DomainError::invalid_reply(format!(
                            "reply.items[{index}].output 必须是字符串"
                        ))
                    })?;
                    items.push(ReplyItem::Tool {
                        name: name.to_string(),
                        input,
                        output: output.to_string(),
                    });
                }
                _ => {
                    return Err(DomainError::invalid_reply(format!(
                        "reply.items[{index}].kind 仅支持 text/tool"
                    )));
                }
            }
        }
        Ok(Self { items })
    }

    pub fn to_value(&self) -> Value {
        let mut payload = Map::new();
        payload.insert(
            "items".into(),
            Value::Array(self.items.iter().map(ReplyItem::to_value).collect()),
        );
        Value::Object(payload)
    }
}

// ---------------------------------------------------------------------------
// 端口：索引与运行环境
// ---------------------------------------------------------------------------

/// 索引解析出来的一条会话记录。等价 `AgentSessionIndex.resolve` 的返回值。
#[derive(Clone, Debug, PartialEq)]
pub struct IndexedSession {
    pub tool: String,
    /// 对外 opaque ref（`fsr_...`）。
    pub opaque_ref: String,
    /// adapter 内部的原生引用。
    pub canonical_ref: String,
    pub revision: String,
    /// 扫描行；`id` / `title` / `dir` / `size` / `updated` 是本包会读的键。
    pub row: Map<String, Value>,
}

impl IndexedSession {
    /// 等价 `sessions.safety.record_session_id(record)` 在 operations 内的用法：
    /// 取 `row["id"]`，转字符串后按 512 字符截断。
    ///
    /// Python 是 `str(value or "")`：**falsy**（`None`/`False`/`0`/`""`/`[]`）
    /// 一律落成空串，其余走 `str()` 的 Python 语义（`True` 而不是 `true`，
    /// 见 [`crate::jsonutil::python_str`]）。
    pub fn session_id(&self) -> String {
        let raw = match self.row.get("id") {
            Some(value) if crate::jsonutil::python_truthy(value) => {
                crate::jsonutil::python_str(value)
            }
            _ => String::new(),
        };
        raw.chars().take(512).collect()
    }
}

/// `resolve_message_locator` 的返回值。
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedMessageLocator {
    pub native_locator: String,
    pub editable: bool,
}

/// operations 对 `sessions::index` 的窄端口（WP-D 提供实现，WP-E 接线）。
pub trait SessionResolver: Send + Sync {
    /// 等价 `AgentSessionIndex.resolve(tool, ref)`。
    fn resolve(&self, tool: &str, reference: &str) -> DomainResult<IndexedSession>;

    /// 等价 `AgentSessionIndex.resolve_message_locator(record, locator)`。
    fn resolve_message_locator(
        &self,
        record: &IndexedSession,
        locator: &str,
    ) -> DomainResult<ResolvedMessageLocator>;

    /// 等价 `sessions.agent_read.read_indexed_session(index, record)`。
    fn read_indexed_session(&self, record: &IndexedSession) -> DomainResult<Session>;
}

/// operations 对 `EngineContext` 的窄端口（WP-E 提供实现）。
pub trait OperationPorts: Send + Sync {
    /// 等价 `ports.adapter(tool)`；未知工具返回 `tool.unknown`。
    ///
    /// 返回克隆值：`AgentAdapter` 的组件都在 `Arc` 后面，克隆是廉价的，
    /// 而按引用返回会把生命周期传染到每一个调用点。
    fn adapter(&self, tool: &str) -> DomainResult<AgentAdapter>;

    /// 等价 `ports.adapters()`：装配顺序的 adapter id 列表。
    fn adapters(&self) -> Vec<String>;

    /// 等价 `ports.state_dir()`：`ferry-state.sqlite3` 所在目录。
    fn state_dir(&self) -> PathBuf;
}

/// 便于在测试与组合根之间搬运的别名。
pub type Ports = Arc<dyn OperationPorts>;
pub type Resolver = Arc<dyn SessionResolver>;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn assistant_reply_rejects_the_python_documented_shapes() {
        let cases: &[(Value, &str)] = &[
            (json!({}), "reply 必须且只能包含 items"),
            (
                json!({"items": [], "extra": 1}),
                "reply 必须且只能包含 items",
            ),
            (json!({"items": []}), "reply.items 必须是非空数组"),
            (json!({"items": [1]}), "reply.items[0] 必须是对象"),
            (
                json!({"items": [{"kind": "text"}]}),
                "reply.items[0] text 结构非法",
            ),
            (
                json!({"items": [{"kind": "text", "text": ""}]}),
                "reply.items[0].text 不可为空",
            ),
            (
                json!({"items": [{"kind": "tool", "name": "x"}]}),
                "reply.items[0] tool 结构非法",
            ),
            (
                json!({"items": [{"kind": "tool", "name": "", "input": {}, "output": ""}]}),
                "reply.items[0].name 必须是非空字符串",
            ),
            (
                json!({"items": [{"kind": "tool", "name": "x", "input": 1, "output": ""}]}),
                "reply.items[0].input 必须是对象或字符串",
            ),
            (
                json!({"items": [{"kind": "tool", "name": "x", "input": {}, "output": 1}]}),
                "reply.items[0].output 必须是字符串",
            ),
            (
                json!({"items": [{"kind": "image"}]}),
                "reply.items[0].kind 仅支持 text/tool",
            ),
        ];
        for (value, expected) in cases {
            let error = AssistantReply::from_value(value).unwrap_err();
            assert_eq!(error.code, "edit.invalid_reply", "value={value}");
            assert_eq!(error.message(), *expected, "value={value}");
        }
    }

    #[test]
    fn assistant_reply_round_trips() {
        let value = json!({"items": [
            {"kind": "text", "text": "你好"},
            {"kind": "tool", "name": "bash", "input": "ls", "output": "a\nb"},
        ]});
        let reply = AssistantReply::from_value(&value).unwrap();
        assert_eq!(reply.to_value(), value);
    }

    #[test]
    fn session_id_follows_python_str_value_or_empty() {
        let of = |id: Value| {
            let mut row = Map::new();
            row.insert("id".into(), id);
            IndexedSession {
                tool: "claude".into(),
                opaque_ref: "fsr_x".into(),
                canonical_ref: "/tmp/a.jsonl".into(),
                revision: "r".into(),
                row,
            }
            .session_id()
        };
        assert_eq!(of(json!("abc")), "abc");
        // `str(True)` 是 "True"，不是 JSON 的 "true"。
        assert_eq!(of(json!(true)), "True");
        assert_eq!(of(json!(7)), "7");
        // `value or ""`：falsy 一律空串。
        assert_eq!(of(json!(false)), "");
        assert_eq!(of(json!(0)), "");
        assert_eq!(of(json!(null)), "");
        assert_eq!(of(json!("")), "");
        assert_eq!(
            IndexedSession {
                tool: "claude".into(),
                opaque_ref: "fsr_x".into(),
                canonical_ref: "/tmp/a.jsonl".into(),
                revision: "r".into(),
                row: Map::new(),
            }
            .session_id(),
            ""
        );
    }

    #[test]
    fn engine_error_keeps_python_class_names() {
        assert_eq!(EngineError::runtime("x").error_type(), "RuntimeError");
        assert_eq!(
            EngineError::from(DomainError::concurrent_modification("x")).error_type(),
            "ConcurrentModificationError"
        );
        assert!(EngineError::from(DomainError::concurrent_modification("x"))
            .is_concurrent_modification());
    }
}
