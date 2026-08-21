//! 跨能力包共享的结构化错误：code + params，供 RPC 下发。
//!
//! `code` 是稳定的机器码（如 `session.concurrent_modification`），`params` 只放
//! 语义字段；`message` 会经 RPC envelope 的 `params.message` 下发给 agent 作恢复
//! 指引，文案按「模型读得懂、知道下一步」来写，中文原样保留（测试逐字断言）。

use serde_json::{Map, Value};

use crate::contracts::errors::error_policy;

/// `probe.timeout` 是**未注册在契约里的幽灵码**（方案 §2.1 第 4 条 / §5）。
/// 只能由 [`DomainError::probe_timeout`] 构造，绕开 code 注册校验。
pub const PROBE_TIMEOUT_CODE: &str = "probe.timeout";

/// 领域错误。等价于 Python 的 `DomainError` 及其全部子类。
///
/// `error_type` 保存 Python 异常类名：operation 失败时落库的就是这个字符串
/// （方案 §2.4 第 22 条），Rust 侧必须维持同名映射。
#[derive(Clone, Debug, PartialEq)]
pub struct DomainError {
    pub code: &'static str,
    pub error_type: &'static str,
    pub category: &'static str,
    pub retryable: bool,
    /// message 与 params 装箱：DomainError 出现在几乎每个 `Result` 的 Err 侧，
    /// 直接内联会把返回值撑到 150+ 字节（clippy::result_large_err）。
    detail: Box<Detail>,
}

#[derive(Clone, Debug, PartialEq)]
struct Detail {
    message: String,
    params: Map<String, Value>,
}

impl DomainError {
    /// 通用构造：`code` 必须已注册在 `contracts/errors.json`，否则视为缺陷。
    ///
    /// 对齐 Python `DomainError.__init__`：`str(err)` 在 message 为空时回落到 code。
    pub fn new(
        code: &'static str,
        error_type: &'static str,
        message: impl Into<String>,
        params: Map<String, Value>,
    ) -> Self {
        let policy = error_policy(code)
            .unwrap_or_else(|| panic!("错误码未注册在 contracts/errors.json: {code}"));
        let message = message.into();
        Self {
            code,
            error_type,
            category: policy.category,
            retryable: policy.retryable,
            detail: Box::new(Detail {
                message: if message.is_empty() {
                    code.to_string()
                } else {
                    message
                },
                params,
            }),
        }
    }

    /// 等价 Python 的 `str(error)`。
    pub fn message(&self) -> &str {
        &self.detail.message
    }

    /// 结构化参数；RPC 信封会在这里补一个 `message` 键。
    pub fn params(&self) -> &Map<String, Value> {
        &self.detail.params
    }

    pub fn params_mut(&mut self) -> &mut Map<String, Value> {
        &mut self.detail.params
    }

    /// 探针超时的幽灵错误码：不在契约里注册，策略硬编码为 internal/retryable。
    pub fn probe_timeout(message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            code: PROBE_TIMEOUT_CODE,
            error_type: "ProbeTimeout",
            category: "internal",
            retryable: true,
            detail: Box::new(Detail {
                message: if message.is_empty() {
                    PROBE_TIMEOUT_CODE.to_string()
                } else {
                    message
                },
                params: Map::new(),
            }),
        }
    }

    /// 未捕获异常的兜底：不泄漏异常文本（除非 FERRY_DEBUG）。
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new("internal.unexpected", "DomainError", message, Map::new())
    }

    /// 源会话在加载后发生变化；不得用旧快照覆盖新内容。
    pub fn concurrent_modification(message: impl Into<String>) -> Self {
        Self::new(
            "session.concurrent_modification",
            "ConcurrentModificationError",
            message,
            Map::new(),
        )
    }

    pub fn invalid_json(message: impl Into<String>) -> Self {
        Self::new("rpc.invalid_json", "InvalidJsonError", message, Map::new())
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(
            "rpc.invalid_request",
            "InvalidRequestError",
            message,
            Map::new(),
        )
    }

    pub fn unsupported_protocol(expected: &str, actual: Value) -> Self {
        let mut params = Map::new();
        params.insert("expected".into(), Value::from(expected));
        params.insert("actual".into(), actual);
        Self::new(
            "rpc.unsupported_protocol",
            "UnsupportedProtocolError",
            "IPC protocol 不匹配",
            params,
        )
    }

    pub fn unknown_method(method: &str) -> Self {
        let mut params = Map::new();
        params.insert("method".into(), Value::from(method));
        Self::new(
            "rpc.unknown_method",
            "UnknownMethodError",
            format!("未知 method: {method}"),
            params,
        )
    }

    /// callers 矩阵拒绝：方法存在，但不对这条传输通道**按方法名分发**。
    ///
    /// 复用 `rpc.unknown_method` 是刻意的：从这条通道看出去，这个方法名就是
    /// 不可分发的；单独造码会把「传输策略」泄漏成新的错误契约。
    pub fn method_not_exposed(method: &str, caller: &str) -> Self {
        let mut params = Map::new();
        params.insert("method".into(), Value::from(method));
        params.insert("caller".into(), Value::from(caller));
        params.insert("reason".into(), Value::from("caller_not_allowed"));
        Self::new(
            "rpc.unknown_method",
            "UnknownMethodError",
            format!("方法未对 {caller} 暴露: {method}"),
            params,
        )
    }

    /// 传输层管理方法被拒（如 App 模式下 CLI 想关引擎）。
    pub fn transport_refused(reason: &str, message: &str, recovery: &str) -> Self {
        let mut params = Map::new();
        params.insert("reason".into(), Value::from(reason));
        params.insert("recovery".into(), Value::from(recovery));
        Self::new(
            "rpc.invalid_request",
            "InvalidRequestError",
            message,
            params,
        )
    }

    /// CLI 客户端侧的连接/传输失败：连不上、拉不起、握手不一致。
    pub fn engine_unavailable(reason: &str, message: impl Into<String>, recovery: &str) -> Self {
        let mut params = Map::new();
        params.insert("reason".into(), Value::from(reason));
        params.insert("recovery".into(), Value::from(recovery));
        Self::new(
            "engine.unavailable",
            "EngineUnavailableError",
            message,
            params,
        )
    }

    pub fn missing_param(param: &str) -> Self {
        let mut params = Map::new();
        params.insert("param".into(), Value::from(param));
        Self::new(
            "rpc.missing_param",
            "MissingParamError",
            format!("缺少参数: {param}"),
            params,
        )
    }

    pub fn tool_unknown(tool: &str) -> Self {
        let mut params = Map::new();
        params.insert("tool".into(), Value::from(tool));
        Self::new(
            "tool.unknown",
            "ToolUnknownError",
            format!("未知工具: {tool}"),
            params,
        )
    }

    pub fn session_not_found(tool: &str, reference: &str) -> Self {
        let mut params = Map::new();
        params.insert("tool".into(), Value::from(tool));
        params.insert("ref".into(), Value::from(reference));
        Self::new(
            "session.not_found",
            "SessionNotFoundError",
            format!("找不到 {tool} 会话: {reference}"),
            params,
        )
    }

    pub fn session_store_unavailable(tool: &str, reason: &str) -> Self {
        let mut params = Map::new();
        params.insert("tool".into(), Value::from(tool));
        params.insert("reason".into(), Value::from(reason));
        Self::new(
            "session.store_unavailable",
            "SessionStoreUnavailableError",
            format!("{tool} 会话存储不可用: {reason}"),
            params,
        )
    }

    pub fn agent_format_changed(
        agent: &str,
        location: &str,
        expected: Value,
        actual: Value,
    ) -> Self {
        let mut params = Map::new();
        params.insert("agent".into(), Value::from(agent));
        params.insert("location".into(), Value::from(location));
        params.insert("expected".into(), expected);
        params.insert("actual".into(), actual);
        Self::new(
            "agent.format_changed",
            "AgentFormatChangedError",
            format!("{agent} 当前结构不匹配: {location}"),
            params,
        )
    }

    pub fn session_asset_not_found(asset_id: &str) -> Self {
        let mut params = Map::new();
        params.insert("asset_id".into(), Value::from(asset_id));
        Self::new(
            "session.asset_not_found",
            "SessionAssetNotFoundError",
            "找不到会话图片",
            params,
        )
    }

    /// UI 持有的定位符与当前会话不再匹配；message 缺省即 Python 的默认文案。
    pub fn locator_stale(message: Option<&str>, params: Map<String, Value>) -> Self {
        Self::new(
            "session.locator_stale",
            "LocatorStaleError",
            message.unwrap_or("turn locator 已失效，请刷新会话"),
            params,
        )
    }

    pub fn turn_out_of_range(requested_turn: Value, turn_count: Option<i64>) -> Self {
        let mut params = Map::new();
        params.insert("requested_turn".into(), requested_turn);
        let message = match turn_count {
            Some(count) => {
                params.insert("turn_count".into(), Value::from(count));
                format!("轮次超界: 共 {count} 轮")
            }
            None => "turn 必须是正整数".to_string(),
        };
        Self::new(
            "edit.turn_out_of_range",
            "TurnOutOfRangeError",
            message,
            params,
        )
    }

    pub fn operation_unsupported(tool: &str, operation: &str, mode: Option<&str>) -> Self {
        let mut params = Map::new();
        params.insert("tool".into(), Value::from(tool));
        params.insert("operation".into(), Value::from(operation));
        let mut message = format!("{tool} 不支持操作 {operation}");
        if let Some(mode) = mode {
            message.push_str(&format!("（{mode}）"));
            params.insert("mode".into(), Value::from(mode));
        }
        Self::new(
            "edit.operation_unsupported",
            "OperationUnsupportedError",
            message,
            params,
        )
    }

    pub fn invalid_reply(message: impl Into<String>) -> Self {
        Self::new(
            "edit.invalid_reply",
            "InvalidReplyError",
            message,
            Map::new(),
        )
    }

    pub fn subagent_not_supported(message: impl Into<String>) -> Self {
        Self::new(
            "edit.subagent_not_supported",
            "SubagentNotSupportedError",
            message,
            Map::new(),
        )
    }

    /// Agent 只能使用当前 Engine 扫描索引签发的 opaque ref。
    pub fn agent_reference_invalid(message: impl Into<String>) -> Self {
        Self::new(
            "agent.reference_invalid",
            "AgentReferenceError",
            message,
            Map::new(),
        )
    }

    pub fn agent_request_invalid(message: impl Into<String>) -> Self {
        Self::new(
            "agent.request_invalid",
            "AgentRequestError",
            message,
            Map::new(),
        )
    }

    /// `AgentCapabilityError`：Python 侧是 `AgentRequestError` 的子类，共用 code。
    pub fn agent_capability(tool: &str, capability: &str) -> Self {
        let mut params = Map::new();
        params.insert("tool".into(), Value::from(tool));
        params.insert("capability".into(), Value::from(capability));
        Self::new(
            "agent.request_invalid",
            "AgentCapabilityError",
            format!("{tool} 不支持能力 {capability}"),
            params,
        )
    }
}

impl std::fmt::Display for DomainError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.detail.message)
    }
}

impl std::error::Error for DomainError {}

/// 领域结果别名。
pub type DomainResult<T> = Result<T, DomainError>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::errors::FERRY_ERROR_CODES;

    #[test]
    fn policy_comes_from_the_generated_contract() {
        let error = DomainError::concurrent_modification("");
        assert_eq!(error.code, "session.concurrent_modification");
        assert_eq!(error.category, "conflict");
        assert!(error.retryable);
        // message 为空时回落到 code（对齐 Python `super().__init__(message or code)`）。
        assert_eq!(error.message(), "session.concurrent_modification");
    }

    #[test]
    fn messages_and_params_match_python_wording() {
        let error = DomainError::session_not_found("claude", "fsr_abc");
        assert_eq!(error.message(), "找不到 claude 会话: fsr_abc");
        assert_eq!(error.params()["tool"], Value::from("claude"));
        assert_eq!(error.params()["ref"], Value::from("fsr_abc"));

        let ranged = DomainError::turn_out_of_range(Value::from(9), Some(3));
        assert_eq!(ranged.message(), "轮次超界: 共 3 轮");
        assert_eq!(ranged.params()["turn_count"], Value::from(3));
        let unbounded = DomainError::turn_out_of_range(Value::from("x"), None);
        assert_eq!(unbounded.message(), "turn 必须是正整数");
        assert!(!unbounded.params().contains_key("turn_count"));

        let unsupported = DomainError::operation_unsupported("pi", "rewrite", Some("inplace"));
        assert_eq!(unsupported.message(), "pi 不支持操作 rewrite（inplace）");
        assert_eq!(unsupported.params()["mode"], Value::from("inplace"));
        let plain = DomainError::operation_unsupported("pi", "rewrite", None);
        assert_eq!(plain.message(), "pi 不支持操作 rewrite");
        assert!(!plain.params().contains_key("mode"));
    }

    #[test]
    fn probe_timeout_stays_a_ghost_code() {
        assert!(!FERRY_ERROR_CODES.contains(&PROBE_TIMEOUT_CODE));
        let error = DomainError::probe_timeout("探针超时: claude --version");
        assert_eq!(error.code, "probe.timeout");
        assert_eq!(error.category, "internal");
        assert!(error.retryable);
    }

    #[test]
    #[should_panic(expected = "错误码未注册")]
    fn unregistered_codes_are_a_bug() {
        DomainError::new("nope.not_a_code", "Nope", "", Map::new());
    }
}
