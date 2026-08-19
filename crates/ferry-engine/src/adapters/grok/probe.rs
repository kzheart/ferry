//! 用官方 export/list/search 命令验收 Grok bundle。
//!
//! 验收在一个**临时 GROK_HOME 沙箱**里做：把待验收的 bundle 拷进去、建好索引，
//! 再让真实的 `grok` CLI 导出、列出、检索一遍。三条命令全过才算通过。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use regex::Regex;
use serde_json::{Map, Value};

use crate::adapters::contracts::{SessionEditor, SessionVerifier};
use crate::adapters::shared::editing::EditDocument;
use crate::errors::{DomainError, DomainResult};
use crate::system::paths::process_environ;
use crate::system::probes::{self, ProbeReport, ProbeTimeout};
use crate::system::{executables, probes as probe_system};

use super::store::read_text;

/// 用于 `grok sessions search` 的哨兵词：标题里第一个长度 ≥3 的 word。
static WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\w-]+").expect("哨兵词正则合法"));

// ---------------------------------------------------------------------------
// 测试注入口
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) type TestProbe = fn(&Path) -> DomainResult<ProbeReport>;

#[cfg(test)]
static TEST_PROBE: std::sync::Mutex<Option<TestProbe>> = std::sync::Mutex::new(None);

/// 注入口是进程级的，用它的测试必须互斥，否则并行跑的用例会互相换掉探针。
#[cfg(test)]
static PROBE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 单测里替换 `probe_bundle`（等价 Python 测试的 `monkeypatch.setattr`）。
///
/// 返回的守卫持有互斥锁，析构即恢复真实实现；写链路的单测不该真的去拉 grok CLI。
#[cfg(test)]
pub(crate) struct ProbeGuard(#[allow(dead_code)] std::sync::MutexGuard<'static, ()>);

#[cfg(test)]
impl Drop for ProbeGuard {
    fn drop(&mut self) {
        *TEST_PROBE.lock().expect("探针注入口锁中毒") = None;
    }
}

#[cfg(test)]
pub(crate) fn install_test_probe(probe: TestProbe) -> ProbeGuard {
    let lock = PROBE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    *TEST_PROBE.lock().expect("探针注入口锁中毒") = Some(probe);
    ProbeGuard(lock)
}

#[cfg(test)]
fn test_probe() -> Option<TestProbe> {
    *TEST_PROBE.lock().expect("探针注入口锁中毒")
}

// ---------------------------------------------------------------------------
// bundle 验收
// ---------------------------------------------------------------------------

fn read_summary(bundle: &Path) -> DomainResult<Value> {
    let path = bundle.join("summary.json");
    read_text(&path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .ok_or_else(|| DomainError::internal(format!("Grok summary 不可读: {}", path.display())))
}

fn text_field(summary: &Value, key: &str) -> String {
    summary
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn copy_tree(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn failed(code: &str, stdout: &str, stderr: &str, exit_code: Option<i32>) -> ProbeReport {
    let mut params = Map::new();
    params.insert("tool".into(), Value::from("grok"));
    if let Some(exit_code) = exit_code {
        params.insert("exit_code".into(), Value::from(exit_code));
    }
    probes::report("failed", Some(code), Some(params), stdout, stderr)
}

/// 在临时 GROK_HOME 里跑 export/list/search 三连验收。
pub fn probe_bundle(bundle: &Path) -> DomainResult<ProbeReport> {
    #[cfg(test)]
    if let Some(probe) = test_probe() {
        return probe(bundle);
    }
    let summary = read_summary(bundle)?;
    let info = summary.get("info").cloned().unwrap_or(Value::Null);
    let sid = info
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let cwd = info
        .get("cwd")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let title = {
        let generated = text_field(&summary, "generated_title");
        let fallback = text_field(&summary, "session_summary");
        if !generated.is_empty() {
            generated
        } else if !fallback.is_empty() {
            fallback
        } else {
            sid.clone()
        }
    };
    let sentinel = WORD
        .find_iter(&title)
        .map(|found| found.as_str())
        .find(|token| token.chars().count() >= 3)
        .unwrap_or("Migrated")
        .to_string();

    let sandbox = tempfile::tempdir()
        .map_err(|error| DomainError::internal(format!("创建 Grok 验收沙箱失败: {error}")))?;
    let home = sandbox.path().to_path_buf();
    let sessions = home.join("sessions");
    let mut command_cwd = PathBuf::from(&cwd);
    if !command_cwd.is_dir() {
        command_cwd = home.join("probe-cwd");
        fs::create_dir_all(&command_cwd)
            .map_err(|error| DomainError::internal(format!("创建验收工作目录失败: {error}")))?;
    }
    let resolved_cwd = fs::canonicalize(&command_cwd).unwrap_or_else(|_| command_cwd.clone());
    let target = sessions
        .join(utf8_percent_encode(&resolved_cwd.to_string_lossy(), NON_ALPHANUMERIC).to_string())
        .join(&sid);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| DomainError::internal(format!("创建验收目录失败: {error}")))?;
    }
    copy_tree(bundle, &target)
        .map_err(|error| DomainError::internal(format!("拷贝待验收 bundle 失败: {error}")))?;
    if command_cwd.to_string_lossy() != cwd {
        // cwd 已经不存在时用影子目录跑 CLI，摘要里的 cwd 也要跟着改，否则
        // `grok sessions list` 找不到这个会话。
        let mut shadow = read_summary(&target)?;
        shadow["info"]["cwd"] = Value::from(command_cwd.to_string_lossy().as_ref());
        fs::write(
            target.join("summary.json"),
            crate::adapters::shared::writing::python_json_dumps(&shadow),
        )
        .map_err(|error| DomainError::internal(format!("改写影子摘要失败: {error}")))?;
    }
    super::writer::index_bundle(&target, &sessions)?;

    let mut environment: Vec<(String, String)> = process_environ().into_iter().collect();
    environment.retain(|(key, _)| key != "GROK_HOME");
    environment.push(("GROK_HOME".into(), home.to_string_lossy().into_owned()));
    let export_path = home.join("export.md");
    let commands = [
        executables::argv("grok", &["export", &sid, &export_path.to_string_lossy()]),
        executables::argv("grok", &["sessions", "list"]),
        executables::argv("grok", &["sessions", "search", &sentinel]),
    ];
    let mut outputs: Vec<String> = Vec::new();
    for command in &commands {
        let result = match probe_system::run(
            command,
            Some(&command_cwd),
            Duration::from_secs(30),
            Some(&environment),
        ) {
            Ok(result) => result,
            Err(error) => return Ok(probes::timeout_report("grok", &error)),
        };
        outputs.push(format!("{}{}", result.stdout, result.stderr));
        if result.returncode != Some(0) {
            return Ok(failed(
                "probe.process_failed",
                &result.stdout,
                &result.stderr,
                result.returncode,
            ));
        }
    }
    if !export_path.is_file() || !outputs[1].contains(&sid) || !outputs[2].contains(&sid) {
        return Ok(failed(
            "probe.unexpected_response",
            &outputs.join("\n"),
            "",
            None,
        ));
    }
    Ok(probes::report(
        "passed",
        None,
        None,
        &outputs.join("\n"),
        "",
    ))
}

/// 对一个已存在的会话跑一轮真实 prompt。
fn prompt_session(
    session_id: &str,
    cwd: &str,
    prompt: &str,
    model: Option<&str>,
    timeout: u64,
) -> DomainResult<Map<String, Value>> {
    let bundle = super::adapter::resolve(session_id)?;
    let summary = read_summary(&bundle)?;
    let native_id = summary
        .get("info")
        .and_then(|info| info.get("id"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let mut command = executables::argv(
        "grok",
        &[
            "--no-auto-update",
            "--cwd",
            cwd,
            "--resume",
            &native_id,
            "--single",
            prompt,
            "--verbatim",
            "--output-format",
            "json",
            "--always-approve",
        ],
    );
    if let Some(model) = model {
        command.push("--model".into());
        command.push(model.to_string());
    }
    let result =
        probe_system::run_agent_command(&command, Some(Path::new(cwd)), None, timeout, None)
            .map_err(DomainError::internal)?;

    let mut params = Map::new();
    params.insert("tool".into(), Value::from("grok"));
    params.insert(
        "exit_code".into(),
        result.returncode.map_or(Value::Null, Value::from),
    );
    let (status, code, text) = if result.timed_out {
        ("failed", Some("agent_prompt.timeout"), String::new())
    } else {
        let raw = result.stdout.trim();
        let output: Option<Value> = if raw.is_empty() {
            None
        } else {
            serde_json::from_str(raw).ok()
        };
        match output.as_ref().and_then(Value::as_object) {
            None => {
                let code = if result.returncode != Some(0) {
                    "agent_prompt.process_failed"
                } else {
                    "agent_prompt.invalid_output"
                };
                ("failed", Some(code), String::new())
            }
            Some(output) => {
                for (source, target) in [
                    ("stopReason", "stop_reason"),
                    ("sessionId", "session_id"),
                    ("requestId", "request_id"),
                ] {
                    if let Some(value) = output.get(source).filter(|value| !value.is_null()) {
                        params.insert(target.into(), value.clone());
                    }
                }
                if result.returncode != Some(0) || output.get("type") == Some(&Value::from("error"))
                {
                    ("failed", Some("agent_prompt.process_failed"), String::new())
                } else {
                    match output.get("text").and_then(Value::as_str) {
                        None => ("failed", Some("agent_prompt.invalid_output"), String::new()),
                        Some(text) => ("completed", None, text.to_string()),
                    }
                }
            }
        }
    };
    let report = probes::report(status, code, Some(params), &result.stdout, &result.stderr);
    let (normalized, truncated) = probes::normalize_agent_text(Some(&text));
    let mut payload = serde_json::to_value(&report)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    payload.insert("text".into(), Value::from(normalized));
    payload.insert("text_truncated".into(), Value::Bool(truncated));
    Ok(payload)
}

/// [`SessionVerifier`] 的 grok 实现。
pub struct GrokVerifier;

impl SessionVerifier for GrokVerifier {
    fn probe(
        &self,
        session_id: &str,
        _cwd: Option<&str>,
        _model: Option<&str>,
    ) -> DomainResult<ProbeReport> {
        probe_bundle(&super::adapter::resolve(session_id)?)
    }

    fn probe_edited(
        &self,
        _editor: &dyn SessionEditor,
        _doc: &EditDocument,
        result: &Map<String, Value>,
        _model: Option<&str>,
    ) -> DomainResult<ProbeReport> {
        let saved_as = result
            .get("saved_as")
            .and_then(Value::as_str)
            .ok_or_else(|| DomainError::internal("编辑结果缺少 saved_as"))?;
        probe_bundle(Path::new(saved_as))
    }

    fn prompt_session(
        &self,
        session_id: &str,
        cwd: Option<&str>,
        prompt: &str,
        model: Option<&str>,
        timeout: u64,
    ) -> DomainResult<Map<String, Value>> {
        let cwd = cwd.ok_or_else(|| DomainError::agent_request_invalid("grok 对话需要工作目录"))?;
        prompt_session(session_id, cwd, prompt, model, timeout)
    }
}

/// 显式再导出，方便调用方在不引入 system::probes 的情况下匹配超时类型。
pub type Timeout = ProbeTimeout;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_search_sentinel_takes_the_first_long_word() {
        let sentinel = |title: &str| -> String {
            WORD.find_iter(title)
                .map(|found| found.as_str())
                .find(|token| token.chars().count() >= 3)
                .unwrap_or("Migrated")
                .to_string()
        };
        assert_eq!(sentinel("a b sentinel-grok"), "sentinel-grok");
        assert_eq!(sentinel("迁移会话"), "迁移会话");
        assert_eq!(sentinel("a b"), "Migrated");
        assert_eq!(sentinel(""), "Migrated");
    }

    #[test]
    fn the_test_probe_replaces_the_real_cli() {
        let root = tempfile::tempdir().unwrap();
        {
            let _guard = install_test_probe(|_| Ok(probes::report("passed", None, None, "", "")));
            assert_eq!(probe_bundle(root.path()).unwrap().status, "passed");
        }
        // 守卫析构后回到真实实现：目录里没有 summary.json，直接报错。
        assert!(probe_bundle(root.path()).is_err());
    }

    #[test]
    fn a_failed_report_carries_the_exit_code() {
        let report = failed("probe.process_failed", "out", "err", Some(2));
        assert_eq!(report.status, "failed");
        assert_eq!(report.code.as_deref(), Some("probe.process_failed"));
        assert_eq!(report.params["exit_code"], Value::from(2));
        assert_eq!(report.diagnostic.stderr, "err");
    }
}
