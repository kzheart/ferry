use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::Duration;
use tauri::{Emitter, Manager};

use crate::sidecar::engine_request_blocking;

const AGENT_PROTOCOL: &str = "ferry-agent/v1";
const MAX_COMMAND_BYTES: usize = 16 * 1024 * 1024;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

type PendingResult = Result<String, String>;
type Pending = Arc<Mutex<HashMap<String, mpsc::Sender<PendingResult>>>>;

struct AgentProcess {
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Pending,
}

impl Drop for AgentProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl AgentProcess {
    fn request(&mut self, request: &str, timeout: Duration) -> Result<String, String> {
        let value: Value = serde_json::from_str(request)
            .map_err(|error| format!("Agent 命令不是有效 JSON: {error}"))?;
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .ok_or("Agent 命令缺少 id")?
            .to_owned();
        let (sender, receiver) = mpsc::channel();
        self.pending
            .lock()
            .map_err(|_| "Agent pending 锁损坏".to_owned())?
            .insert(id.clone(), sender);
        if let Err(error) = write_line(&self.stdin, request) {
            self.pending.lock().ok().and_then(|mut map| map.remove(&id));
            return Err(error);
        }
        receiver.recv_timeout(timeout).map_err(|error| {
            self.pending.lock().ok().and_then(|mut map| map.remove(&id));
            format!("Agent 命令等待失败: {error}")
        })?
    }
}

static AGENT: OnceLock<Mutex<Option<AgentProcess>>> = OnceLock::new();

fn next_id(prefix: &str) -> String {
    format!(
        "{prefix}_{}_{}",
        std::process::id(),
        REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

fn write_line(stdin: &Arc<Mutex<ChildStdin>>, line: &str) -> Result<(), String> {
    let mut writer = stdin.lock().map_err(|_| "Agent stdin 锁损坏".to_owned())?;
    writer
        .write_all(line.as_bytes())
        .and_then(|_| writer.write_all(b"\n"))
        .and_then(|_| writer.flush())
        .map_err(|error| format!("写入 Agent 失败: {error}"))
}

fn spawn_agent(app: &tauri::AppHandle, resource_dir: &Path) -> Result<AgentProcess, String> {
    let mut command = agent_binary_command(resource_dir)?;
    command.env_clear();
    for name in [
        "PATH",
        "HOME",
        "USERPROFILE",
        "TMPDIR",
        "TEMP",
        "TMP",
        "SystemRoot",
        "WINDIR",
        "LANG",
        "LC_ALL",
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "NO_PROXY",
        "SSL_CERT_FILE",
        "NODE_EXTRA_CA_CERTS",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("agent-runtime");
    std::fs::create_dir_all(&data_dir).map_err(|error| error.to_string())?;
    command.env("FERRY_AGENT_DATA_DIR", data_dir);
    hide_console(&mut command);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|error| format!("启动 Agent 失败: {error}"))?;
    let stdin = Arc::new(Mutex::new(child.stdin.take().ok_or("Agent stdin 不可用")?));
    let stdout = child.stdout.take().ok_or("Agent stdout 不可用")?;
    let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
    let reader_stdin = stdin.clone();
    let reader_pending = pending.clone();
    let reader_app = app.clone();
    let reader_resource = resource_dir.to_owned();
    std::thread::spawn(move || {
        read_agent_output(
            reader_app,
            reader_resource,
            BufReader::new(stdout),
            reader_stdin,
            reader_pending,
        )
    });
    Ok(AgentProcess {
        child,
        stdin,
        pending,
    })
}

fn read_agent_output(
    app: tauri::AppHandle,
    resource_dir: PathBuf,
    mut stdout: impl BufRead,
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Pending,
) {
    let mut line = String::new();
    loop {
        line.clear();
        match stdout.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }
        let trimmed = line.trim_end();
        let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        if value.get("type").is_some() {
            let _ = app.emit("ferry-agent-event", &value);
            if value.get("type").and_then(Value::as_str) == Some("tool.request") {
                let worker_resource = resource_dir.clone();
                let worker_stdin = stdin.clone();
                std::thread::spawn(move || {
                    complete_tool_request(&worker_resource, &worker_stdin, &value)
                });
            }
            continue;
        }
        if let Some(id) = value.get("id").and_then(Value::as_str) {
            if let Some(sender) = pending.lock().ok().and_then(|mut map| map.remove(id)) {
                let _ = sender.send(Ok(trimmed.to_owned()));
            }
        }
    }
    if let Ok(mut waiters) = pending.lock() {
        for (_, sender) in waiters.drain() {
            let _ = sender.send(Err("Agent 进程已退出".to_owned()));
        }
    }
    let _ = app.emit(
        "ferry-agent-event",
        json!({"protocol": AGENT_PROTOCOL, "type": "runtime.disconnected"}),
    );
}

fn complete_tool_request(resource_dir: &Path, stdin: &Arc<Mutex<ChildStdin>>, event: &Value) {
    let session_id = event
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let run_id = event.get("run_id").and_then(Value::as_str).unwrap_or("");
    let payload = event.get("payload").and_then(Value::as_object);
    let request_id = payload
        .and_then(|value| value.get("request_id"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let name = payload
        .and_then(|value| value.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let args = payload
        .and_then(|value| value.get("args"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let outcome = route_tool(resource_dir, name, args, run_id);
    let params = match outcome {
        Ok(result) => json!({
            "request_id": request_id,
            "session_id": session_id,
            "ok": true,
            "result": result,
        }),
        Err(code) => json!({
            "request_id": request_id,
            "session_id": session_id,
            "ok": false,
            "error": code,
        }),
    };
    let command = json!({
        "protocol": AGENT_PROTOCOL,
        "id": next_id("tool_result"),
        "method": "tool.result",
        "params": params,
    });
    let _ = write_line(stdin, &command.to_string());
}

fn route_tool(
    resource_dir: &Path,
    name: &str,
    mut args: Map<String, Value>,
    run_id: &str,
) -> Result<Value, String> {
    let method = tool_method(name).ok_or_else(|| "tool.not_allowed".to_owned())?;
    if name.starts_with("ferry_propose_") {
        if run_id.is_empty() {
            return Err("agent.run_missing".to_owned());
        }
        args.insert("run_id".to_owned(), Value::String(run_id.to_owned()));
    }
    let request = json!({
        "method": method,
        "request_id": next_id("engine_tool"),
        "params": args,
    });
    let response = engine_request_blocking(resource_dir, &request.to_string())
        .map_err(|_| "engine.unavailable".to_owned())?;
    let envelope: Value =
        serde_json::from_str(&response).map_err(|_| "engine.invalid_response".to_owned())?;
    if envelope.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(envelope.get("result").cloned().unwrap_or(Value::Null))
    } else {
        Err(envelope
            .pointer("/error/code")
            .and_then(Value::as_str)
            .unwrap_or("engine.request_failed")
            .to_owned())
    }
}

fn tool_method(name: &str) -> Option<&'static str> {
    Some(match name {
        "ferry_list_capabilities" => "agent_list_capabilities",
        "ferry_search_sessions" => "agent_search_sessions",
        "ferry_get_session_context" => "agent_get_session_context",
        "ferry_get_usage" => "agent_get_usage",
        "ferry_preview_migration" => "agent_preview_migration",
        "ferry_preview_edit" => "agent_preview_edit",
        "ferry_propose_migration" => "agent_propose_migration",
        "ferry_propose_edit" => "agent_propose_edit",
        "ferry_propose_metadata_change" => "agent_propose_metadata_change",
        _ => return None,
    })
}

fn request_agent(
    app: &tauri::AppHandle,
    resource_dir: &Path,
    request: &str,
) -> Result<String, String> {
    let slot = AGENT.get_or_init(|| Mutex::new(None));
    let mut guard = slot.lock().map_err(|_| "Agent 状态锁损坏".to_owned())?;
    let exited = guard
        .as_mut()
        .and_then(|process| process.child.try_wait().ok().flatten())
        .is_some();
    if exited {
        *guard = None;
    }
    if guard.is_none() {
        let mut candidate = spawn_agent(app, resource_dir)?;
        let health = json!({
            "protocol": AGENT_PROTOCOL,
            "id": next_id("health"),
            "method": "health",
            "params": {},
        });
        let response = candidate.request(&health.to_string(), Duration::from_secs(10))?;
        let value: Value = serde_json::from_str(&response).map_err(|e| e.to_string())?;
        if value.get("ok").and_then(Value::as_bool) != Some(true)
            || value.pointer("/result/protocol").and_then(Value::as_str) != Some(AGENT_PROTOCOL)
        {
            return Err("Agent 协议握手失败".to_owned());
        }
        *guard = Some(candidate);
    }
    let result = guard
        .as_mut()
        .expect("agent ensured")
        .request(request, COMMAND_TIMEOUT);
    if result.is_err() {
        *guard = None;
    }
    result
}

fn validate_public_command(request: &str) -> Result<(), String> {
    if request.len() > MAX_COMMAND_BYTES || request.contains('\n') || request.contains('\r') {
        return Err("Agent 命令 framing 非法".to_owned());
    }
    let value: Value = serde_json::from_str(request).map_err(|error| error.to_string())?;
    if value.get("protocol").and_then(Value::as_str) != Some(AGENT_PROTOCOL) {
        return Err("Agent 协议不兼容".to_owned());
    }
    let method = value.get("method").and_then(Value::as_str).unwrap_or("");
    if !matches!(
        method,
        "health"
            | "session.create"
            | "prompt"
            | "abort"
            | "steer"
            | "follow_up"
            | "state"
            | "events.replay"
            | "providers.list"
            | "models.list"
            | "config.get"
            | "credential.set"
            | "provider.logout"
            | "model.select"
            | "custom_provider.upsert"
            | "custom_provider.delete"
            | "auth.login.start"
            | "auth.login.respond"
            | "auth.login.cancel"
    ) {
        return Err("Agent 命令不允许从前端调用".to_owned());
    }
    Ok(())
}

fn engine_value(resource_dir: &Path, method: &str, params: Value) -> Result<Value, String> {
    let request = json!({"method": method, "params": params,
                         "request_id": next_id("trusted")});
    let response = engine_request_blocking(resource_dir, &request.to_string())?;
    let envelope: Value = serde_json::from_str(&response).map_err(|e| e.to_string())?;
    if envelope.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(envelope.get("result").cloned().unwrap_or(Value::Null))
    } else {
        Err(envelope
            .pointer("/error/code")
            .and_then(Value::as_str)
            .unwrap_or("engine.request_failed")
            .to_owned())
    }
}

#[tauri::command]
pub(crate) async fn agent_command(
    app: tauri::AppHandle,
    request: String,
) -> Result<String, String> {
    validate_public_command(&request)?;
    let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || request_agent(&app, &resource_dir, &request))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(crate) async fn agent_operation_detail(
    app: tauri::AppHandle,
    operation_id: String,
) -> Result<Value, String> {
    let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        engine_value(
            &resource_dir,
            "agent_operation_detail",
            json!({"operation_id": operation_id}),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(crate) async fn agent_operation_approve_and_apply(
    app: tauri::AppHandle,
    operation_id: String,
    run_id: String,
) -> Result<Value, String> {
    let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        let approval = engine_value(
            &resource_dir,
            "agent_operation_authorize",
            json!({"operation_id": operation_id, "run_id": run_id}),
        )?;
        let token = approval
            .get("approval_token")
            .and_then(Value::as_str)
            .ok_or("Engine 未返回审批凭证")?;
        engine_value(
            &resource_dir,
            "agent_operation_apply",
            json!({"operation_id": operation_id, "run_id": run_id,
                   "approval_token": token}),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(crate) async fn agent_operation_status(
    app: tauri::AppHandle,
    operation_id: String,
) -> Result<Value, String> {
    let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        engine_value(
            &resource_dir,
            "agent_operation_status",
            json!({"operation_id": operation_id}),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

pub(crate) fn warm_up(app: tauri::AppHandle, resource_dir: PathBuf) {
    std::thread::spawn(move || {
        let request = json!({"protocol": AGENT_PROTOCOL, "id": next_id("warmup"),
                             "method": "health", "params": {}});
        let _ = request_agent(&app, &resource_dir, &request.to_string());
    });
}

#[cfg(debug_assertions)]
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn agent_binary_command(resource_dir: &Path) -> Result<Command, String> {
    let name = if cfg!(target_os = "windows") {
        "ferry-agent.exe"
    } else {
        "ferry-agent"
    };
    let candidates = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join(name)))
        .into_iter()
        .chain(std::iter::once(resource_dir.join(name)))
        .collect::<Vec<_>>();
    if let Some(binary) = candidates.iter().find(|path| path.is_file()) {
        return Ok(Command::new(binary));
    }
    #[cfg(debug_assertions)]
    {
        let mut command = Command::new("node");
        command.arg(repo_root().join("agent-runtime/dist/cli.js"));
        command.current_dir(repo_root());
        Ok(command)
    }
    #[cfg(not(debug_assertions))]
    Err(format!(
        "正式包缺少 Agent sidecar: {}",
        candidates
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("; ")
    ))
}

#[cfg(target_os = "windows")]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x0800_0000);
}

#[cfg(not(target_os = "windows"))]
fn hide_console(_command: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_gateway_is_an_exact_allowlist() {
        assert_eq!(
            tool_method("ferry_propose_edit"),
            Some("agent_propose_edit")
        );
        assert_eq!(tool_method("agent_operation_apply"), None);
        assert_eq!(tool_method("shell"), None);
    }

    #[test]
    fn frontend_cannot_submit_tool_results() {
        let request = json!({"protocol": AGENT_PROTOCOL, "id": "x",
                             "method": "tool.result", "params": {}});
        assert!(validate_public_command(&request.to_string()).is_err());
    }
}
