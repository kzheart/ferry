//! OpenCode 会话验收探针：编辑后克隆影子副本探测并清理。
//!
//! `probe_edited` 不在原会话上跑探针：探针会真的向模型发一次请求并写回会话，
//! 因此先把编辑后的 payload 原样 import 成一份影子会话，探完无论成败都删掉。
//!
//! `probe_edited` 在报告**顶层**追加 `isolation`（`ProbeReport::isolation`），
//! 与 Python 的 `rep["isolation"] = {...}` 及前端 `events.js::probeText` 读的
//! `p.isolation` 对齐（与 `adapters::claude::probe` 的处理一致）。

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use serde_json::{Map, Value};

use crate::adapters::contracts::{SessionEditor, SessionVerifier};
use crate::adapters::shared::editing::EditDocument;
use crate::errors::{DomainError, DomainResult};
use crate::system::probes::ProbeReport;
use crate::system::{executables, probes};

use super::editor::OpenCodeData;
use super::{reader, store, writer};

/// 探针超时（Python `timeout=360`）。
const PROBE_TIMEOUT: Duration = Duration::from_secs(360);

/// 事件里的 content 字段：字符串原样，数组取所有 text 段拼接。
fn content_text(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) => Some(text.clone()),
        Some(Value::Array(items)) => Some(
            items
                .iter()
                .filter(|item| item.get("type") == Some(&Value::from("text")))
                .filter_map(|item| item.get("text").and_then(Value::as_str))
                .collect::<String>(),
        ),
        _ => None,
    }
}

/// 从一条 JSON 事件里抽 assistant 正文；抽不到返回 `None`。
fn assistant_text(event: &Value) -> Option<String> {
    let event_type = event.get("type").and_then(Value::as_str);
    match event_type {
        Some("text") => {
            return match event.get("part") {
                Some(Value::Object(part)) => content_text(part.get("text")),
                _ => content_text(event.get("text")),
            };
        }
        Some("assistant") => {}
        _ if event.get("role") == Some(&Value::from("assistant")) => {}
        Some("message") | Some("message.updated") => {
            let message = match event.get("message") {
                Some(Value::Object(message)) => Some(message.clone()),
                _ => event
                    .get("properties")
                    .and_then(Value::as_object)
                    .and_then(|properties| properties.get("info"))
                    .and_then(Value::as_object)
                    .cloned(),
            };
            let message = message?;
            if message.get("role") != Some(&Value::from("assistant")) {
                return None;
            }
            return content_text(message.get("content"));
        }
        _ => return None,
    }
    match event.get("message") {
        Some(Value::Object(message)) => content_text(message.get("content")),
        _ => content_text(event.get("content")),
    }
}

/// 解析 `--format json` 的 NDJSON 输出，返回 `(最后一条正文, 是否出现 error 事件)`。
fn parse_prompt_output(raw: &str) -> (Option<String>, bool) {
    let mut events = Vec::new();
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(line) {
            Ok(Value::Object(entries)) => events.push(Value::Object(entries)),
            // 非对象或解析失败都按「进程失败」处理（Python 的两个 return 分支）。
            _ => return (None, false),
        }
    }
    if events.is_empty() {
        return (None, false);
    }
    if events
        .iter()
        .any(|event| event.get("type") == Some(&Value::from("error")))
    {
        return (None, true);
    }
    let last = events.iter().filter_map(assistant_text).next_back();
    (last, false)
}

/// `opencode run -s <id> [--dir cwd] [-m model] <PROBE_PROMPT>`。
fn probe(session_id: &str, cwd: Option<&str>, model: Option<&str>) -> DomainResult<ProbeReport> {
    let mut args: Vec<String> = vec!["run".into(), "-s".into(), session_id.into()];
    if let Some(model) = model {
        args.push("-m".into());
        args.push(model.to_string());
    }
    if let Some(cwd) = cwd {
        // Python 的 `command[2:2] = [...]`：插在 argv[0]（可执行）与 "run" 之后。
        args.splice(1..1, ["--dir".to_string(), cwd.to_string()]);
    }
    args.push(probes::PROBE_PROMPT.to_string());
    let mut argv = vec![executables::argv("opencode", &[])
        .into_iter()
        .next()
        .unwrap_or_else(|| "opencode".to_string())];
    argv.extend(args);

    let output = probes::run(&argv, cwd.map(Path::new), PROBE_TIMEOUT, None)
        .map_err(|error| DomainError::probe_timeout(error.message))?;
    if output.returncode != Some(0) {
        let mut params = Map::new();
        params.insert("tool".into(), Value::from("opencode"));
        params.insert(
            "exit_code".into(),
            output.returncode.map_or(Value::Null, Value::from),
        );
        return Ok(probes::report(
            "failed",
            Some("probe.process_failed"),
            Some(params),
            &output.stdout,
            &output.stderr,
        ));
    }
    if !probes::response_matches(Some(&output.stdout)) {
        let mut params = Map::new();
        params.insert("tool".into(), Value::from("opencode"));
        return Ok(probes::report(
            "failed",
            Some("probe.unexpected_response"),
            Some(params),
            &output.stdout,
            &output.stderr,
        ));
    }
    Ok(probes::report(
        "passed",
        None,
        None,
        &output.stdout,
        &output.stderr,
    ))
}

/// `opencode run --format json --auto` 的一次真实提问。
fn prompt_session(
    session_id: &str,
    cwd: Option<&str>,
    prompt: &str,
    model: Option<&str>,
    timeout: u64,
) -> DomainResult<Map<String, Value>> {
    let working_dir = cwd.filter(|value| !value.is_empty()).unwrap_or(".");
    let mut args: Vec<&str> = vec![
        "run",
        "-s",
        session_id,
        "--dir",
        working_dir,
        "--format",
        "json",
        "--auto",
    ];
    if let Some(model) = model {
        args.push("-m");
        args.push(model);
    }
    args.push(prompt);
    let argv = executables::argv("opencode", &args);
    let result =
        probes::run_agent_command(&argv, Some(Path::new(working_dir)), None, timeout, None)
            .map_err(DomainError::internal)?;

    let mut params = Map::new();
    params.insert("tool".into(), Value::from("opencode"));
    params.insert(
        "exit_code".into(),
        result.returncode.map_or(Value::Null, Value::from),
    );
    let (status, code, text) = if result.timed_out {
        ("failed", Some("agent_prompt.timeout"), None)
    } else if result.returncode != Some(0) {
        ("failed", Some("agent_prompt.process_failed"), None)
    } else {
        let (text, process_failed) = parse_prompt_output(&result.stdout);
        if process_failed {
            ("failed", Some("agent_prompt.process_failed"), None)
        } else if text.is_none() {
            ("failed", Some("agent_prompt.invalid_output"), None)
        } else {
            ("completed", None, text)
        }
    };
    let report = probes::report(status, code, Some(params), &result.stdout, &result.stderr);
    let mut payload = serde_json::to_value(&report)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    let (normalized, truncated) = probes::normalize_agent_text(text.as_deref());
    payload.insert("text".into(), Value::from(normalized));
    payload.insert("text_truncated".into(), Value::Bool(truncated));
    Ok(payload)
}

/// OpenCode 的验收探针组件。
pub struct OpenCodeVerifier;

impl SessionVerifier for OpenCodeVerifier {
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
        editor: &dyn SessionEditor,
        doc: &EditDocument,
        result: &Map<String, Value>,
        model: Option<&str>,
    ) -> DomainResult<ProbeReport> {
        let session_id = result
            .get("session_id")
            .and_then(Value::as_str)
            .ok_or_else(|| DomainError::internal("OpenCode 提交结果缺少 session_id"))?;
        let authored = editor.load(session_id)?;
        let authored_data = authored
            .data
            .downcast_ref::<OpenCodeData>()
            .ok_or_else(|| DomainError::internal("OpenCode 编辑文档载荷类型不符"))?;
        let doc_data = doc
            .data
            .downcast_ref::<OpenCodeData>()
            .ok_or_else(|| DomainError::internal("OpenCode 编辑文档载荷类型不符"))?;
        let cwd = doc_data
            .data
            .get("info")
            .and_then(|info| info.get("directory"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or(".")
            .to_string();

        let mut payloads = BTreeMap::new();
        payloads.insert(
            authored_data.tree.source_id.clone(),
            authored_data.data.clone(),
        );
        let shadow = writer::write(&authored_data.tree, Some(&cwd), None, Some(&payloads))?;
        let outcome = probe(&shadow.session_id, Some(&cwd), model);

        // 探针成败都要清理影子会话，且自底向上删。
        let ids = match reader::read(&shadow.session_id) {
            Ok(tree) => {
                let mut ids: Vec<String> = tree
                    .walk()
                    .iter()
                    .map(|node| node.source_id.clone())
                    .collect();
                ids.reverse();
                ids
            }
            Err(_) => vec![shadow.session_id.clone()],
        };
        for id in ids {
            let _ = store::delete_session(&id, None);
        }

        outcome.map(|report| report.with_isolation("shadow_session", &shadow.session_id, true))
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
    fn assistant_text_reads_every_event_shape() {
        assert_eq!(
            assistant_text(&json!({"type": "text", "part": {"text": "hello"}})),
            Some("hello".into())
        );
        assert_eq!(
            assistant_text(&json!({"type": "text", "text": [
                {"type": "text", "text": "a"},
                {"type": "image"},
                {"type": "text", "text": "b"}]})),
            Some("ab".into())
        );
        assert_eq!(
            assistant_text(&json!({"type": "assistant",
                                   "message": {"content": "answer"}})),
            Some("answer".into())
        );
        assert_eq!(
            assistant_text(&json!({"role": "assistant", "content": "direct"})),
            Some("direct".into())
        );
        assert_eq!(
            assistant_text(&json!({"type": "message.updated",
                                   "properties": {"info": {"role": "assistant",
                                                           "content": "late"}}})),
            Some("late".into())
        );
        // 非 assistant 的 message 不产出正文。
        assert_eq!(
            assistant_text(&json!({"type": "message", "message": {"role": "user",
                                                                  "content": "q"}})),
            None
        );
        assert_eq!(assistant_text(&json!({"type": "tool"})), None);
    }

    #[test]
    fn prompt_output_takes_the_last_assistant_text() {
        let raw = "{\"type\":\"text\",\"part\":{\"text\":\"first\"}}\n\
                   \n\
                   {\"type\":\"tool\"}\n\
                   {\"type\":\"text\",\"part\":{\"text\":\"last\"}}\n";
        assert_eq!(parse_prompt_output(raw), (Some("last".into()), false));
    }

    #[test]
    fn error_events_and_malformed_lines_are_reported_separately() {
        // error 事件 → process_failed。
        assert_eq!(
            parse_prompt_output("{\"type\":\"error\",\"message\":\"boom\"}"),
            (None, true)
        );
        // 非 JSON / 非对象 → 直接判进程失败（不是 error 事件）。
        assert_eq!(parse_prompt_output("not json"), (None, false));
        assert_eq!(parse_prompt_output("[1,2]"), (None, false));
        // 空输出。
        assert_eq!(parse_prompt_output("   \n"), (None, false));
        // 有事件但无正文 → invalid_output（text 为 None、未失败）。
        assert_eq!(parse_prompt_output("{\"type\":\"tool\"}"), (None, false));
    }
}
