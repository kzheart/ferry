//! 结构化事件：code + params，渲染语言由 UI 决定。

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// `event()` 的默认 severity。
pub const DEFAULT_SEVERITY: &str = "warning";

/// 一条结构化事件（会话损耗记录用的也是它）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub code: String,
    pub severity: String,
    pub params: Map<String, Value>,
}

impl Event {
    /// 对应 `event(code, **params)`：severity 取默认值 warning。
    pub fn new(code: impl Into<String>, params: Map<String, Value>) -> Self {
        Self::with_severity(code, DEFAULT_SEVERITY, params)
    }

    /// 对应 `event(code, severity, **params)`。
    pub fn with_severity(
        code: impl Into<String>,
        severity: impl Into<String>,
        params: Map<String, Value>,
    ) -> Self {
        Self {
            code: code.into(),
            severity: severity.into(),
            params,
        }
    }
}

/// `event(code, **params)` 的函数式写法。
pub fn event(code: &str, params: Map<String, Value>) -> Event {
    Event::new(code, params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jsonutil::canonical_json;

    #[test]
    fn event_defaults_to_warning_severity() {
        let mut params = Map::new();
        params.insert("tool".into(), Value::from("claude"));
        let value = serde_json::to_value(event("migration.truncated", params)).unwrap();
        assert_eq!(
            canonical_json(&value).unwrap(),
            r#"{"code":"migration.truncated","params":{"tool":"claude"},"severity":"warning"}"#
        );
    }
}
