//! Claude 会话提问实现。

use std::path::Path;

use serde_json::{Map, Value};

use crate::adapters::contracts::SessionVerifier;
use crate::errors::{DomainError, DomainResult};
use crate::system::executables;
use crate::system::probes::{self, AgentProcessResult, ProbeReport};

fn params_for(exit_code: Option<i32>) -> Map<String, Value> {
    let mut params = Map::new();
    params.insert("tool".into(), Value::from("claude"));
    params.insert(
        "exit_code".into(),
        exit_code.map_or(Value::Null, |code| Value::from(i64::from(code))),
    );
    params
}

fn report_to_map(report: &ProbeReport) -> Map<String, Value> {
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
        report.code.as_deref().map_or(Value::Null, Value::from),
    );
    payload.insert("params".into(), Value::Object(report.params.clone()));
    payload.insert("diagnostic".into(), Value::Object(diagnostic));
    payload
}

fn prompt_report(
    result: &AgentProcessResult,
    status: &str,
    code: Option<&str>,
    params: Map<String, Value>,
    text: &str,
) -> Map<String, Value> {
    let report = probes::report(status, code, Some(params), &result.stdout, &result.stderr);
    let mut payload = report_to_map(&report);
    let (text, truncated) = probes::normalize_agent_text(Some(text));
    payload.insert("text".into(), Value::from(text));
    payload.insert("text_truncated".into(), Value::Bool(truncated));
    payload
}

fn prompt_session(
    session_id: &str,
    cwd: Option<&str>,
    prompt: &str,
    model: Option<&str>,
    timeout: u64,
) -> DomainResult<Map<String, Value>> {
    let cwd = cwd
        .filter(|cwd| !cwd.is_empty())
        .ok_or_else(|| DomainError::internal("claude prompt 必须提供项目目录"))?;
    let mut command = executables::argv(
        "claude",
        &[
            "-p",
            prompt,
            "--resume",
            session_id,
            "--output-format",
            "json",
            "--dangerously-skip-permissions",
        ],
    );
    if let Some(model) = model.filter(|model| !model.is_empty()) {
        command.push("--model".to_string());
        command.push(model.to_string());
    }
    let result = probes::run_agent_command(&command, Some(Path::new(cwd)), None, timeout, None)
        .map_err(DomainError::internal)?;
    let mut params = params_for(result.returncode);
    if result.timed_out {
        return Ok(prompt_report(
            &result,
            "failed",
            Some("agent_prompt.timeout"),
            params,
            "",
        ));
    }
    let raw = result.stdout.trim();
    let output = if raw.is_empty() {
        None
    } else {
        serde_json::from_str::<Value>(raw).ok()
    };
    let Some(output) = output.as_ref().and_then(Value::as_object) else {
        let code = if result.returncode != Some(0) {
            "agent_prompt.process_failed"
        } else {
            "agent_prompt.invalid_output"
        };
        return Ok(prompt_report(&result, "failed", Some(code), params, ""));
    };
    for key in ["terminal_reason", "stop_reason", "session_id"] {
        if let Some(value) = output.get(key).filter(|value| !value.is_null()) {
            params.insert(key.to_string(), value.clone());
        }
    }
    if truthy(output.get("is_error")) || result.returncode != Some(0) {
        return Ok(prompt_report(
            &result,
            "failed",
            Some("agent_prompt.process_failed"),
            params,
            "",
        ));
    }
    let Some(text) = output.get("result").and_then(Value::as_str) else {
        return Ok(prompt_report(
            &result,
            "failed",
            Some("agent_prompt.invalid_output"),
            params,
            "",
        ));
    };
    Ok(prompt_report(&result, "completed", None, params, text))
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|float| float != 0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(entries)) => !entries.is_empty(),
    }
}

pub struct ClaudeVerifier;

impl SessionVerifier for ClaudeVerifier {
    fn prompt_session(
        &self,
        session_id: &str,
        cwd: Option<&str>,
        prompt: &str,
        model: Option<&str>,
        timeout: u64,
    ) -> DomainResult<Map<String, Value>> {
        prompt_session(session_id, cwd, prompt, model, timeout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_working_directory_is_rejected() {
        assert!(prompt_session("sid", None, "hi", None, 30).is_err());
    }
}
