mod policy;

use self::policy::{request_attempts, request_timeout};
use crate::contracts::engine_methods::{self, Exposure};
use crate::contracts::ipc::{FERRY_CONTRACT_HASH, FERRY_IPC_PROTOCOL};
use crate::process::client::{JsonlProcessClient, PendingResponses};
use crate::process::error::ProcessError;
use crate::process::supervisor::{ManagedProcess, ProcessSupervisor};
use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    OnceLock,
};
use std::time::Duration;

#[derive(Clone)]
struct EngineClient {
    generation: u64,
    transport: JsonlProcessClient,
}

type EngineProcess = ManagedProcess<EngineClient>;

static ENGINE: OnceLock<ProcessSupervisor<EngineClient>> = OnceLock::new();
static ENGINE_REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static ENGINE_GENERATION_SEQUENCE: AtomicU64 = AtomicU64::new(1);

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

fn spawn_engine(resource_dir: &Path) -> Result<EngineProcess, String> {
    let mut command = engine_command(resource_dir)?;
    command.arg("serve");
    crate::desktop::platform::configure_background_command(&mut command);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|error| format!("启动引擎失败: {error}"))?;
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
/// 在 release 下会让 PyInstaller onefile 多解压一整次,冷启动时间翻倍。
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
    if health.get("ok").and_then(Value::as_bool) != Some(true)
        || health.pointer("/result/service").and_then(Value::as_str) != Some("engine")
        || health
            .pointer("/result/contract_hash")
            .and_then(Value::as_str)
            != Some(FERRY_CONTRACT_HASH)
    {
        return Err("引擎协议或契约握手失败".to_owned());
    }
    Ok(())
}

impl EngineClient {
    fn request(&self, request: &str, timeout: Duration) -> Result<String, ProcessError> {
        let value: Value = serde_json::from_str(request).map_err(|error| {
            ProcessError::InvalidFrame(format!("Engine 请求不是有效 JSON: {error}"))
        })?;
        let request_id = value
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| ProcessError::InvalidFrame("Engine 请求缺少 id".to_owned()))?;
        self.transport.request(request_id, request, timeout)
    }
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
        let request_id = serde_json::from_str::<Value>(response)
            .ok()
            .and_then(|value| value.get("id").and_then(Value::as_str).map(str::to_owned));
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
    let timeout = request_timeout(&request);
    let mut last_error = String::new();
    for _attempt in 0..request_attempts(&request) {
        let client = engine_client(resource_dir)?;
        match client.request(&request, timeout) {
            Ok(line) => match validate_engine_response_id(&line, &request_id) {
                Ok(()) => return Ok(line),
                Err(error) => {
                    last_error = error;
                    invalidate_engine(client.generation);
                }
            },
            Err(error) => {
                last_error = error.to_string();
                if error.invalidates_process() {
                    invalidate_engine(client.generation);
                }
            }
        }
    }
    Err(format!("引擎通信失败: {last_error}"))
}

/// 引擎仓库根目录:优先 FERRY_REPO 环境变量,
/// 否则取本 crate 上两级(app/src-tauri → 仓库根,开发形态)。
#[cfg(debug_assertions)]
fn repo_root() -> PathBuf {
    if let Ok(p) = std::env::var("FERRY_REPO") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from("."))
}

/// externalBin 在 macOS 上被放进 Contents/MacOS(主程序旁),Windows 上在安装根目录;
/// 依次尝试可执行文件所在目录与 resource_dir,取第一个存在的。
fn bundled_engine_candidates(resource_dir: &Path) -> Vec<PathBuf> {
    let name = bundled_engine_name(cfg!(target_os = "windows"));
    let mut candidates = Vec::new();
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    {
        candidates.push(exe_dir.join(name));
    }
    candidates.push(resource_dir.join(name));
    candidates
}

fn bundled_engine_name(is_windows: bool) -> &'static str {
    if is_windows {
        "ferry-engine.exe"
    } else {
        "ferry-engine"
    }
}

fn engine_command(resource_dir: &Path) -> Result<Command, String> {
    let candidates = bundled_engine_candidates(resource_dir);
    if let Some(sidecar) = candidates.iter().find(|path| path.is_file()) {
        return Ok(Command::new(sidecar));
    }

    #[cfg(debug_assertions)]
    {
        let mut command = Command::new(if cfg!(target_os = "windows") {
            "python"
        } else {
            "python3"
        });
        command.args(["-m", "engine.server.cli"]);
        command.current_dir(repo_root());
        Ok(command)
    }

    #[cfg(not(debug_assertions))]
    Err(format!(
        "正式包缺少引擎 sidecar,已尝试: {}",
        candidates
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("; ")
    ))
}

/// 应用启动即预热常驻引擎:PyInstaller 解压与 webview 启动并行,
/// 首个前端 RPC 到达时引擎大概率已就绪。失败静默,错误会在首个真实 RPC 上重现。
pub(crate) fn warm_up(resource_dir: PathBuf) {
    std::thread::spawn(move || {
        let _ = engine_request_blocking(&resource_dir, r#"{"method":"health"}"#);
    });
}

#[tauri::command]
pub(crate) async fn engine_rpc(app: tauri::AppHandle, request: String) -> Result<String, String> {
    use tauri::Manager;
    validate_engine_request_exposure(&request, Exposure::Public)?;
    let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || engine_request_blocking(&resource_dir, &request))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(crate) async fn trusted_engine_rpc(
    app: tauri::AppHandle,
    request: String,
) -> Result<String, String> {
    use tauri::Manager;
    validate_engine_request_exposure(&request, Exposure::TrustedUi)?;
    let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || engine_request_blocking(&resource_dir, &request))
        .await
        .map_err(|e| e.to_string())?
}

fn validate_engine_request_exposure(request: &str, expected: Exposure) -> Result<(), String> {
    let value: Value = serde_json::from_str(request)
        .map_err(|error| format!("Engine 请求不是有效 JSON: {error}"))?;
    let method = value.get("method").and_then(Value::as_str).unwrap_or("");
    if !engine_methods::policy(method).is_some_and(|policy| policy.exposure == expected) {
        return Err("该 Engine 方法不允许从当前前端通道调用".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
