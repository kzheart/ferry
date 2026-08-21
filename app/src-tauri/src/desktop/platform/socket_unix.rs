//! 本地引擎 socket 的 unix 客户端。
//!
//! 宿主只用它发管理方法(`daemon.status` / `daemon.shutdown`):一次连接、一问一答,
//! 不复用连接、不订阅事件——那是引擎 stdio 通道的事。

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// 与引擎 `server::socket::lock::default_socket_path` 同一条规则:
/// `FERRY_ENGINE_SOCKET` 优先,否则 `~/.ferry/engine.sock`。两边必须一起改。
pub(super) fn engine_socket_path() -> Result<PathBuf, String> {
    match std::env::var_os("FERRY_ENGINE_SOCKET") {
        Some(value) if !value.is_empty() => Ok(PathBuf::from(value)),
        _ => Ok(super::home_dir()?.join(".ferry").join("engine.sock")),
    }
}

/// 发一行请求、读一行应答。连接失败即「没有引擎在这个路径上监听」。
pub(super) fn engine_socket_call(
    socket: &Path,
    request: &str,
    timeout: Duration,
) -> Result<String, String> {
    let mut stream = UnixStream::connect(socket)
        .map_err(|error| format!("无法连接 {}: {error}", socket.display()))?;
    let deadline = Some(timeout);
    stream
        .set_read_timeout(deadline)
        .and_then(|_| stream.set_write_timeout(deadline))
        .map_err(|error| format!("无法设置 socket 超时: {error}"))?;
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .map_err(|error| format!("socket 连接不可复制: {error}"))?,
    );
    writeln!(stream, "{request}")
        .and_then(|_| stream.flush())
        .map_err(|error| format!("写入引擎 socket 失败: {error}"))?;
    let mut line = String::new();
    let read = reader
        .read_line(&mut line)
        .map_err(|error| format!("读取引擎 socket 失败: {error}"))?;
    if read == 0 {
        return Err("引擎 socket 未应答就关闭了连接".to_owned());
    }
    Ok(line.trim_end().to_owned())
}
