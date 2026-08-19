//! Codex 会话验收探针：真实探测与临时 `CODEX_HOME` 完整树探测。

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Map, Value};

use crate::adapters::contracts::{SessionEditor, SessionVerifier};
use crate::adapters::shared::editing::EditDocument;
use crate::errors::{DomainError, DomainResult};
use crate::system::paths::home_dir;
use crate::system::probes::{self, ProbeReport};
use crate::system::{executables, paths};

/// Codex 探针。
pub struct CodexVerifier;

fn resume_argv(session_id: &str, model: Option<&str>) -> Vec<String> {
    let mut command = executables::argv("codex", &["exec", "resume", session_id]);
    command.push("--skip-git-repo-check".to_string());
    if let Some(model) = model.filter(|value| !value.is_empty()) {
        command.push("-m".to_string());
        command.push(model.to_string());
    }
    command
}

fn probe_in_env(
    session_id: &str,
    model: Option<&str>,
    env: Option<&[(String, String)]>,
) -> DomainResult<ProbeReport> {
    let mut command = resume_argv(session_id, model);
    command.push(probes::PROBE_PROMPT.to_string());
    let result = probes::run(&command, None, Duration::from_secs(180), env)
        .map_err(|error| DomainError::probe_timeout(error.message))?;
    if result.returncode != Some(0) {
        let mut params = Map::new();
        params.insert("tool".into(), Value::from("codex"));
        params.insert(
            "exit_code".into(),
            result.returncode.map_or(Value::Null, Value::from),
        );
        return Ok(probes::report(
            "failed",
            Some("probe.process_failed"),
            Some(params),
            &result.stdout,
            &result.stderr,
        ));
    }
    if !probes::response_matches(Some(&result.stdout)) {
        let mut params = Map::new();
        params.insert("tool".into(), Value::from("codex"));
        return Ok(probes::report(
            "failed",
            Some("probe.unexpected_response"),
            Some(params),
            &result.stdout,
            &result.stderr,
        ));
    }
    Ok(probes::report(
        "passed",
        None,
        None,
        &result.stdout,
        &result.stderr,
    ))
}

impl SessionVerifier for CodexVerifier {
    fn probe(
        &self,
        session_id: &str,
        _cwd: Option<&str>,
        model: Option<&str>,
    ) -> DomainResult<ProbeReport> {
        probe_in_env(session_id, model, None)
    }

    fn probe_edited(
        &self,
        _editor: &dyn SessionEditor,
        _doc: &EditDocument,
        result: &Map<String, Value>,
        model: Option<&str>,
    ) -> DomainResult<ProbeReport> {
        let temp = tempfile::Builder::new()
            .prefix("ferry-codex-probe-")
            .tempdir()
            .map_err(|error| DomainError::internal(format!("创建探针临时目录失败: {error}")))?;
        let codex_home = temp.path().join(".codex");
        let sessions = codex_home
            .join("sessions")
            .join("probe")
            .join("01")
            .join("01");
        fs::create_dir_all(&sessions)
            .map_err(|error| DomainError::internal(format!("创建探针会话目录失败: {error}")))?;

        let published: Vec<String> = match result.get("published_paths").and_then(Value::as_array) {
            Some(items) => items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect(),
            None => result
                .get("saved_as")
                .and_then(Value::as_str)
                .map(str::to_string)
                .into_iter()
                .collect(),
        };
        for raw in &published {
            let source = PathBuf::from(raw);
            let Some(name) = source.file_name() else {
                continue;
            };
            fs::copy(&source, sessions.join(name))
                .map_err(|error| DomainError::internal(format!("复制探针会话失败: {error}")))?;
        }
        for name in ["auth.json", "config.toml"] {
            let source = home_dir().join(".codex").join(name);
            if source.exists() {
                let _ = fs::copy(&source, codex_home.join(name));
            }
        }
        let mut env: Vec<(String, String)> = paths::process_environ().into_iter().collect();
        env.retain(|(key, _)| key != "CODEX_HOME");
        env.push((
            "CODEX_HOME".to_string(),
            codex_home.to_string_lossy().into_owned(),
        ));

        let session_id = result
            .get("session_id")
            .and_then(Value::as_str)
            .unwrap_or("");
        let report = probe_in_env(session_id, model, Some(&env))?;
        // Python 把 isolation 挂在报告**顶层**（`rep["isolation"]`），前端
        // `app/src/shared/contracts/events.js::probeText` 读的也是 `p.isolation`。
        Ok(report.with_isolation("temp_home", session_id, true))
    }

    fn prompt_session(
        &self,
        session_id: &str,
        _cwd: Option<&str>,
        prompt: &str,
        model: Option<&str>,
        timeout: u64,
    ) -> DomainResult<Map<String, Value>> {
        let mut command = executables::argv(
            "codex",
            &[
                "exec",
                "resume",
                "--skip-git-repo-check",
                "--dangerously-bypass-approvals-and-sandbox",
            ],
        );
        if let Some(model) = model.filter(|value| !value.is_empty()) {
            command.push("-m".to_string());
            command.push(model.to_string());
        }
        command.push(session_id.to_string());
        command.push(prompt.to_string());
        let result = probes::run_agent_command(&command, None, None, timeout, None)
            .map_err(DomainError::internal)?;
        let mut params = Map::new();
        params.insert("tool".into(), Value::from("codex"));
        params.insert(
            "exit_code".into(),
            result.returncode.map_or(Value::Null, Value::from),
        );
        let (status, code, text) = if result.timed_out {
            ("failed", Some("agent_prompt.timeout"), "")
        } else if result.returncode != Some(0) {
            ("failed", Some("agent_prompt.process_failed"), "")
        } else {
            ("completed", None, result.stdout.as_str())
        };
        let report = probes::report(status, code, Some(params), &result.stdout, &result.stderr);
        let mut payload = serde_json::to_value(&report)
            .ok()
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        let (normalized, truncated) = probes::normalize_agent_text(Some(text));
        payload.insert("text".into(), Value::from(normalized));
        payload.insert("text_truncated".into(), Value::Bool(truncated));
        Ok(payload)
    }
}

/// `probe_edited` 复制会话时用到的会话目录（供测试断言路径形态）。
#[cfg(test)]
fn probe_sessions_subpath() -> PathBuf {
    PathBuf::from("sessions")
        .join("probe")
        .join("01")
        .join("01")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_argv_appends_the_model_flag_last() {
        let plain = resume_argv("abc", None);
        // argv[0] 是解析出来的可执行文件路径，其余参数逐字固定。
        assert_eq!(
            &plain[1..],
            ["exec", "resume", "abc", "--skip-git-repo-check"]
        );
        let with_model = resume_argv("abc", Some("gpt-5.4"));
        assert_eq!(&with_model[with_model.len() - 2..], ["-m", "gpt-5.4"]);
    }

    #[test]
    fn the_probe_home_layout_is_stable() {
        assert_eq!(
            probe_sessions_subpath(),
            PathBuf::from("sessions/probe/01/01")
        );
    }
}
