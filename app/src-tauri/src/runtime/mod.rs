mod approval;
pub(crate) mod bash;
pub(crate) mod choice;
mod gateway;
mod shell_platform;
mod tool_routes;

use serde::Serialize;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Duration;
use tauri::{Emitter, Manager};

use crate::contracts::events::{event_policy, EventSource};
use crate::contracts::features::Feature;
use crate::contracts::ipc::{FERRY_CONTRACT_HASH, FERRY_IPC_PROTOCOL};
use crate::contracts::runtime_methods;
use crate::process::client::{JsonlProcessClient, PendingResponses};
use crate::process::command::{bundled_sidecar_command, configure_background};
use crate::process::error::ProcessError;
use crate::process::framing::JsonlWriter;
use crate::process::handshake::verify_handshake;
use crate::process::logging::{host_log, sidecar_stderr};
use crate::process::supervisor::{ManagedProcess, ProcessSupervisor};
use approval::{forget_auto_policy, remember_auto_policy};
use gateway::{complete_engine_request, complete_tool_request};

const MAX_COMMAND_BYTES: usize = 16 * 1024 * 1024;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const STARTUP_HEALTH_TIMEOUT: Duration = Duration::from_secs(30);
static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
struct RuntimeClient {
    generation: u64,
    transport: JsonlProcessClient,
}

/// 给前端的失败。`Message` 保持原来的纯文本形状(untagged 序列化成裸字符串,前端
/// 已有的兜底展示不受影响),`Structured` 只用于需要按 code 分支的拒绝。
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(crate) enum RuntimeError {
    Structured {
        code: &'static str,
        feature: &'static str,
        message: String,
    },
    Message(String),
}

impl From<String> for RuntimeError {
    fn from(message: String) -> Self {
        Self::Message(message)
    }
}

impl From<&str> for RuntimeError {
    fn from(message: &str) -> Self {
        Self::Message(message.to_owned())
    }
}

/// 日志里只出现人话那一半,code 是给前端分支用的。
impl std::fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Structured { message, .. } | Self::Message(message) => {
                formatter.write_str(message)
            }
        }
    }
}

/// `builtin-agent` 特性的当前值。事实源是宿主的配置文件,不接受 WebView 传参;
/// 每次都回读,所以设置页一改就生效,不必重启 App。
#[cfg(not(test))]
fn agent_enabled() -> bool {
    crate::desktop::features::feature_enabled(Feature::BuiltinAgent)
}

/// 测试里不碰用户真实的 host-settings.json,用进程内的覆盖位注入开关。
#[cfg(test)]
static AGENT_ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(test)]
fn agent_enabled() -> bool {
    AGENT_ENABLED.load(Ordering::Relaxed)
}

/// 通往 runtime sidecar 的每条通道都要先过这道门。关着时不 spawn,也不去碰进程
/// 管理器;已经跑起来的进程不主动杀,跟着 App 的生命周期走(设置页有说明)。
///
/// 拒绝的形状对所有特性统一:`feature.disabled` + 是哪个特性,前端不必为每道门
/// 认一个新 code。
fn runtime_gate() -> Result<(), RuntimeError> {
    if agent_enabled() {
        return Ok(());
    }
    Err(RuntimeError::Structured {
        code: "feature.disabled",
        feature: Feature::BuiltinAgent.id(),
        message: "内置 AI 助手是测试中的功能,请先在设置里开启".to_owned(),
    })
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
    configure_background(&mut command);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(sidecar_stderr("runtime.log"));
    let mut child = command
        .spawn()
        .map_err(|error| format!("启动 Ferry Runtime 失败: {error}"))?;
    host_log("runtime", &format!("Runtime 进程已启动 pid={}", child.id()));
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
    let health_id = next_id("health");
    let health = json!({
        "protocol": FERRY_IPC_PROTOCOL,
        "id": health_id,
        "method": "health",
        "params": {},
    });
    let started = std::time::Instant::now();
    let response = client
        .transport
        .request(&health_id, &health.to_string(), STARTUP_HEALTH_TIMEOUT)
        .map_err(|error| {
            host_log(
                "runtime",
                &format!(
                    "Runtime health 请求失败 耗时={:.1}s: {error}",
                    started.elapsed().as_secs_f64()
                ),
            );
            error.to_string()
        })?;
    let value: Value = serde_json::from_str(&response).map_err(|error| error.to_string())?;
    verify_handshake(&value, "ferry-runtime", FERRY_CONTRACT_HASH).map_err(|reason| {
        // 展示文案保持稳定,真实原因(含对端返回的 hash)只进日志。
        host_log(
            "runtime",
            &format!("Runtime 握手失败: {reason}; 响应={:.500}", response),
        );
        "Ferry Runtime 协议握手失败".to_owned()
    })?;
    host_log(
        "runtime",
        &format!(
            "Runtime 握手成功 耗时={:.1}s",
            started.elapsed().as_secs_f64()
        ),
    );
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
            if matches!(event_type, "run.completed" | "run.failed" | "run.cancelled") {
                if let Some(session_id) = value.get("session_id").and_then(Value::as_str) {
                    forget_auto_policy(session_id);
                    let run_id = value.get("run_id").and_then(Value::as_str).unwrap_or("");
                    choice::finish_run(session_id, run_id);
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
    // 进程没了就不会再有 run 终态事件,挂起的选择要在这里了结。
    choice::cancel_all();
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
) -> Result<String, RuntimeError> {
    let id = serde_json::from_str::<Value>(request)
        .ok()
        .and_then(|value| value.get("id").and_then(Value::as_str).map(str::to_owned))
        .ok_or("Runtime 命令缺少 id")?;
    let client = ensure_runtime(app, resource_dir).map_err(|error| {
        host_log("runtime", &format!("Runtime 启动失败: {error}"));
        error
    })?;
    let result = client.transport.request(&id, request, COMMAND_TIMEOUT);
    if let Err(error) = &result {
        host_log("runtime", &format!("Runtime 命令失败 id={id}: {error}"));
    }
    if result
        .as_ref()
        .is_err_and(ProcessError::invalidates_process)
    {
        invalidate_runtime(client.generation);
    }
    result.map_err(|error| RuntimeError::Message(error.to_string()))
}

fn ensure_runtime(
    app: &tauri::AppHandle,
    resource_dir: &Path,
) -> Result<RuntimeClient, RuntimeError> {
    ensure_runtime_with(|| spawn_runtime(app, resource_dir))
}

/// 门在进程管理器之前:开关关着时连 supervisor 都不进,更不会 spawn。
fn ensure_runtime_with(
    spawn: impl FnOnce() -> Result<RuntimeProcess, String>,
) -> Result<RuntimeClient, RuntimeError> {
    runtime_gate()?;
    Ok(RUNTIME_PROCESS
        .get_or_init(|| ProcessSupervisor::new("Runtime"))
        .ensure(spawn)?)
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
) -> Result<String, RuntimeError> {
    // 先过门:开关关着时连请求形状都不必看,更不会记下自动批准策略。
    runtime_gate()?;
    validate_public_command(&request)?;
    remember_auto_policy(&request);
    let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || request_runtime(&app, &resource_dir, &request))
        .await
        .map_err(|e| e.to_string())?
}

pub(crate) fn warm_up(app: tauri::AppHandle, resource_dir: PathBuf) {
    // 预热的全部意义就是提前把 sidecar 拉起来;开关关着时连线程都不必起。
    if runtime_gate().is_err() {
        return;
    }
    std::thread::spawn(move || {
        let request = json!({"protocol": FERRY_IPC_PROTOCOL, "id": next_id("warmup"),
                             "method": "health", "params": {}});
        let _ = request_runtime(&app, &resource_dir, &request.to_string());
    });
}

fn runtime_binary_command(resource_dir: &Path) -> Result<Command, String> {
    let (command, candidates) = bundled_sidecar_command(resource_dir, "ferry-runtime");
    if let Some(command) = command {
        return Ok(command);
    }

    #[cfg(debug_assertions)]
    {
        let _ = candidates;
        let root = crate::process::command::repository_root();
        let mut command = Command::new("node");
        command.arg(root.join("ferry-runtime/dist/server/server.js"));
        command.current_dir(root);
        Ok(command)
    }

    #[cfg(not(debug_assertions))]
    Err(crate::process::command::missing_sidecar_message(
        "Ferry Runtime",
        &candidates,
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard, PoisonError};

    /// 开关是进程级的覆盖位,翻动它的用例必须串行。
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn with_switch(enabled: bool) -> MutexGuard<'static, ()> {
        let guard = TEST_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        AGENT_ENABLED.store(enabled, Ordering::Relaxed);
        guard
    }

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

    #[test]
    fn a_closed_switch_refuses_before_anything_is_spawned() {
        let _serial = with_switch(false);
        let spawned = std::sync::atomic::AtomicBool::new(false);
        let Err(error) = ensure_runtime_with(|| {
            spawned.store(true, Ordering::Relaxed);
            Err("不该走到 spawn".to_owned())
        }) else {
            panic!("开关关着时必须被拒");
        };
        assert!(!spawned.load(Ordering::Relaxed), "拒绝发生在 spawn 之前");
        let value = serde_json::to_value(&error).expect("可序列化");
        assert_eq!(value["code"], "feature.disabled");
        assert_eq!(value["feature"], "builtin-agent");
    }

    #[test]
    fn an_open_switch_lets_the_spawn_through() {
        let _serial = with_switch(true);
        let spawned = std::sync::atomic::AtomicBool::new(false);
        let Err(error) = ensure_runtime_with(|| {
            spawned.store(true, Ordering::Relaxed);
            Err("这次由 spawn 自己失败".to_owned())
        }) else {
            panic!("spawn 失败照旧上报");
        };
        assert!(spawned.load(Ordering::Relaxed));
        // 门以外的失败仍是纯文本,前端已有的兜底展示不变。
        assert_eq!(
            serde_json::to_value(&error).expect("可序列化"),
            Value::String("这次由 spawn 自己失败".to_owned())
        );
    }

    /// 审批与选择卡是 runtime 会话的一部分,不能因为「不直接发帧」就漏在门外。
    #[test]
    fn the_gate_covers_the_approval_channels_too() {
        let _serial = with_switch(false);
        let bash = tauri::async_runtime::block_on(bash::bash_apply("shl_anything".to_owned()))
            .expect_err("bash 审批要过门");
        let choice = choice::choice_respond(
            "session".to_owned(),
            "request".to_owned(),
            json!({"answered": false, "selected": []}),
        )
        .expect_err("选择卡应答要过门");
        for error in [bash, choice] {
            let value = serde_json::to_value(&error).expect("可序列化");
            assert_eq!(value["code"], "feature.disabled");
            assert_eq!(value["feature"], "builtin-agent");
        }
    }
}
