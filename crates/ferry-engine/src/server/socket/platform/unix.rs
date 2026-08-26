//! unix domain socket 实现。
//!
//! 权限模型：socket 文件建成后立刻 chmod 0600，且它落在 `~/.ferry` 里——
//! 只有本用户可达，不监听任何网络地址。

use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

pub(super) const SUPPORTED: bool = true;

pub(super) struct Listener(UnixListener);
pub(super) struct Stream(UnixStream);

pub(super) fn bind(path: &Path) -> Result<Listener, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("无法创建 socket 目录 {}: {error}", parent.display()))?;
    }
    let listener = UnixListener::bind(path)
        .map_err(|error| format!("无法监听 {}: {error}", path.display()))?;
    // bind 与 chmod 之间有一瞬窗口；`~/.ferry` 本身不对外开放，窗口内也进不来。
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("无法收紧 {} 的权限: {error}", path.display()))?;
    Ok(Listener(listener))
}

pub(super) fn connect(path: &Path) -> Result<Stream, String> {
    UnixStream::connect(path)
        .map(Stream)
        .map_err(|error| format!("无法连接 {}: {error}", path.display()))
}

pub(super) fn listener_available(path: &Path) -> bool {
    connect(path).is_ok()
}

pub(super) fn process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    // `kill(pid, 0)`：成功即存活；EPERM 说明进程在但不属于本用户，同样算活。
    let code = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if code == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

pub(super) fn detach_session() -> std::io::Result<()> {
    // setsid 让 daemon 脱离调用者的控制终端与进程组：终端关掉不会带走它。
    if unsafe { libc::setsid() } == -1 {
        let error = std::io::Error::last_os_error();
        // 已经是会话首进程（EPERM）不算失败。
        if error.raw_os_error() != Some(libc::EPERM) {
            return Err(error);
        }
    }
    Ok(())
}

/// 信号处理器只能碰这一个标志位（原子写是 async-signal-safe 的少数动作之一）。
static TERMINATION_FLAG: OnceLock<&'static AtomicBool> = OnceLock::new();

extern "C" fn on_termination(_signal: libc::c_int) {
    if let Some(flag) = TERMINATION_FLAG.get() {
        flag.store(true, Ordering::SeqCst);
    }
}

pub(super) fn install_termination_handler(flag: &'static AtomicBool) -> bool {
    let _ = TERMINATION_FLAG.set(flag);
    let handler = on_termination as *const () as libc::sighandler_t;
    unsafe { libc::signal(libc::SIGTERM, handler) != libc::SIG_ERR }
}

impl Listener {
    pub(super) fn accept(&self) -> std::io::Result<Stream> {
        self.0.accept().map(|(stream, _address)| Stream(stream))
    }
}

impl Stream {
    pub(super) fn try_clone(&self) -> std::io::Result<Self> {
        self.0.try_clone().map(Stream)
    }

    pub(super) fn set_read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        self.0.set_read_timeout(timeout)
    }
}

impl Read for Stream {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buffer)
    }
}

impl Write for Stream {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.0.write(buffer)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}
