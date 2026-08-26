//! 既非 unix 也非 windows 的平台：与 [`super::windows`] 同样 fail-closed。

use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

pub(super) const SUPPORTED: bool = false;

pub(super) enum Listener {}
pub(super) enum Stream {}

pub(super) fn bind(_path: &Path) -> Result<Listener, String> {
    Err("当前平台尚未实现本地 socket 传输".to_string())
}

pub(super) fn connect(_path: &Path) -> Result<Stream, String> {
    Err("当前平台尚未实现本地 socket 传输".to_string())
}

pub(super) fn listener_available(_path: &Path) -> bool {
    false
}

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
