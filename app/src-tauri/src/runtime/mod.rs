mod approval;
mod gateway;
mod tool_routes;

use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Duration;
use tauri::{Emitter, Manager};

use crate::contracts::events::{event_policy, EventSource};
use crate::contracts::ipc::{FERRY_CONTRACT_HASH, FERRY_IPC_PROTOCOL};
use crate::contracts::runtime_methods;
use crate::process::client::{JsonlProcessClient, PendingResponses};
use crate::process::error::ProcessError;
use crate::process::framing::JsonlWriter;
use crate::process::supervisor::{ManagedProcess, ProcessSupervisor};
use approval::{forget_auto_policy, remember_auto_policy};
use gateway::{complete_engine_request, complete_tool_request};

const MAX_COMMAND_BYTES: usize = 16 * 1024 * 1024;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
struct RuntimeClient {
    generation: u64,
    transport: JsonlProcessClient,
}

type RuntimeProcess = ManagedProcess<RuntimeClient>;

static RUNTIME_PROCESS: OnceLock<ProcessSupervisor<RuntimeClient>> = OnceLock::new();
static RUNTIME_GENERATION_SEQUENCE: AtomicU64 = AtomicU64::new(1);

fn emit_host_event(app: &tauri::AppHandle, event: Value) {
    let Some(event_type) = event.get("type").and_then(Value::as_str) else {
        return;
    };
    let Some(policy) = event_policy(event_type) else {
        return;
    };
    if policy.source == EventSource::Host && policy.forward_to_ui {
        let _ = app.emit("ferry-runtime-event", event);
    }
}

fn next_id(prefix: &str) -> String {
    format!(
        "{prefix}_{}_{}",
        std::process::id(),
        REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

fn spawn_runtime(app: &tauri::AppHandle, resource_dir: &Path) -> Result<RuntimeProcess, String> {
    let mut command = runtime_binary_command(resource_dir)?;
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
        .home_dir()
        .map_err(|error| error.to_string())?
        .join(".ferry");
    std::fs::create_dir_all(&data_dir).map_err(|error| error.to_string())?;
    command.env("FERRY_RUNTIME_DATA_DIR", data_dir);
    crate::desktop::platform::configure_background_command(&mut command);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|error| format!("启动 Ferry Runtime 失败: {error}"))?;
    let stdin = child.stdin.take().ok_or("Ferry Runtime stdin 不可用")?;
    let stdout = child.stdout.take().ok_or("Ferry Runtime stdout 不可用")?;
    let transport = JsonlProcessClient::new("Ferry Runtime", stdin);
    let reader_stdin = transport.writer();
    let reader_pending = transport.pending();
    let reader_app = app.clone();
    let reader_resource = resource_dir.to_owned();
    std::thread::spawn(move || {
        read_runtime_output(
            reader_app,
            reader_resource,
            BufReader::new(stdout),
            reader_stdin,
            reader_pending,
        )
    });
    let generation = RUNTIME_GENERATION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let client = RuntimeClient {
        generation,
        transport,
    };
    let process = ManagedProcess::new(generation, child, client.clone());
    let health = json!({
        "protocol": FERRY_IPC_PROTOCOL,
        "id": next_id("health"),
        "method": "health",
        "params": {},
    });
    let health_id = health
        .get("id")
        .and_then(Value::as_str)
        .expect("health request has id");
    let response = client
        .transport
        .request(health_id, &health.to_string(), Duration::from_secs(10))
        .map_err(|error| error.to_string())?;
    let value: Value = serde_json::from_str(&response).map_err(|error| error.to_string())?;
    if value.get("ok").and_then(Value::as_bool) != Some(true)
        || value.pointer("/result/service").and_then(Value::as_str) != Some("ferry-runtime")
        || value
            .pointer("/result/contract_hash")
            .and_then(Value::as_str)
            != Some(FERRY_CONTRACT_HASH)
    {
        return Err("Ferry Runtime 协议握手失败".to_owned());
    }
    Ok(process)
}

fn read_runtime_output(
    app: tauri::AppHandle,
    resource_dir: PathBuf,
    mut stdout: impl BufRead,
    stdin: JsonlWriter,
    pending: PendingResponses,
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
        if let Some(event_type) = value.get("type").and_then(Value::as_str) {
            let Some(policy) = event_policy(event_type) else {
                continue;
            };
            if policy.source != EventSource::Runtime {
                continue;
            }
            if event_type == "engine.request" {
                let worker_resource = resource_dir.clone();
                let worker_stdin = stdin.clone();
                std::thread::spawn(move || {
                    complete_engine_request(&worker_resource, &worker_stdin, &value)
                });
                continue;
            }
            if matches!(
                Some(event_type),
                Some("run.completed" | "run.failed" | "run.cancelled")
            ) {
                if let Some(session_id) = value.get("session_id").and_then(Value::as_str) {
                    forget_auto_policy(session_id);
                }
            }
            if policy.forward_to_ui {
                let _ = app.emit("ferry-runtime-event", &value);
            }
            if event_type == "tool.request" {
                let worker_app = app.clone();
                let worker_resource = resource_dir.clone();
                let worker_stdin = stdin.clone();
                std::thread::spawn(move || {
                    complete_tool_request(&worker_app, &worker_resource, &worker_stdin, &value)
                });
            }
            continue;
        }
        if let Some(id) = value.get("id").and_then(Value::as_str) {
            pending.complete(id, trimmed.to_owned());
        }
    }
    pending.fail_all(crate::process::error::ProcessError::Exited(
        "Ferry Runtime 进程已退出".to_owned(),
    ));
    emit_host_event(
        &app,
        json!({
            "protocol": FERRY_IPC_PROTOCOL,
            "type": "runtime.disconnected",
            "payload": {},
        }),
    );
}

fn request_runtime(
    app: &tauri::AppHandle,
    resource_dir: &Path,
    request: &str,
) -> Result<String, String> {
    let id = serde_json::from_str::<Value>(request)
        .ok()
        .and_then(|value| value.get("id").and_then(Value::as_str).map(str::to_owned))
        .ok_or("Runtime 命令缺少 id")?;
    let client = ensure_runtime(app, resource_dir)?;
    let result = client.transport.request(&id, request, COMMAND_TIMEOUT);
    if result
        .as_ref()
        .is_err_and(ProcessError::invalidates_process)
    {
        invalidate_runtime(client.generation);
    }
    result.map_err(|error| error.to_string())
}

fn ensure_runtime(app: &tauri::AppHandle, resource_dir: &Path) -> Result<RuntimeClient, String> {
    RUNTIME_PROCESS
        .get_or_init(|| ProcessSupervisor::new("Runtime"))
        .ensure(|| spawn_runtime(app, resource_dir))
}

fn invalidate_runtime(generation: u64) {
    RUNTIME_PROCESS
        .get_or_init(|| ProcessSupervisor::new("Runtime"))
        .invalidate(generation);
}

fn validate_public_command(request: &str) -> Result<(), String> {
    if request.len() > MAX_COMMAND_BYTES || request.contains('\n') || request.contains('\r') {
        return Err("Runtime 命令 framing 非法".to_owned());
    }
    let value: Value = serde_json::from_str(request).map_err(|error| error.to_string())?;
    if value.get("protocol").and_then(Value::as_str) != Some(FERRY_IPC_PROTOCOL) {
        return Err("Agent 协议不兼容".to_owned());
    }
    let method = value.get("method").and_then(Value::as_str).unwrap_or("");
    if !runtime_methods::is_public(method) {
        return Err("Runtime 命令不允许从前端调用".to_owned());
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn agent_command(
    app: tauri::AppHandle,
    request: String,
) -> Result<String, String> {
    validate_public_command(&request)?;
    remember_auto_policy(&request);
    let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || request_runtime(&app, &resource_dir, &request))
        .await
        .map_err(|e| e.to_string())?
}

pub(crate) fn warm_up(app: tauri::AppHandle, resource_dir: PathBuf) {
    std::thread::spawn(move || {
        let request = json!({"protocol": FERRY_IPC_PROTOCOL, "id": next_id("warmup"),
                             "method": "health", "params": {}});
        let _ = request_runtime(&app, &resource_dir, &request.to_string());
    });
}

#[cfg(debug_assertions)]
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn runtime_binary_command(resource_dir: &Path) -> Result<Command, String> {
    let name = if cfg!(target_os = "windows") {
        "ferry-runtime.exe"
    } else {
        "ferry-runtime"
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
        command.arg(repo_root().join("ferry-runtime/dist/server/server.js"));
        command.current_dir(repo_root());
        Ok(command)
    }
    #[cfg(not(debug_assertions))]
    Err(format!(
        "正式包缺少 Ferry Runtime sidecar: {}",
        candidates
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("; ")
    ))
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontend_cannot_submit_tool_results() {
        let request = json!({
            "protocol": FERRY_IPC_PROTOCOL,
            "id": "x",
            "method": "tool.result",
            "params": {}
        });
        assert!(validate_public_command(&request.to_string()).is_err());
    }
}
