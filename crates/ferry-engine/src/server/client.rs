//! `ferry` 薄客户端的传输层：连 socket、必要时自拉起 daemon、发一条 JSONL 请求。
//!
//! 三条纪律：
//!
//! 1. **不加工**。请求参数怎么来的在 [`super::commands`] 决定，这里只负责把
//!    信封送出去、把应答原样带回来；成功与失败都不改词表。
//! 2. **自拉起是幂等的**。连不上先看锁：持有者活着就等它把 socket 挂出来，
//!    死了才清残留并 spawn。两个 CLI 同时冷启动最多有一个绑定成功，另一个
//!    会在重试窗口里连上它。
//! 3. **版本一致性在连接时就查**。契约哈希不一致的 daemon 直接换掉；App 的
//!    引擎不能杀，只能报错让用户升级 App。

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Map, Value};

use crate::contracts::ipc::FERRY_CONTRACT_HASH;
use crate::errors::DomainError;
use crate::server::rpc::PROTOCOL;
use crate::server::socket::platform::{self, SocketStream};
use crate::server::socket::{lock, EngineMode};

/// 自拉起后等 socket 出现的总预算。冷启动要建注册表、开库，1s 打不住。
const CONNECT_BUDGET: Duration = Duration::from_secs(15);

/// 单次请求的读超时。内容检索在索引未就绪时可能扫全库，留足余量。
const READ_TIMEOUT: Duration = Duration::from_secs(300);

/// 自拉起 daemon 的空闲退出时长（秒）。
const DAEMON_IDLE_EXIT_SEC: u64 = 600;

/// 一次调用的失败形态。退出码映射靠它区分。
pub enum Failure {
    /// 引擎回了错误信封：`{code, category, retryable, params}` 原样带回。
    Engine(Value),
    /// 连不上、拉不起、协议错乱——CLI 自己造的结构化错误。
    Transport(DomainError),
    /// 等待类命令超时：带的是最后一次拿到的状态，不是错误信封。
    Timeout(Value),
}

impl Failure {
    /// 打印的 JSON：三种结局同一形状（超时带的是状态本身）。
    pub fn payload(&self) -> Value {
        match self {
            Self::Engine(payload) => payload.clone(),
            Self::Timeout(status) => status.clone(),
            Self::Transport(error) => {
                let mut params = error.params().clone();
                params
                    .entry("message".to_string())
                    .or_insert_with(|| Value::from(error.message()));
                let mut payload = Map::new();
                payload.insert("code".into(), Value::from(error.code));
                payload.insert("params".into(), Value::Object(params));
                payload.insert("category".into(), Value::from(error.category));
                payload.insert("retryable".into(), Value::Bool(error.retryable));
                Value::Object(payload)
            }
        }
    }

    /// 退出码：引擎业务错误 1，连接/传输失败 2，等待超时 3。
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Engine(_) => 1,
            Self::Transport(_) => 2,
            Self::Timeout(_) => 3,
        }
    }
}

fn transport(reason: &str, message: impl Into<String>, recovery: &str) -> Failure {
    Failure::Transport(DomainError::engine_unavailable(reason, message, recovery))
}

/// 一条已连上的 socket 会话。
pub struct Client {
    stream: SocketStream,
    reader: BufReader<SocketStream>,
    sequence: u64,
    status: Value,
}

impl Client {
    fn open(path: &Path) -> Result<Self, Failure> {
        let stream = platform::connect(path).map_err(|error| {
            transport(
                "connect_failed",
                error,
                "确认 Ferry App 正在运行，或让 CLI 自行拉起 daemon",
            )
        })?;
        stream
            .set_read_timeout(Some(READ_TIMEOUT))
            .map_err(|error| transport("socket_setup_failed", error.to_string(), "重试一次"))?;
        let reader = stream
            .try_clone()
            .map(BufReader::new)
            .map_err(|error| transport("socket_setup_failed", error.to_string(), "重试一次"))?;
        Ok(Self {
            stream,
            reader,
            sequence: 0,
            status: Value::Null,
        })
    }

    /// 引擎自报的实例状态（`daemon.status` 的结果）。
    pub fn status(&self) -> &Value {
        &self.status
    }

    /// 发一条请求，拿回 `result`。
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, Failure> {
        self.sequence += 1;
        let id = format!("cli-{}-{}", std::process::id(), self.sequence);
        let mut envelope = Map::new();
        envelope.insert("protocol".into(), Value::from(PROTOCOL));
        envelope.insert("id".into(), Value::from(id.as_str()));
        envelope.insert("method".into(), Value::from(method));
        envelope.insert("params".into(), params);
        let line = Value::Object(envelope).to_string();
        self.stream
            .write_all(line.as_bytes())
            .and_then(|()| self.stream.write_all(b"\n"))
            .and_then(|()| self.stream.flush())
            .map_err(|error| {
                transport(
                    "write_failed",
                    format!("请求写入失败: {error}"),
                    "引擎可能已退出，重试一次",
                )
            })?;
        // 一条连接同一时刻只有一条在途请求，按 id 校验就够。
        loop {
            let mut raw = String::new();
            let read = self.reader.read_line(&mut raw).map_err(|error| {
                transport(
                    "read_failed",
                    format!("应答读取失败: {error}"),
                    "引擎可能已退出，重试一次",
                )
            })?;
            if read == 0 {
                return Err(transport(
                    "connection_closed",
                    "引擎在应答前关闭了连接",
                    "重试一次；持续失败见 ~/.ferry/daemon.log",
                ));
            }
            if raw.trim().is_empty() {
                continue;
            }
            let response: Value = serde_json::from_str(raw.trim()).map_err(|error| {
                transport(
                    "invalid_response",
                    format!("应答不是合法 JSON: {error}"),
                    "引擎版本可能不匹配",
                )
            })?;
            if response.get("id").and_then(Value::as_str) != Some(id.as_str()) {
                continue;
            }
            if response.get("ok") == Some(&Value::Bool(true)) {
                return Ok(response.get("result").cloned().unwrap_or(Value::Null));
            }
            return Err(Failure::Engine(
                response.get("error").cloned().unwrap_or(Value::Null),
            ));
        }
    }
}

/// 连接（必要时自拉起），并完成契约哈希握手。
pub fn connect(socket: &Path) -> Result<Client, Failure> {
    if !platform::supported() {
        return Err(transport(
            "transport_unsupported",
            "当前平台尚未实现本地 socket 传输",
            "Windows 命名管道待实现；请在 App 内使用 Ferry",
        ));
    }
    let client = connect_or_spawn(socket)?;
    handshake(socket, client)
}

/// 只连已有实例，不自拉起，也不做契约握手。
///
/// `daemon status` / `daemon stop` 用它：为了「报告/停止现状」而先拉起一个
/// daemon 是荒谬的；契约不一致的旧实例也应当能被查看和停掉。
pub fn attach(socket: &Path) -> Result<Client, Failure> {
    if !platform::supported() {
        return Err(transport(
            "transport_unsupported",
            "当前平台尚未实现本地 socket 传输",
            "Windows 命名管道待实现；请在 App 内使用 Ferry",
        ));
    }
    Client::open(socket).map_err(|_| {
        transport(
            "not_running",
            format!("没有引擎在 {} 上监听", socket.display()),
            "任意一条 ferry 命令都会按需拉起 daemon",
        )
    })
}

fn connect_or_spawn(socket: &Path) -> Result<Client, Failure> {
    if let Ok(client) = Client::open(socket) {
        return Ok(client);
    }
    let lock_path = lock::lock_path();
    // 持有者还活着（正在启动）就别再拉一个，等它把 socket 挂出来。
    let holder_alive = lock::read(&lock_path).is_some_and(|record| {
        record.socket == socket.display().to_string() && platform::process_alive(record.pid)
    });
    if !holder_alive {
        clear_stale(socket, &lock_path);
        spawn_daemon(socket)?;
    }
    wait_for_socket(socket)
}

fn clear_stale(socket: &Path, lock_path: &Path) {
    if socket.exists() && platform::connect(socket).is_err() {
        let _ = std::fs::remove_file(socket);
    }
    if lock::read(lock_path).is_some_and(|record| !platform::process_alive(record.pid)) {
        let _ = std::fs::remove_file(lock_path);
    }
}

fn wait_for_socket(socket: &Path) -> Result<Client, Failure> {
    let deadline = Instant::now() + CONNECT_BUDGET;
    let mut wait = Duration::from_millis(50);
    loop {
        match Client::open(socket) {
            Ok(client) => return Ok(client),
            Err(failure) if Instant::now() >= deadline => {
                // 预算耗尽才把最后一次失败抛出去，并指向 daemon 日志。
                if let Failure::Transport(error) = &failure {
                    return Err(transport(
                        "daemon_unreachable",
                        format!(
                            "{}（{}s 内未能连上引擎）",
                            error.message(),
                            CONNECT_BUDGET.as_secs()
                        ),
                        &format!("查看 {} 里的启动日志", lock::daemon_log_path().display()),
                    ));
                }
                return Err(failure);
            }
            Err(_) => {
                std::thread::sleep(wait);
                wait = (wait * 2).min(Duration::from_millis(400));
            }
        }
    }
}

/// 用当前二进制拉起一个脱离终端的 daemon。
fn spawn_daemon(socket: &Path) -> Result<(), Failure> {
    let executable = std::env::current_exe().map_err(|error| {
        transport(
            "spawn_failed",
            format!("无法定位自身二进制: {error}"),
            "重新安装 ferry",
        )
    })?;
    let log_path = lock::daemon_log_path();
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|error| {
            transport(
                "spawn_failed",
                format!("无法打开 {}: {error}", log_path.display()),
                "检查 ~/.ferry 目录权限",
            )
        })?;
    let errors = log
        .try_clone()
        .map_err(|error| transport("spawn_failed", error.to_string(), "检查 ~/.ferry 目录权限"))?;
    let mut command = std::process::Command::new(executable);
    command
        .arg("serve")
        .arg("--socket")
        .arg(socket)
        .arg("--mode")
        .arg(EngineMode::Daemon.as_str())
        .arg("--idle-exit")
        .arg(DAEMON_IDLE_EXIT_SEC.to_string())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(log))
        .stderr(std::process::Stdio::from(errors));
    detach(&mut command);
    command.spawn().map(|_child| ()).map_err(|error| {
        transport(
            "spawn_failed",
            format!("无法拉起引擎 daemon: {error}"),
            &format!(
                "手动运行 `ferry-engine serve --socket {}`",
                socket.display()
            ),
        )
    })
}

#[cfg(unix)]
fn detach(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    // pre_exec 在 fork 之后、exec 之前跑，只调 setsid（async-signal-safe）。
    unsafe {
        command.pre_exec(platform::detach_session);
    }
}

#[cfg(not(unix))]
fn detach(_command: &mut std::process::Command) {}

/// 契约哈希握手：不一致的 daemon 换掉，不一致的 App 报错。
fn handshake(socket: &Path, mut client: Client) -> Result<Client, Failure> {
    let status = client.call("daemon.status", Value::Object(Map::new()))?;
    let matches = status.get("contract_hash").and_then(Value::as_str) == Some(FERRY_CONTRACT_HASH);
    if matches {
        client.status = status;
        return Ok(client);
    }
    let mode = status.get("mode").and_then(Value::as_str).unwrap_or("app");
    if mode != EngineMode::Daemon.as_str() {
        return Err(transport(
            "contract_mismatch",
            "共享中的 App 引擎与本 CLI 契约版本不一致",
            "升级 Ferry App，或退出 App 让 CLI 使用自己的 daemon",
        ));
    }
    // 旧 daemon：关掉它，用自己的二进制重来一次。
    let _ = client.call("daemon.shutdown", Value::Object(Map::new()));
    drop(client);
    let deadline = Instant::now() + CONNECT_BUDGET;
    while socket.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let mut client = connect_or_spawn(socket)?;
    let status = client.call("daemon.status", Value::Object(Map::new()))?;
    if status.get("contract_hash").and_then(Value::as_str) != Some(FERRY_CONTRACT_HASH) {
        return Err(transport(
            "contract_mismatch",
            "重启后的引擎契约版本仍不一致",
            "确认 PATH 里的 ferry 与正在运行的引擎来自同一次安装",
        ));
    }
    client.status = status;
    Ok(client)
}

/// 默认 socket 路径（`FERRY_ENGINE_SOCKET` > `~/.ferry/engine.sock`）。
pub fn default_socket() -> PathBuf {
    lock::default_socket_path()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_failures_render_the_engine_error_shape() {
        let failure = transport("connect_failed", "连不上", "先启动 App");
        let payload = failure.payload();
        assert_eq!(payload["code"], Value::from("engine.unavailable"));
        assert_eq!(payload["category"], Value::from("unavailable"));
        assert_eq!(payload["params"]["reason"], Value::from("connect_failed"));
        assert_eq!(payload["params"]["message"], Value::from("连不上"));
        assert!(payload["params"]["recovery"].is_string());
        assert_eq!(failure.exit_code(), 2);
    }

    #[test]
    fn engine_failures_are_passed_through_verbatim() {
        let payload = serde_json::json!({
            "code": "agent.reference_invalid",
            "category": "validation",
            "retryable": false,
            "params": {"reason": "unknown_ref"},
        });
        let failure = Failure::Engine(payload.clone());
        assert_eq!(failure.payload(), payload);
        assert_eq!(failure.exit_code(), 1);
    }
}
