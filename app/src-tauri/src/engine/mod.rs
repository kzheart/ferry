pub(crate) mod daemon;
mod policy;

use self::policy::{request_attempts, request_timeout};
use crate::contracts::engine_methods::is_ui_engine_method;
use crate::contracts::events::{event_policy, EventSource};
use crate::contracts::ipc::{FERRY_CONTRACT_HASH, FERRY_IPC_PROTOCOL};
use crate::desktop::{host_settings, platform};
use crate::process::client::{JsonlProcessClient, PendingResponses};
use crate::process::command::{bundled_sidecar_command, configure_background};
use crate::process::error::ProcessError;
use crate::process::handshake::verify_handshake;
use crate::process::logging::{host_log, sidecar_stderr};
use crate::process::supervisor::{ManagedProcess, ProcessSupervisor};
use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    OnceLock,
};
use std::time::{Duration, Instant};

#[derive(Clone)]
struct EngineClient {
    generation: u64,
    transport: JsonlProcessClient,
}

type EngineProcess = ManagedProcess<EngineClient>;

static ENGINE: OnceLock<ProcessSupervisor<EngineClient>> = OnceLock::new();
static ENGINE_REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static ENGINE_GENERATION_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static ENGINE_APP: OnceLock<tauri::AppHandle> = OnceLock::new();

/// 引擎主动通知(无 id 的事件帧):按契约事件策略转发给 webview。
fn forward_engine_event(value: &Value) {
    let Some(event_type) = value.get("type").and_then(Value::as_str) else {
        return;
    };
    if value.get("protocol").and_then(Value::as_str) != Some(FERRY_IPC_PROTOCOL) {
        return;
    }
    let Some(policy) = event_policy(event_type) else {
        host_log("engine", &format!("忽略未注册的引擎事件: {event_type}"));
        return;
    };
    if policy.source != EventSource::Engine || !policy.forward_to_ui {
        return;
    }
    if let Some(app) = ENGINE_APP.get() {
        use tauri::Emitter;
        let _ = app.emit("ferry-engine-event", value.clone());
    }
}

fn stamp_engine_request(request: &str) -> Result<(String, String), String> {
    let value: Value = serde_json::from_str(request)
        .map_err(|error| format!("Engine 请求不是有效 JSON: {error}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "Engine 请求必须是 JSON object".to_owned())?;
    let method = object
        .get("method")
        .and_then(Value::as_str)
        .ok_or_else(|| "Engine 请求缺少 method".to_owned())?;
    let params = object
        .get("params")
        .cloned()
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
    if !params.is_object() {
        return Err("Engine 请求 params 必须是 JSON object".to_owned());
    }
    let request_id = format!(
        "engine_{:x}",
        ENGINE_REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed),
    );
    let envelope = serde_json::json!({
        "protocol": FERRY_IPC_PROTOCOL,
        "id": request_id,
        "method": method,
        "params": params,
    });
    Ok((envelope.to_string(), request_id))
}

fn validate_engine_response_id(response: &str, request_id: &str) -> Result<(), String> {
    let value: Value = serde_json::from_str(response)
        .map_err(|error| format!("Engine 响应不是有效 JSON: {error}"))?;
    if value.get("protocol").and_then(Value::as_str) != Some(FERRY_IPC_PROTOCOL) {
        return Err("Engine 响应 protocol 不匹配".to_owned());
    }
    if value.get("id").and_then(Value::as_str) != Some(request_id) {
        return Err("Engine 响应 id 不匹配".to_owned());
    }
    Ok(())
}

/// 带 socket 起不来时的重试间隔:绑定冲突多半是上一个实例正在收尾。
const SOCKET_RETRY_DELAY: Duration = Duration::from_millis(500);

/// 本次启动要不要共享引擎。开关的事实源是宿主自己的配置文件(WebView 此刻还没起来),
/// 平台不支持 socket 时无论开关如何都不共享。
fn socket_argument(share: bool, socket: Result<PathBuf, String>) -> Option<PathBuf> {
    if !share {
        return None;
    }
    match socket {
        Ok(path) => Some(path),
        Err(reason) => {
            host_log("engine", &format!("本平台没有引擎 socket,不共享: {reason}"));
            None
        }
    }
}

fn spawn_engine(resource_dir: &Path) -> Result<EngineProcess, String> {
    match socket_argument(
        host_settings::engine_share(),
        platform::engine_socket_path(),
    ) {
        Some(socket) => spawn_shared_engine(resource_dir, &socket),
        None => spawn_engine_process(resource_dir, None),
    }
}

/// 共享形态的启动序列:先把在跑的 CLI daemon 请下去(App 优先级恒高于 daemon),
/// 再带 `--socket` 起自己;绑定竞态给一次重试,仍失败就**降级为不共享**——
/// 共享是增值能力,App 必须能起来(.docs/cli-skill-design.md §4.2)。
fn spawn_shared_engine(resource_dir: &Path, socket: &Path) -> Result<EngineProcess, String> {
    daemon::evict(socket);
    match spawn_engine_process(resource_dir, Some(socket)) {
        Ok(process) => return Ok(process),
        Err(error) => host_log(
            "engine",
            &format!(
                "带 socket 启动失败,{}ms 后重试一次: {error}",
                SOCKET_RETRY_DELAY.as_millis()
            ),
        ),
    }
    std::thread::sleep(SOCKET_RETRY_DELAY);
    match spawn_engine_process(resource_dir, Some(socket)) {
        Ok(process) => Ok(process),
        Err(error) => {
            host_log(
                "engine",
                &format!("socket 仍不可用,降级为不共享引擎启动: {error}"),
            );
            spawn_engine_process(resource_dir, None)
        }
    }
}

fn spawn_engine_process(
    resource_dir: &Path,
    socket: Option<&Path>,
) -> Result<EngineProcess, String> {
    let mut command = engine_command(resource_dir)?;
    command.arg("serve");
    // 不传 --mode:默认就是 app 模式——不 idle-exit,也拒绝 CLI 的 daemon.shutdown。
    if let Some(socket) = socket {
        command.arg("--socket").arg(socket);
    }
    configure_background(&mut command);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(sidecar_stderr("engine.log"));
    let mut child = command
        .spawn()
        .map_err(|error| format!("启动引擎失败: {error}"))?;
    host_log(
        "engine",
        &format!(
            "引擎进程已启动 pid={} socket={}",
            child.id(),
            socket
                .map(Path::display)
                .map_or("无".to_owned(), |path| path.to_string()),
        ),
    );
    let stdin = child.stdin.take().ok_or("引擎 stdin 不可用")?;
    let stdout = child.stdout.take().ok_or("引擎 stdout 不可用")?;
    let transport = JsonlProcessClient::new("Engine", stdin);
    let reader_pending = transport.pending();
    std::thread::spawn(move || {
        read_engine_output(BufReader::new(stdout), reader_pending);
    });
    let generation = ENGINE_GENERATION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let client = EngineClient {
        generation,
        transport,
    };
    let process = ManagedProcess::new(generation, child, client.clone());
    handshake(&client)?;
    Ok(process)
}

/// 协议握手作为常驻进程的首条请求完成:独立的一次性 health 子进程
/// 会让打包 sidecar 多付一整次冷启动成本。
fn handshake(engine: &EngineClient) -> Result<(), String> {
    let (request, request_id) = stamp_engine_request(r#"{"method":"health"}"#)?;
    let line = engine
        .transport
        .request(&request_id, &request, Duration::from_secs(15))
        .map_err(|error| error.to_string())
        .map_err(|error| format!("引擎健康检查失败: {error}"))?;
    validate_engine_response_id(&line, &request_id)
        .map_err(|error| format!("引擎健康检查失败: {error}"))?;
    let health: Value = serde_json::from_str(&line)
        .map_err(|error| format!("引擎健康检查返回无效 JSON: {error}"))?;
    verify_handshake(&health, "engine", FERRY_CONTRACT_HASH).map_err(|reason| {
        // 展示文案保持稳定,真实原因只进日志。
        host_log("engine", &format!("引擎握手失败: {reason}"));
        "引擎协议或契约握手失败".to_owned()
    })?;
    host_log("engine", "引擎握手成功");
    Ok(())
}

fn read_engine_output(mut stdout: impl BufRead, pending: PendingResponses) {
    let mut line = String::new();
    loop {
        line.clear();
        match stdout.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(error) => {
                pending.fail_all(ProcessError::Exited(format!("读取引擎失败: {error}")));
                return;
            }
        }
        let response = line.trim_end();
        let parsed = serde_json::from_str::<Value>(response).ok();
        if let Some(value) = parsed.as_ref() {
            if value.get("id").is_none() && value.get("type").is_some() {
                forward_engine_event(value);
                continue;
            }
        }
        let request_id =
            parsed.and_then(|value| value.get("id").and_then(Value::as_str).map(str::to_owned));
        let Some(request_id) = request_id else {
            pending.fail_all(ProcessError::Exited("Engine 响应缺少 id".to_owned()));
            return;
        };
        pending.complete(&request_id, response.to_owned());
    }
    pending.fail_all(ProcessError::Exited("引擎进程已退出".to_owned()));
}

fn engine_client(resource_dir: &Path) -> Result<EngineClient, String> {
    ENGINE
        .get_or_init(|| ProcessSupervisor::new("引擎"))
        .ensure(|| spawn_engine(resource_dir))
}

fn invalidate_engine(generation: u64) {
    ENGINE
        .get_or_init(|| ProcessSupervisor::new("引擎"))
        .invalidate(generation);
}

pub(crate) fn engine_request_blocking(
    resource_dir: &Path,
    request: &str,
) -> Result<String, String> {
    let (request, request_id) = stamp_engine_request(request)?;
    let method = serde_json::from_str::<Value>(&request)
        .ok()
        .and_then(|value| {
            value
                .get("method")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_default();
    let timeout = request_timeout(&request);
    let mut last_error = String::new();
    for attempt in 0..request_attempts(&request) {
        let client = engine_client(resource_dir)?;
        let started = Instant::now();
        match client.transport.request(&request_id, &request, timeout) {
            Ok(line) => match validate_engine_response_id(&line, &request_id) {
                Ok(()) => {
                    // scan_progress 这类高频轮询不刷屏,只记慢请求。
                    if started.elapsed() >= Duration::from_secs(1) {
                        host_log(
                            "engine",
                            &format!(
                                "{method} 完成 id={request_id} 耗时={:.1}s",
                                started.elapsed().as_secs_f64()
                            ),
                        );
                    }
                    return Ok(line);
                }
                Err(error) => {
                    host_log(
                        "engine",
                        &format!("{method} 响应校验失败 id={request_id}: {error}"),
                    );
                    last_error = error;
                    invalidate_engine(client.generation);
                }
            },
            Err(error) => {
                host_log(
                    "engine",
                    &format!(
                        "{method} 失败 id={request_id} attempt={attempt} 耗时={:.1}s: {error}",
                        started.elapsed().as_secs_f64()
                    ),
                );
                last_error = error.to_string();
                if error.invalidates_process() {
                    invalidate_engine(client.generation);
                }
            }
        }
    }
    Err(format!("引擎通信失败: {last_error}"))
}

fn engine_command(resource_dir: &Path) -> Result<Command, String> {
    let (command, candidates) = bundled_sidecar_command(resource_dir, "ferry-engine");
    if let Some(command) = command {
        return Ok(command);
    }

    #[cfg(debug_assertions)]
    {
        let _ = candidates;
        // 开发模式跑仓库内的引擎产物；没有产物就是没有引擎,不存在回退路径。
        crate::process::command::local_engine_command()
            .ok_or_else(crate::process::command::missing_local_engine_message)
    }

    #[cfg(not(debug_assertions))]
    Err(crate::process::command::missing_sidecar_message(
        "引擎",
        &candidates,
    ))
}

/// 应用启动即预热常驻引擎:引擎冷启动与 webview 启动并行,
/// 首个前端 RPC 到达时引擎大概率已就绪。失败静默,错误会在首个真实 RPC 上重现。
pub(crate) fn warm_up(app: tauri::AppHandle, resource_dir: PathBuf) {
    let _ = ENGINE_APP.set(app);
    std::thread::spawn(move || {
        let _ = engine_request_blocking(&resource_dir, r#"{"method":"health"}"#);
    });
}

#[tauri::command]
pub(crate) async fn engine_rpc(app: tauri::AppHandle, request: String) -> Result<String, String> {
    use tauri::Manager;
    validate_engine_request_caller(&request)?;
    let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || engine_request_blocking(&resource_dir, &request))
        .await
        .map_err(|e| e.to_string())?
}

/// 通用通道只认 callers 含 ui 的方法;operation.* 这类走各自的专用命令,
/// 参数形状由宿主固定,不能从这里按方法名直通。
fn validate_engine_request_caller(request: &str) -> Result<(), String> {
    let value: Value = serde_json::from_str(request)
        .map_err(|error| format!("Engine 请求不是有效 JSON: {error}"))?;
    let method = value.get("method").and_then(Value::as_str).unwrap_or("");
    if !is_ui_engine_method(method) {
        return Err("该 Engine 方法不允许从当前前端通道调用".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
