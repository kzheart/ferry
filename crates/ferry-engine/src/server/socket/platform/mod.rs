//! 本地 socket 的平台边界。
//!
//! 与 `desktop/platform` 同款规则：Unix 走 domain socket，Windows 走命名管道，
//! 其余平台是显式的、可编译的 unsupported 占位。业务代码只依赖本模块的统一接口，
//! 不感知传输差异。

use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

#[cfg(unix)]
mod unix;
#[cfg(not(any(unix, target_os = "windows")))]
mod unsupported;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(unix)]
use self::unix as imp;
#[cfg(not(any(unix, target_os = "windows")))]
use self::unsupported as imp;
#[cfg(target_os = "windows")]
use self::windows as imp;

/// 监听中的本地 socket。
pub struct SocketListener(imp::Listener);

/// 一条本地 socket 连接。
pub struct SocketStream(imp::Stream);

/// 绑定并监听；权限收紧到 0600 由平台实现负责。
///
/// 调用方必须先处理陈旧 socket（见 `socket::lock`），这里不做仲裁。
pub fn bind(path: &Path) -> Result<SocketListener, String> {
    imp::bind(path).map(SocketListener)
}

/// 连接已有 socket。失败即「没有引擎在这个路径上监听」。
pub fn connect(path: &Path) -> Result<SocketStream, String> {
    imp::connect(path).map(SocketStream)
}

/// Whether a listener owns this endpoint without opening a client connection.
/// Windows must not consume the only pending named-pipe instance during stale
/// lock arbitration.
pub fn listener_available(path: &Path) -> bool {
    imp::listener_available(path)
}

/// 进程是否存活。用于陈旧锁判定。
///
/// 拿不准时一律回 `true`（fail-closed）：宁可拒绝抢占，也不能清掉活实例的
/// socket。
pub fn process_alive(pid: u32) -> bool {
    imp::process_alive(pid)
}

/// 把当前进程脱离控制终端（`spawn` 子进程时在 `pre_exec` 里调用）。
pub fn detach_session() -> std::io::Result<()> {
    imp::detach_session()
}

/// 本平台是否支持 socket 传输。
pub fn supported() -> bool {
    imp::SUPPORTED
}

/// 收到过终止信号（SIGTERM）。
static TERMINATED: AtomicBool = AtomicBool::new(false);

/// 装终止信号处理器：只把标志位抬起来，清理由主循环做。
///
/// **只有 daemon 模式能装**：装上之后 SIGTERM 不再默认终止进程，stdio 模式的
/// 主线程阻塞在 stdin 上、看不到这个标志，装了等于把 SIGTERM 吞掉。
pub fn install_termination_handler() -> bool {
    imp::install_termination_handler(&TERMINATED)
}

pub fn termination_requested() -> bool {
    TERMINATED.load(Ordering::SeqCst)
}

impl SocketListener {
    pub fn accept(&self) -> std::io::Result<SocketStream> {
        self.0.accept().map(SocketStream)
    }
}

impl SocketStream {
    pub fn try_clone(&self) -> std::io::Result<Self> {
        self.0.try_clone().map(SocketStream)
    }

    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        self.0.set_read_timeout(timeout)
    }
}

impl Read for SocketStream {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buffer)
    }
}

impl Write for SocketStream {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.0.write(buffer)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}
