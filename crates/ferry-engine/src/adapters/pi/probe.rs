//! Pi 原生加载探针：只用 RPC 元数据命令，不触发任何模型调用。
//!
//! 探针把会话拷进临时目录跑「影子会话」，pi CLI 只被要求执行 `get_entries` /
//! `get_tree` 两条元数据命令，两条都成功且退出码为 0 才算通过。

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::adapters::contracts::SessionVerifier;
use crate::adapters::shared::scanner::split_jsonl_lines;
use crate::errors::{DomainError, DomainResult};
use crate::system::executables;
use crate::system::paths::process_environ;
use crate::system::probes::{self, ProbeReport, ProbeTimeout};

/// pi RPC 探针的两条元数据命令。
const RPC_PAYLOAD: &str =
    "{\"id\":\"entries\",\"type\":\"get_entries\"}\n{\"id\":\"tree\",\"type\":\"get_tree\"}\n";

fn exit_code_params(returncode: Option<i32>) -> Map<String, Value> {
    let mut params = Map::new();
    params.insert("tool".into(), Value::from("pi"));
    params.insert(
        "exit_code".into(),
        returncode.map_or(Value::Null, |code| Value::from(i64::from(code))),
    );
    params
}

/// 用真实 pi RPC 加载一个会话文件；migration writer 的第三道验收就是它。
pub fn probe_path(path: &str, cwd: Option<&str>) -> DomainResult<ProbeReport> {
    let resolved =
        fs::canonicalize(path).map_err(|_| DomainError::session_not_found("pi", path))?;
    let session_dir = resolved
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let session_path = resolved.to_string_lossy().into_owned();
    let session_dir_text = session_dir.to_string_lossy().into_owned();
    let config = tempfile::tempdir()
        .map_err(|error| DomainError::internal(format!("Pi 探针临时目录失败: {error}")))?;

    let command = executables::argv(
        "pi",
        &[
            "--mode",
            "rpc",
            "--session",
            &session_path,
            "--session-dir",
            &session_dir_text,
            "--offline",
            "--no-extensions",
            "--no-skills",
            "--no-prompt-templates",
            "--no-themes",
            "--no-context-files",
            "--no-tools",
            "--no-approve",
        ],
    );
    let mut environ = process_environ();
    for (key, value) in [
        (
            "PI_CODING_AGENT_DIR",
            config.path().to_string_lossy().into_owned(),
        ),
        ("PI_CODING_AGENT_SESSION_DIR", session_dir_text.clone()),
        ("PI_OFFLINE", "1".into()),
        ("PI_SKIP_VERSION_CHECK", "1".into()),
        ("PI_TELEMETRY", "0".into()),
    ] {
        environ.insert(key.to_string(), value);
    }
    let env: Vec<(String, String)> = environ.into_iter().collect();
    let result = probes::run_agent_command(
        &command,
        cwd.map(Path::new),
        Some(RPC_PAYLOAD),
        30,
        Some(&env),
    )
    .map_err(DomainError::internal)?;
    if result.timed_out {
        return Ok(probes::timeout_report(
            "pi",
            &ProbeTimeout {
                message: format!("探针超时: {}", command.join(" ")),
            },
        ));
    }

    let mut answered: Vec<String> = Vec::new();
    for line in split_jsonl_lines(&result.stdout) {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let command_name = value.get("command").and_then(Value::as_str);
        let id = value.get("id").and_then(Value::as_str);
        let shaped = value.get("type").and_then(Value::as_str) == Some("response")
            && matches!(command_name, Some("get_entries") | Some("get_tree"))
            && matches!(id, Some("entries") | Some("tree"));
        if !shaped {
            continue;
        }
        // `success` 显式为 false 才算失败；缺席按成功处理。
        if value.get("success") == Some(&Value::Bool(false)) {
            continue;
        }
        if let Some(name) = command_name {
            if !answered.iter().any(|known| known == name) {
                answered.push(name.to_string());
            }
        }
    }
    answered.sort_unstable();
    if result.returncode == Some(0) && answered == ["get_entries", "get_tree"] {
        return Ok(probes::report(
            "passed",
            None,
            None,
            &result.stdout,
            &result.stderr,
        ));
    }
    Ok(probes::report(
        "failed",
        Some("probe.process_failed"),
        Some(exit_code_params(result.returncode)),
        &result.stdout,
        &result.stderr,
    ))
}

/// assistant 消息的纯文本投影；`None` 表示结构不认识。
fn assistant_text(message: &Value) -> Option<String> {
    match message.get("content") {
        Some(Value::String(text)) => Some(text.clone()),
        Some(Value::Array(parts)) => Some(
            parts
                .iter()
                .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .concat(),
        ),
        _ => None,
    }
}

/// `--mode json` 的事件流 → `(最后一条 assistant 消息, 文本)`。
fn parse_prompt_output(raw: &str) -> Option<(Value, String)> {
    let mut events: Vec<Value> = Vec::new();
    for line in split_jsonl_lines(raw) {
        if line.trim().is_empty() {
            continue;
        }
        let value = serde_json::from_str::<Value>(line).ok()?;
        if !value.is_object() {
            return None;
        }
        events.push(value);
    }
    let agent_end = events
        .iter()
        .rev()
        .find(|event| event.get("type").and_then(Value::as_str) == Some("agent_end"))?;
    let messages = agent_end.get("messages")?.as_array()?;
    let assistant = messages
        .iter()
        .rev()
        .find(|message| {
            message.is_object() && message.get("role").and_then(Value::as_str) == Some("assistant")
        })?
        .clone();
    let text = assistant_text(&assistant)?;
    Some((assistant, text))
}

fn report_to_map(report: &ProbeReport) -> Map<String, Value> {
    serde_json::to_value(report)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default()
}

fn run_prompt_path(
    path: &Path,
    cwd: Option<&str>,
    prompt: &str,
    model: Option<&str>,
    timeout: u64,
) -> DomainResult<Map<String, Value>> {
    let session_path = fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned();
    let mut command = executables::argv(
        "pi",
        &["--mode", "json", "--session", &session_path, "--approve"],
    );
    if let Some(model) = model.filter(|value| !value.is_empty()) {
        command.push("--model".into());
        command.push(model.to_string());
    }
    let result =
        probes::run_agent_command(&command, cwd.map(Path::new), Some(prompt), timeout, None)
            .map_err(DomainError::internal)?;

    let mut params = exit_code_params(result.returncode);
    let (status, code, text) = if result.timed_out {
        ("failed", Some("agent_prompt.timeout"), String::new())
    } else if result.returncode != Some(0) {
        ("failed", Some("agent_prompt.process_failed"), String::new())
    } else {
        match parse_prompt_output(&result.stdout) {
            None => ("failed", Some("agent_prompt.invalid_output"), String::new()),
            Some((assistant, text)) => {
                for (source, target) in [
                    ("stopReason", "stop_reason"),
                    ("provider", "provider"),
                    ("model", "model"),
                    ("errorMessage", "error_message"),
                ] {
                    if let Some(value) = assistant.get(source).filter(|value| !value.is_null()) {
                        params.insert(target.into(), value.clone());
                    }
                }
                let stop = assistant.get("stopReason").and_then(Value::as_str);
                if matches!(stop, Some("error") | Some("aborted")) {
                    ("failed", Some("agent_prompt.process_failed"), String::new())
                } else {
                    ("completed", None, text)
                }
            }
        }
    };
    let report = probes::report(status, code, Some(params), &result.stdout, &result.stderr);
    let mut payload = report_to_map(&report);
    let (text, truncated) = probes::normalize_agent_text(Some(&text));
    payload.insert("text".into(), Value::from(text));
    payload.insert("text_truncated".into(), Value::Bool(truncated));
    Ok(payload)
}

pub struct PiVerifier;

impl SessionVerifier for PiVerifier {
    fn prompt_session(
        &self,
        session_id: &str,
        cwd: Option<&str>,
        prompt: &str,
        model: Option<&str>,
        timeout: u64,
    ) -> DomainResult<Map<String, Value>> {
        run_prompt_path(
            &super::adapter::resolve(session_id)?,
            cwd,
            prompt,
            model,
            timeout,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn assistant_text_handles_both_content_shapes() {
        assert_eq!(
            assistant_text(&json!({"content": "PROBE_OK"})).as_deref(),
            Some("PROBE_OK")
        );
        assert_eq!(
            assistant_text(&json!({"content": [
                {"type": "text", "text": "PROBE"},
                {"type": "thinking", "thinking": "x"},
                {"type": "text", "text": "_OK"},
            ]}))
            .as_deref(),
            Some("PROBE_OK")
        );
        // 结构不认识 -> None（上层折成 agent_prompt.invalid_output）。
        assert_eq!(assistant_text(&json!({"content": 1})), None);
        assert_eq!(assistant_text(&json!({})), None);
    }

    #[test]
    fn prompt_output_takes_the_last_assistant_of_the_last_agent_end() {
        let raw = [
            json!({"type": "agent_start"}),
            json!({"type": "agent_end", "messages": [
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": "first"},
            ]}),
            json!({"type": "agent_end", "messages": [
                {"role": "assistant", "content": "second"},
                {"role": "user", "content": "tail"},
            ]}),
        ]
        .iter()
        .map(|value| serde_json::to_string(value).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
        let (assistant, text) = parse_prompt_output(&raw).unwrap();
        assert_eq!(text, "second");
        assert_eq!(assistant["role"], json!("assistant"));
    }

    #[test]
    fn malformed_prompt_output_is_rejected() {
        assert!(parse_prompt_output("not json").is_none());
        assert!(parse_prompt_output("").is_none());
        // 没有 agent_end。
        assert!(parse_prompt_output(r#"{"type":"agent_start"}"#).is_none());
        // agent_end 没有 messages 数组。
        assert!(parse_prompt_output(r#"{"type":"agent_end","messages":1}"#).is_none());
        // messages 里没有 assistant。
        assert!(parse_prompt_output(
            r#"{"type":"agent_end","messages":[{"role":"user","content":"x"}]}"#
        )
        .is_none());
        // 非对象事件行。
        assert!(parse_prompt_output("[1]").is_none());
    }

    #[test]
    fn probe_path_reports_session_not_found_for_missing_files() {
        let error = probe_path("/nonexistent/pi/session.jsonl", None).unwrap_err();
        assert_eq!(error.code, "session.not_found");
    }

    #[test]
    fn probe_reports_serialise_to_the_python_dict_shape() {
        let report = probes::report(
            "failed",
            Some("probe.process_failed"),
            Some(exit_code_params(Some(2))),
            "out",
            "err",
        );
        let payload = report_to_map(&report);
        assert_eq!(payload["status"], json!("failed"));
        assert_eq!(payload["code"], json!("probe.process_failed"));
        assert_eq!(payload["params"]["tool"], json!("pi"));
        assert_eq!(payload["params"]["exit_code"], json!(2));
        assert_eq!(payload["diagnostic"]["stdout"], json!("out"));
        assert_eq!(payload["diagnostic"]["truncated"], json!(false));
    }
}
