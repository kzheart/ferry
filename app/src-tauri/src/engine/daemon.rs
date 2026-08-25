//! 本地引擎 socket 上的管理方法客户端:`daemon.status` / `daemon.shutdown`。
//!
//! 这两条方法只存在于 socket 传输,由引擎的传输层在分发之前直接应答(不进方法表)。
//! 宿主用它们做两件事:
//!
//! 1. **启动接管**:App 起来时把 CLI 拉起的 daemon 请下去,自己接管 socket——
//!    App 优先级恒高于 daemon(.docs/cli-skill-design.md §4.2);
//! 2. **设置页停 daemon**:手动停掉一个独立 daemon;对面若是 App 自己的引擎,
//!    引擎会结构化拒绝,这里把它翻成 `app_mode` 交给前端出文案。
//!
//! 平台差异全在 [`crate::desktop::platform`](unix 真实现 / 其余 fail-closed),
//! 这里只管帧的形状与语义。

use serde::Serialize;
use serde_json::Value;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::contracts::ipc::FERRY_IPC_PROTOCOL;
use crate::desktop::platform;
use crate::process::logging::host_log;

/// 管理方法是一问一答的本地调用,慢过这个时间只可能是对面卡死。
const CALL_TIMEOUT: Duration = Duration::from_secs(3);

/// 等 daemon 真正退出(socket 文件消失)的上限。引擎收到 shutdown 后要先把应答
/// 写出去再拆 socket,所以「回了 ok」不等于「已经让位」。
pub(crate) const RELEASE_BUDGET: Duration = Duration::from_secs(5);

/// 等待期间的巡检间隔。
const POLL: Duration = Duration::from_millis(50);

/// 给前端的结构化失败:`code` 稳定可分支,`message` 只作兜底展示。
#[derive(Debug, Serialize)]
pub(crate) struct DaemonError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl DaemonError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// 一条管理方法请求帧。id 由宿主给定,与 stdio 通道同一套信封形状。
fn control_frame(method: &str) -> String {
    serde_json::json!({
        "protocol": FERRY_IPC_PROTOCOL,
        "id": control_id(method),
        "method": method,
        "params": {},
    })
    .to_string()
}

fn control_id(method: &str) -> String {
    format!("host_{}", method.replace('.', "_"))
}

/// 校验信封:协议与 id 都要对上,否则这行不是我们这条请求的应答。
fn parse_control_response(line: &str, method: &str) -> Result<Value, String> {
    let value: Value = serde_json::from_str(line)
        .map_err(|error| format!("引擎 socket 应答不是有效 JSON: {error}"))?;
    if value.get("protocol").and_then(Value::as_str) != Some(FERRY_IPC_PROTOCOL) {
        return Err("引擎 socket 应答 protocol 不匹配".to_owned());
    }
    if value.get("id").and_then(Value::as_str) != Some(control_id(method).as_str()) {
        return Err("引擎 socket 应答 id 不匹配".to_owned());
    }
    Ok(value)
}

/// 发一条管理方法。`Ok` 只代表通信成功,业务上的拒绝在信封的 `ok` 里。
fn call(socket: &Path, method: &str) -> Result<Value, String> {
    let line = platform::engine_socket_call(socket, &control_frame(method), CALL_TIMEOUT)?;
    parse_control_response(&line, method)
}

fn envelope_ok(response: &Value) -> bool {
    response.get("ok") == Some(&Value::Bool(true))
}

/// 拒绝信封里的解释。引擎把人话放在 `error.params.message`(见 rpc 的 error_envelope),
/// 拿不到就退回 code,再拿不到给一句固定文案。
fn envelope_message(response: &Value) -> String {
    response
        .pointer("/error/params/message")
        .or_else(|| response.pointer("/error/code"))
        .and_then(Value::as_str)
        .unwrap_or("引擎拒绝了这次请求")
        .to_owned()
}

/// 等 socket 文件消失。symlink 语义:socket 不是常规文件,断链场景下 lstat 才是真相。
pub(crate) fn wait_for_release(socket: &Path, budget: Duration) -> bool {
    let deadline = Instant::now() + budget;
    loop {
        if std::fs::symlink_metadata(socket).is_err() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(POLL);
    }
}

/// 启动前把可能在跑的 CLI daemon 请下去。
///
/// 全程尽力而为,任何失败都只落日志:连不上说明没人在听;被拒说明对面是另一个
/// App 的引擎,等它退出没有意义——后续绑定失败会走降级路径。
pub(crate) fn evict(socket: &Path) {
    let response = match call(socket, "daemon.shutdown") {
        Ok(response) => response,
        Err(error) => {
            host_log("engine", &format!("socket 上无人应答,直接接管: {error}"));
            return;
        }
    };
    if !envelope_ok(&response) {
        host_log(
            "engine",
            &format!("socket 占用者拒绝让位: {}", envelope_message(&response)),
        );
        return;
    }
    if wait_for_release(socket, RELEASE_BUDGET) {
        host_log("engine", "已请下在跑的 daemon,接管 socket");
    } else {
        host_log(
            "engine",
            &format!(
                "daemon 未在 {}s 内释放 socket,继续尝试绑定",
                RELEASE_BUDGET.as_secs()
            ),
        );
    }
}

/// 设置页的「停止 daemon」:先确认对面确实是 daemon,再发关停。
pub(crate) fn stop() -> Result<(), DaemonError> {
    let socket =
        platform::engine_socket_path().map_err(|reason| DaemonError::new("unsupported", reason))?;
    let status =
        call(&socket, "daemon.status").map_err(|reason| DaemonError::new("unavailable", reason))?;
    if !envelope_ok(&status) {
        return Err(DaemonError::new("unavailable", envelope_message(&status)));
    }
    // App 自己的引擎不从这里停:它跟着 App 的生命周期走,引擎也会拒绝。
    if status.pointer("/result/mode").and_then(Value::as_str) != Some("daemon") {
        return Err(DaemonError::new(
            "app_mode",
            "这个引擎是 App 自己的 sidecar,不能从这里停止",
        ));
    }
    let stopped = call(&socket, "daemon.shutdown")
        .map_err(|reason| DaemonError::new("unavailable", reason))?;
    if !envelope_ok(&stopped) {
        return Err(DaemonError::new("refused", envelope_message(&stopped)));
    }
    if !wait_for_release(&socket, RELEASE_BUDGET) {
        return Err(DaemonError::new(
            "timeout",
            format!("daemon 未在 {}s 内退出", RELEASE_BUDGET.as_secs()),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
