//! Codex 会话提问实现。

use serde_json::{Map, Value};

use crate::adapters::contracts::SessionVerifier;
use crate::errors::{DomainError, DomainResult};
use crate::system::executables;
use crate::system::probes;

/// Codex 会话提问组件。
pub struct CodexVerifier;

impl SessionVerifier for CodexVerifier {
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
