//! Windows 占位实现：命名管道（`\\.\pipe\ferry-engine-<user>`）是 P2 的事。
//!
//! 现在交付的是「显式、可编译、fail-closed」的边界：`serve --socket` 会带着
//! 清晰文案退出，CLI 客户端把它翻成结构化的 unavailable 错误。整个 crate 在
//! windows target 下必须能编译，这个文件就是保证。

use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

pub(super) const SUPPORTED: bool = false;

/// 不可构造：这个平台上不存在监听器实例。
pub(super) enum Listener {}
/// 不可构造：这个平台上不存在连接实例。
pub(super) enum Stream {}

pub(super) fn bind(_path: &Path) -> Result<Listener, String> {
    Err("当前平台尚未实现本地 socket 传输（Windows 命名管道待实现）".to_string())
}

pub(super) fn connect(_path: &Path) -> Result<Stream, String> {
    Err("当前平台尚未实现本地 socket 传输（Windows 命名管道待实现）".to_string())
}

/// 拿不准一律当活着：宁可拒绝抢占，也不能清掉活实例的锁。
pub(super) fn process_alive(_pid: u32) -> bool {
    true
}

pub(super) fn detach_session() -> std::io::Result<()> {
    Err(std::io::Error::other("当前平台尚未实现进程脱离终端"))
}

/// 没有 socket 也就没有需要优雅收尾的东西：保持信号的默认处置。
pub(super) fn install_termination_handler(_flag: &'static AtomicBool) -> bool {
    false
}

impl Listener {
    pub(super) fn accept(&self) -> std::io::Result<Stream> {
        match *self {}
    }
}

impl Stream {
    pub(super) fn try_clone(&self) -> std::io::Result<Self> {
        match *self {}
    }

    pub(super) fn set_read_timeout(&self, _timeout: Option<Duration>) -> std::io::Result<()> {
        match *self {}
    }
}

impl Read for Stream {
    fn read(&mut self, _buffer: &mut [u8]) -> std::io::Result<usize> {
        match *self {}
    }
}

impl Write for Stream {
    fn write(&mut self, _buffer: &[u8]) -> std::io::Result<usize> {
        match *self {}
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match *self {}
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    #[test]
    fn socket_transport_fails_closed() {
        assert!(!super::SUPPORTED);
        assert!(super::bind(Path::new("pipe")).is_err());
        assert!(super::connect(Path::new("pipe")).is_err());
        // 死活判定必须保守，否则会清掉活实例的锁。
        assert!(super::process_alive(1));
    }
}
