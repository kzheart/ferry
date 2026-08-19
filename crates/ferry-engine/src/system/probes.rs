//! CLI probing primitives shared by adapter-owned verifiers.
//!
//! 语义事实源：`engine/system/probes.py`。
//!
//! 返回结构化报告：status/code/params 承载业务判定；
//! stdout/stderr 是 opaque diagnostic，不翻译、不参与判定。

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Map, Value};

pub const PROBE_TOKEN: &str = "PROBE_OK";

/// 探针提示词；文案逐字保留（各 adapter 的验收断言依赖它）。
pub const PROBE_PROMPT: &str = concat!(
    "Runtime validation only. Do not explain, use tools, or add formatting. ",
    "Your entire response must be exactly this single token: PROBE_OK"
);

/// diagnostic 截断上限（按**字符**计，与 Python 的字符串切片一致）。
const DIAG_LIMIT: usize = 8000;
/// agent 输出截断上限（按字符计）。
const AGENT_TEXT_LIMIT: usize = 65536;

/// 探针超时。RPC 层会把它翻成 `probe.timeout` 幽灵错误码。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProbeTimeout {
    pub message: String,
}

impl std::fmt::Display for ProbeTimeout {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ProbeTimeout {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentProcessResult {
    pub returncode: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

/// 一次普通子进程调用的结果（对齐 `subprocess.CompletedProcess` 的用到部分）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandOutput {
    pub returncode: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// 探针诊断信息。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Diagnostic {
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
}

/// 结构化探针报告；序列化形状与 Python 的 dict 逐字段一致。
///
/// `isolation` 是 Python 侧 `probe_edited` / `_isolated_probe` 往报告 dict 上
/// **顶层**追加的扩展键（`rep["isolation"] = {...}`），前端
/// `app/src/shared/contracts/events.js::probeText` 读的就是 `p.isolation`——
/// 它是 wire 面的一部分，不能降级成 `params` 里的子键。未做隔离时整个键不出现。
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProbeReport {
    pub status: String,
    pub code: Option<String>,
    pub params: Map<String, Value>,
    pub diagnostic: Diagnostic,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isolation: Option<Map<String, Value>>,
}

impl ProbeReport {
    /// 追加 `isolation` 描述（链式，便于 `probe_edited` 的 `rep["isolation"] = ...`）。
    pub fn with_isolation(mut self, kind: &str, id: &str, cleaned: bool) -> Self {
        let mut isolation = Map::new();
        isolation.insert("kind".into(), Value::from(kind));
        isolation.insert("id".into(), Value::from(id));
        isolation.insert("cleaned".into(), Value::Bool(cleaned));
        self.isolation = Some(isolation);
        self
    }
}

fn truncate_chars(text: &str, limit: usize) -> (String, bool) {
    let mut characters = text.chars();
    let head: String = characters.by_ref().take(limit).collect();
    (head, characters.next().is_some())
}

/// `report(status, code, params, stdout, stderr)` 的等价物。
pub fn report(
    status: &str,
    code: Option<&str>,
    params: Option<Map<String, Value>>,
    stdout: &str,
    stderr: &str,
) -> ProbeReport {
    let (clipped_stdout, stdout_truncated) = truncate_chars(stdout, DIAG_LIMIT);
    let (clipped_stderr, stderr_truncated) = truncate_chars(stderr, DIAG_LIMIT);
    ProbeReport {
        status: status.to_string(),
        code: code.map(str::to_string),
        params: params.unwrap_or_default(),
        diagnostic: Diagnostic {
            stdout: clipped_stdout,
            stderr: clipped_stderr,
            truncated: stdout_truncated || stderr_truncated,
        },
        isolation: None,
    }
}

/// 超时报告：`probe.timeout` 是不在契约里的幽灵码，原样复刻。
pub fn timeout_report(tool: &str, error: &ProbeTimeout) -> ProbeReport {
    let mut params = Map::new();
    params.insert("tool".into(), Value::from(tool));
    report(
        "failed",
        Some("probe.timeout"),
        Some(params),
        "",
        &error.message,
    )
}

/// 截断 agent 文本并回报是否发生截断。
pub fn normalize_agent_text(value: Option<&str>) -> (String, bool) {
    truncate_chars(value.unwrap_or(""), AGENT_TEXT_LIMIT)
}

/// A resumed agent passes only when it returns the probe token exactly.
pub fn response_matches(stdout: Option<&str>) -> bool {
    stdout.unwrap_or("").trim() == PROBE_TOKEN
}

/// Windows 下抑制子进程闪现控制台窗口。
fn apply_run_flags(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = command;
    }
}

fn build_command(argv: &[String], cwd: Option<&Path>, env: Option<&[(String, String)]>) -> Command {
    let mut command = Command::new(&argv[0]);
    command.args(&argv[1..]);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    if let Some(env) = env {
        command.env_clear();
        for (key, value) in env {
            command.env(key, value);
        }
    }
    apply_run_flags(&mut command);
    command
}

/// `probes.run`：超时即抛 [`ProbeTimeout`]（默认 180 秒）。
pub fn run(
    argv: &[String],
    cwd: Option<&Path>,
    timeout: Duration,
    env: Option<&[(String, String)]>,
) -> Result<CommandOutput, ProbeTimeout> {
    let mut command = build_command(argv, cwd, env);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return Ok(CommandOutput {
                returncode: None,
                stdout: String::new(),
                stderr: error.to_string(),
            })
        }
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_reader = spawn_reader(stdout);
    let stderr_reader = spawn_reader(stderr);
    match wait_with_timeout(&mut child, timeout) {
        Some(status) => Ok(CommandOutput {
            returncode: status,
            stdout: stdout_reader.join().unwrap_or_default(),
            stderr: stderr_reader.join().unwrap_or_default(),
        }),
        None => {
            signal_process_group(&mut child, true);
            let _ = child.wait();
            Err(ProbeTimeout {
                message: format!("探针超时: {}", argv.join(" ")),
            })
        }
    }
}

/// `probes.run_agent_command`：独立进程组 + 超时先 TERM 后 KILL。
pub fn run_agent_command(
    argv: &[String],
    cwd: Option<&Path>,
    input_text: Option<&str>,
    timeout_secs: u64,
    env: Option<&[(String, String)]>,
) -> Result<AgentProcessResult, String> {
    if argv.is_empty() || argv.iter().any(String::is_empty) {
        return Err("Agent 命令必须是非空 argv".to_string());
    }
    if !(1..=360).contains(&timeout_secs) {
        return Err("Agent timeout 必须在 1..360 秒".to_string());
    }
    let mut command = build_command(argv, cwd, env);
    command
        .stdin(if input_text.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    new_process_group(&mut command);

    let mut child = command.spawn().map_err(|error| error.to_string())?;
    if let (Some(text), Some(mut stdin)) = (input_text, child.stdin.take()) {
        let payload = text.to_string();
        std::thread::spawn(move || {
            let _ = stdin.write_all(payload.as_bytes());
        });
    }
    let stdout_reader = spawn_reader(child.stdout.take());
    let stderr_reader = spawn_reader(child.stderr.take());

    let deadline = Duration::from_secs(timeout_secs);
    let (returncode, timed_out) = match wait_with_timeout(&mut child, deadline) {
        Some(status) => (status, false),
        None => {
            signal_process_group(&mut child, false);
            let status = match wait_with_timeout(&mut child, Duration::from_secs(2)) {
                Some(status) => status,
                None => {
                    signal_process_group(&mut child, true);
                    child.wait().ok().and_then(|status| status.code())
                }
            };
            (status, true)
        }
    };
    Ok(AgentProcessResult {
        returncode,
        stdout: stdout_reader.join().unwrap_or_default(),
        stderr: stderr_reader.join().unwrap_or_default(),
        timed_out,
    })
}

fn spawn_reader<R: Read + Send + 'static>(stream: Option<R>) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut buffer = Vec::new();
        if let Some(mut stream) = stream {
            let _ = stream.read_to_end(&mut buffer);
        }
        String::from_utf8_lossy(&buffer).into_owned()
    })
}

/// 轮询等待；标准库没有带超时的 `wait`。返回 `None` 表示超时。
fn wait_with_timeout(child: &mut Child, timeout: Duration) -> Option<Option<i32>> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status.code()),
            Ok(None) => {}
            Err(_) => return Some(None),
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(unix)]
fn new_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    // 等价 Python 的 `start_new_session=True`：超时后可以整组终止。
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn new_process_group(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
}

#[cfg(unix)]
fn signal_process_group(child: &mut Child, force: bool) {
    let signal = if force { libc::SIGKILL } else { libc::SIGTERM };
    // setsid 之后 pid == pgid；killpg 覆盖 CLI 自己拉起的全部子进程。
    unsafe {
        libc::killpg(child.id() as libc::pid_t, signal);
    }
}

#[cfg(not(unix))]
fn signal_process_group(child: &mut Child, force: bool) {
    if !force {
        // Windows 上非强制路径靠 CTRL_BREAK_EVENT；WP-B2 接 winapi 前先直接终止。
        let _ = child.kill();
        return;
    }
    let mut command = Command::new("taskkill");
    command
        .args(["/PID", &child.id().to_string(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    apply_run_flags(&mut command);
    let _ = command.status();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_prompt_embeds_the_token_verbatim() {
        assert!(PROBE_PROMPT.ends_with(PROBE_TOKEN));
        assert!(response_matches(Some("  PROBE_OK\n")));
        assert!(!response_matches(Some("PROBE_OK extra")));
        assert!(!response_matches(None));
    }

    #[test]
    fn diagnostics_truncate_by_characters() {
        let long = "中".repeat(DIAG_LIMIT + 5);
        let payload = report("failed", None, None, &long, "");
        assert_eq!(payload.diagnostic.stdout.chars().count(), DIAG_LIMIT);
        assert!(payload.diagnostic.truncated);

        let (text, truncated) = normalize_agent_text(Some(&"a".repeat(AGENT_TEXT_LIMIT)));
        assert_eq!(text.len(), AGENT_TEXT_LIMIT);
        assert!(!truncated);
        let (_, truncated) = normalize_agent_text(Some(&"a".repeat(AGENT_TEXT_LIMIT + 1)));
        assert!(truncated);
    }

    #[test]
    fn timeout_report_uses_the_ghost_code() {
        let payload = timeout_report(
            "claude",
            &ProbeTimeout {
                message: "探针超时: claude".into(),
            },
        );
        assert_eq!(payload.status, "failed");
        assert_eq!(payload.code.as_deref(), Some("probe.timeout"));
        assert_eq!(payload.params["tool"], Value::from("claude"));
    }

    #[test]
    fn agent_command_validates_its_arguments() {
        assert!(run_agent_command(&[], None, None, 10, None).is_err());
        assert!(run_agent_command(&["".to_string()], None, None, 10, None).is_err());
        assert!(run_agent_command(&["echo".to_string()], None, None, 0, None).is_err());
        assert!(run_agent_command(&["echo".to_string()], None, None, 361, None).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn agent_command_captures_stdout() {
        let result = run_agent_command(
            &["/bin/echo".to_string(), "PROBE_OK".to_string()],
            None,
            None,
            10,
            None,
        )
        .unwrap();
        assert!(!result.timed_out);
        assert!(response_matches(Some(&result.stdout)));
    }
}
