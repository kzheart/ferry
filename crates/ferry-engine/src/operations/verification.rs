//! Operation 探针入口；具体 CLI 执行由 adapter verifier 持有。
//!
//! Python 的 `ProbeTimeout` 是裸 `RuntimeError` 子类，靠 `error.__class__.__name__`
//! 做鸭子类型识别。Rust 侧统一走 `DomainError::probe_timeout` 的幽灵错误码
//! （`probe.timeout` 未注册在契约里，方案 §2.1 第 4 条 / §5），
//! [`is_probe_timeout`] 是唯一的识别入口。

use serde_json::{Map, Value};

use crate::errors::{DomainError, PROBE_TIMEOUT_CODE};
use crate::operations::types::{EngineResult, Ports};
use crate::system::probes::ProbeReport;

/// 该错误是否是探针超时（等价 Python 的 `except ProbeTimeout`）。
pub fn is_probe_timeout(error: &crate::operations::types::EngineError) -> bool {
    error
        .as_domain()
        .is_some_and(|domain| domain.code == PROBE_TIMEOUT_CODE)
}

/// 与 `verification.timeout_report` 逐字段一致的失败报告。
pub fn timeout_report(tool: &str, message: &str) -> Value {
    let mut diagnostic = Map::new();
    diagnostic.insert("stdout".into(), Value::from(""));
    diagnostic.insert("stderr".into(), Value::from(message));
    diagnostic.insert("truncated".into(), Value::Bool(false));
    let mut params = Map::new();
    params.insert("tool".into(), Value::from(tool));
    let mut report = Map::new();
    report.insert("status".into(), Value::from("failed"));
    report.insert("code".into(), Value::from("probe.timeout"));
    report.insert("params".into(), Value::Object(params));
    report.insert("diagnostic".into(), Value::Object(diagnostic));
    Value::Object(report)
}

/// `ProbeReport` → wire DTO。
pub fn report_to_value(report: &ProbeReport) -> Value {
    let mut diagnostic = Map::new();
    diagnostic.insert(
        "stdout".into(),
        Value::from(report.diagnostic.stdout.as_str()),
    );
    diagnostic.insert(
        "stderr".into(),
        Value::from(report.diagnostic.stderr.as_str()),
    );
    diagnostic.insert("truncated".into(), Value::Bool(report.diagnostic.truncated));
    let mut payload = Map::new();
    payload.insert("status".into(), Value::from(report.status.as_str()));
    payload.insert(
        "code".into(),
        match &report.code {
            Some(code) => Value::from(code.as_str()),
            None => Value::Null,
        },
    );
    payload.insert("params".into(), Value::Object(report.params.clone()));
    payload.insert("diagnostic".into(), Value::Object(diagnostic));
    // Python 的 `rep["isolation"] = {...}` 是最后追加的键；没有隔离就不出现。
    if let Some(isolation) = &report.isolation {
        payload.insert("isolation".into(), Value::Object(isolation.clone()));
    }
    Value::Object(payload)
}

/// 等价 `verification.run_probe`：探针超时原样上抛，其余错误照旧。
pub fn run_probe(
    tool: &str,
    session_id: &str,
    dirpath: Option<&str>,
    model: Option<&str>,
    ports: &Ports,
) -> EngineResult<ProbeReport> {
    let adapter = ports.adapter(tool)?;
    let verifier = adapter.require_verifier("probe")?;
    Ok(verifier.probe(session_id, dirpath, model)?)
}

/// 等价 `verification.run_agent_prompt`；`timeout` 默认 360 秒由分发层给。
pub fn run_agent_prompt(
    tool: &str,
    session_id: &str,
    prompt: &str,
    dirpath: Option<&str>,
    model: Option<&str>,
    timeout: u64,
    ports: &Ports,
) -> EngineResult<Map<String, Value>> {
    let adapter = ports.adapter(tool)?;
    let verifier = adapter.require_verifier("prompt")?;
    Ok(verifier.prompt_session(session_id, dirpath, prompt, model, timeout)?)
}

/// 构造一个探针超时错误（供 adapter 与测试使用）。
pub fn probe_timeout(message: impl Into<String>) -> DomainError {
    DomainError::probe_timeout(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operations::types::EngineError;
    use serde_json::json;

    #[test]
    fn timeout_report_matches_python_field_for_field() {
        assert_eq!(
            timeout_report("claude", "探针超时"),
            json!({
                "status": "failed",
                "code": "probe.timeout",
                "params": {"tool": "claude"},
                "diagnostic": {"stdout": "", "stderr": "探针超时", "truncated": false},
            })
        );
    }

    #[test]
    fn isolation_lands_on_the_report_top_level() {
        // 前端 `events.js::probeText` 读的是 `p.isolation`，不是 `p.params.isolation`。
        let report = crate::system::probes::report("passed", None, None, "", "").with_isolation(
            "shadow_session",
            "abc",
            true,
        );
        assert_eq!(
            report_to_value(&report)["isolation"],
            json!({"kind": "shadow_session", "id": "abc", "cleaned": true})
        );
        // 未做隔离的报告不带这个键。
        let plain = crate::system::probes::report("passed", None, None, "", "");
        assert!(report_to_value(&plain).get("isolation").is_none());
    }

    #[test]
    fn probe_timeout_is_recognisable_but_unregistered() {
        let error = EngineError::from(probe_timeout("boom"));
        assert!(is_probe_timeout(&error));
        assert!(!is_probe_timeout(&EngineError::runtime("boom")));
        assert_eq!(error.error_type(), "ProbeTimeout");
    }
}
