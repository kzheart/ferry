//! 引擎 socket 在非 unix 平台上的 fail-closed 边界。
//!
//! 引擎侧的 Windows 命名管道(`\\.\pipe\ferry-engine-<user>`)本身还没实现,所以这里
//! 不是「宿主缺一块」而是「这个平台上根本没有共享引擎这回事」:调用方据此跳过赶
//! daemon、也不给 sidecar 传 `--socket`。

use std::path::{Path, PathBuf};
use std::time::Duration;

const UNSUPPORTED: &str = "当前平台尚未实现引擎 socket(Windows 命名管道待实现)";

pub(super) fn engine_socket_path() -> Result<PathBuf, String> {
    Err(UNSUPPORTED.to_owned())
}

pub(super) fn engine_socket_call(
    _socket: &Path,
    _request: &str,
    _timeout: Duration,
) -> Result<String, String> {
    Err(UNSUPPORTED.to_owned())
}
