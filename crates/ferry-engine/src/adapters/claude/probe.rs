//! Claude 会话验收探针：真实探测与编辑后的影子副本探测。
//!
//! `probe_edited` 在报告顶层追加 `isolation`（`ProbeReport::isolation`），与
//! Python 的 `rep["isolation"] = {...}` 及前端 `events.js::probeText` 读的
//! `p.isolation` 对齐。

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Map, Value};

use crate::adapters::contracts::{SessionEditor, SessionVerifier};
use crate::adapters::shared::editing::EditDocument;
use crate::errors::{DomainError, DomainResult};
use crate::system::probes::{self, AgentProcessResult, ProbeReport, PROBE_PROMPT};
use crate::system::{executables, probes::response_matches};

use super::editing as claude_edit;
use super::editing::uuid4;

/// `probes.run` 的默认超时（Python 侧 `timeout=180`）。
const PROBE_TIMEOUT: Duration = Duration::from_secs(180);

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

fn probe(session_id: &str, cwd: Option<&str>, model: Option<&str>) -> DomainResult<ProbeReport> {
    let cwd = cwd
        .filter(|cwd| !cwd.is_empty())
        .ok_or_else(|| DomainError::internal("claude 探针必须提供 --dir(项目目录)"))?;
    let mut command = executables::argv(
        "claude",
        &[
            "-p",
            PROBE_PROMPT,
            "--resume",
            session_id,
            "--output-format",
            "json",
        ],
    );
    if let Some(model) = model.filter(|model| !model.is_empty()) {
        command.push("--model".to_string());
        command.push(model.to_string());
    }
    let result = probes::run(&command, Some(Path::new(cwd)), PROBE_TIMEOUT, None)
        .map_err(|timeout| DomainError::probe_timeout(timeout.message))?;
    Ok(classify(&result.stdout, &result.stderr, result.returncode))
}

/// 把一次 CLI 调用的输出折成探针报告（与进程调用解耦，便于单测）。
fn classify(stdout: &str, stderr: &str, returncode: Option<i32>) -> ProbeReport {
    let raw = stdout.trim();
    let error = stderr.trim();
    if returncode != Some(0) && raw.is_empty() {
        return probes::report(
            "failed",
            Some("probe.process_failed"),
            Some(params_for(returncode)),
            "",
            error,
        );
    }
    let output = if raw.is_empty() {
        Some(Value::Object(Map::new()))
    } else {
        serde_json::from_str::<Value>(raw).ok()
    };
    let Some(output) = output else {
        return probes::report(
            "failed",
            Some("probe.non_json_output"),
            Some(params_for(returncode)),
            raw,
            error,
        );
    };
    if truthy(output.get("is_error")) || returncode != Some(0) {
        let mut params = params_for(returncode);
        for key in [
            "terminal_reason",
            "stop_reason",
            "api_error_status",
            "session_id",
        ] {
            if let Some(value) = output.get(key).filter(|value| !value.is_null()) {
                params.insert(key.to_string(), value.clone());
            }
        }
        return probes::report(
            "failed",
            Some("probe.process_failed"),
            Some(params),
            raw,
            error,
        );
    }
    let reply = match output.get("result") {
        Some(Value::String(text)) => text.clone(),
        Some(other) => crate::adapters::shared::dialect::python_str(other),
        None => String::new(),
    };
    if !response_matches(Some(&reply)) {
        let mut params = Map::new();
        params.insert("tool".into(), Value::from("claude"));
        return probes::report(
            "failed",
            Some("probe.unexpected_response"),
            Some(params),
            &reply,
            error,
        );
    }
    probes::report("passed", None, None, &reply, "")
}

/// 递归复制 sidecar 目录（等价 `shutil.copytree(dirs_exist_ok=True)`）。
fn copy_tree(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// 把编辑结果复制成一个新 sessionId 的影子会话，探完即删。
fn probe_edited_session(
    result: &Map<String, Value>,
    model: Option<&str>,
) -> DomainResult<ProbeReport> {
    let saved_as = result
        .get("saved_as")
        .and_then(Value::as_str)
        .ok_or_else(|| DomainError::internal("claude 编辑结果缺少 saved_as"))?;
    let path = PathBuf::from(saved_as);
    let mut records = claude_edit::load(&path)?;
    let cwd = records
        .iter()
        .filter_map(|record| record.get("cwd").and_then(Value::as_str))
        .find(|cwd| !cwd.is_empty())
        .unwrap_or(".")
        .to_string();
    let shadow_id = uuid4();
    for record in &mut records {
        if let Some(entries) = record.as_object_mut() {
            if entries.contains_key("sessionId") {
                entries.insert("sessionId".into(), Value::from(shadow_id.as_str()));
            }
        }
    }
    let shadow = path.with_file_name(format!("{shadow_id}.jsonl"));
    claude_edit::save(&shadow, &records)?;
    let sidecar = path.with_extension("");
    let shadow_sidecar = shadow.with_extension("");
    if sidecar.is_dir() {
        copy_tree(&sidecar, &shadow_sidecar).map_err(|error| {
            DomainError::internal(format!("claude 影子 sidecar 复制失败: {error}"))
        })?;
    }
    let outcome = probe(&shadow_id, Some(&cwd), model);
    let _ = std::fs::remove_file(&shadow);
    let _ = std::fs::remove_dir_all(&shadow_sidecar);
    outcome.map(|report| report.with_isolation("shadow_session", &shadow_id, true))
}

pub struct ClaudeVerifier;

impl SessionVerifier for ClaudeVerifier {
    fn probe(
        &self,
        session_id: &str,
        cwd: Option<&str>,
        model: Option<&str>,
    ) -> DomainResult<ProbeReport> {
        probe(session_id, cwd, model)
    }

    fn probe_edited(
        &self,
        _editor: &dyn SessionEditor,
        _doc: &EditDocument,
        result: &Map<String, Value>,
        model: Option<&str>,
    ) -> DomainResult<ProbeReport> {
        probe_edited_session(result, model)
    }

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
    use serde_json::json;

    #[test]
    fn probe_token_round_trip_passes() {
        let report = classify("{\"result\": \"PROBE_OK\"}", "", Some(0));
        assert_eq!(report.status, "passed");
        assert_eq!(report.code, None);
        assert_eq!(report.diagnostic.stdout, "PROBE_OK");
    }

    #[test]
    fn non_zero_exit_without_output_is_a_process_failure() {
        let report = classify("", "boom", Some(2));
        assert_eq!(report.status, "failed");
        assert_eq!(report.code.as_deref(), Some("probe.process_failed"));
        assert_eq!(report.params["exit_code"], json!(2));
        assert_eq!(report.params["tool"], json!("claude"));
        assert_eq!(report.diagnostic.stderr, "boom");
    }

    #[test]
    fn non_json_output_is_reported_separately() {
        let report = classify("not json", "", Some(0));
        assert_eq!(report.code.as_deref(), Some("probe.non_json_output"));
        assert_eq!(report.diagnostic.stdout, "not json");
    }

    #[test]
    fn is_error_payloads_carry_the_agent_reason_fields() {
        let report = classify(
            "{\"is_error\": true, \"stop_reason\": \"refusal\", \"session_id\": \"s1\"}",
            "",
            Some(0),
        );
        assert_eq!(report.code.as_deref(), Some("probe.process_failed"));
        assert_eq!(report.params["stop_reason"], json!("refusal"));
        assert_eq!(report.params["session_id"], json!("s1"));
        assert!(!report.params.contains_key("terminal_reason"));
    }

    #[test]
    fn unexpected_replies_fail_without_an_exit_code_param() {
        let report = classify("{\"result\": \"hello\"}", "warn", Some(0));
        assert_eq!(report.code.as_deref(), Some("probe.unexpected_response"));
        assert_eq!(
            report.params,
            json!({"tool": "claude"}).as_object().cloned().unwrap()
        );
        assert_eq!(report.diagnostic.stdout, "hello");
        assert_eq!(report.diagnostic.stderr, "warn");
    }

    #[test]
    fn missing_working_directory_is_rejected() {
        assert!(probe("sid", None, None).is_err());
        assert!(probe("sid", Some(""), None).is_err());
        assert!(prompt_session("sid", None, "hi", None, 30).is_err());
    }

    #[test]
    fn shadow_probe_cleans_up_its_copies() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("real.jsonl");
        std::fs::write(
            &path,
            "{\"sessionId\": \"real\", \"cwd\": \"/ferry-no-such-cwd\", \"type\": \"user\"}\n",
        )
        .unwrap();
        let sidecar = root.path().join("real/subagents");
        std::fs::create_dir_all(&sidecar).unwrap();
        std::fs::write(sidecar.join("agent-a.jsonl"), "{}\n").unwrap();

        let mut result = Map::new();
        result.insert(
            "saved_as".into(),
            Value::from(path.to_string_lossy().into_owned()),
        );
        // claude CLI 不存在 -> 探针以 process_failed 收场，但清理必须发生。
        let _ = probe_edited_session(&result, None);
        let leftovers: Vec<String> = std::fs::read_dir(root.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            leftovers.len(),
            2,
            "只应留下原会话与其 sidecar: {leftovers:?}"
        );
        assert!(leftovers.contains(&"real.jsonl".to_string()));
        assert!(leftovers.contains(&"real".to_string()));
    }
}
