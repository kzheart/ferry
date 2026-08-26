//! Windows 本地传输：命名管道 + 路径上的标记文件。
//!
//! Unix 侧 `bind(path)` 会在 `path` 上留下 socket inode；锁文件与陈旧清理都靠
//! `path.exists()`。Windows 没有等价物，所以这里：
//!
//! 1. 真正听的是 `\\.\pipe\ferry-<user>-<hash>`（拒绝远程客户端）；
//! 2. `path` 上写一行管道名，当作标记文件——`lock.rs` 不用改。
//!
//! 业务代码仍然只看见 [`super::SocketListener`] / [`super::SocketStream`]。

use std::fs::File;
use std::io::{self, Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::Path;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY,
    ERROR_PIPE_CONNECTED, FALSE, HANDLE, INVALID_HANDLE_VALUE, STILL_ACTIVE, TRUE,
};
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PeekNamedPipe, WaitNamedPipeW, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

pub(super) const SUPPORTED: bool = true;

const PIPE_BUFFER: u32 = 64 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
const WAIT_PIPE_MS: u32 = 200;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const OPEN_EXISTING: u32 = 3;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateFileW(
        lp_file_name: *const u16,
        dw_desired_access: u32,
        dw_share_mode: u32,
        lp_security_attributes: *mut core::ffi::c_void,
        dw_creation_disposition: u32,
        dw_flags_and_attributes: u32,
        h_template_file: HANDLE,
    ) -> HANDLE;
}

pub(super) struct Listener {
    name_wide: Vec<u16>,
    next: Mutex<File>,
}

pub(super) struct Stream {
    file: File,
    read_timeout: Arc<Mutex<Option<Duration>>>,
}

impl Stream {
    fn new(file: File) -> Self {
        Self {
            file,
            read_timeout: Arc::new(Mutex::new(None)),
        }
    }
}

fn to_wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn last_error() -> io::Error {
    io::Error::from_raw_os_error(unsafe { GetLastError() } as i32)
}

fn pipe_name_for(path: &Path) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    let user: String = std::env::var("USERNAME")
        .unwrap_or_else(|_| "user".into())
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(16)
        .collect();
    format!(r"\\.\pipe\ferry-{user}-{:016x}", hasher.finish())
}

fn create_instance(name_wide: &[u16], first: bool) -> io::Result<File> {
    let mut open_mode = PIPE_ACCESS_DUPLEX;
    if first {
        open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }
    let handle = unsafe {
        CreateNamedPipeW(
            name_wide.as_ptr(),
            open_mode,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            PIPE_BUFFER,
            PIPE_BUFFER,
            0,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE || handle.is_null() {
        return Err(last_error());
    }
    Ok(unsafe { File::from_raw_handle(handle) })
}

fn connect_instance(file: &File) -> io::Result<()> {
    let ok = unsafe { ConnectNamedPipe(file.as_raw_handle(), null_mut()) };
    if ok != FALSE {
        return Ok(());
    }
    if unsafe { GetLastError() } == ERROR_PIPE_CONNECTED {
        return Ok(());
    }
    Err(last_error())
}

fn wait_until_readable(file: &File, timeout: Option<Duration>) -> io::Result<()> {
    let Some(timeout) = timeout else {
        return Ok(());
    };
    let deadline = Instant::now() + timeout;
    loop {
        let mut available = 0u32;
        let ok = unsafe {
            PeekNamedPipe(
                file.as_raw_handle(),
                null_mut(),
                0,
                null_mut(),
                &mut available,
                null_mut(),
            )
        };
        if ok == FALSE {
            return Err(last_error());
        }
        if available > 0 {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "命名管道读取超时"));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn open_existing_pipe(name: &str) -> io::Result<File> {
    let wide = to_wide(name);
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "命名管道忙，连接超时",
            ));
        }
        let wait_ms = remaining.as_millis().min(WAIT_PIPE_MS as u128).max(1) as u32;
        let ready = unsafe { WaitNamedPipeW(wide.as_ptr(), wait_ms) };
        if ready == FALSE {
            match unsafe { GetLastError() } {
                ERROR_FILE_NOT_FOUND => {
                    return Err(io::Error::new(io::ErrorKind::NotFound, "命名管道不存在"));
                }
                ERROR_PIPE_BUSY => continue,
                _ if Instant::now() < deadline => continue,
                other => return Err(io::Error::from_raw_os_error(other as i32)),
            }
        }
        let handle = create_file_w(wide.as_ptr());
        if handle != INVALID_HANDLE_VALUE && !handle.is_null() {
            return Ok(unsafe { File::from_raw_handle(handle) });
        }
        match unsafe { GetLastError() } {
            ERROR_PIPE_BUSY => {
                continue;
            }
            ERROR_FILE_NOT_FOUND => {
                return Err(io::Error::new(io::ErrorKind::NotFound, "命名管道不存在"));
            }
            other => {
                return Err(io::Error::from_raw_os_error(other as i32));
            }
        }
    }
}

/// 必须走 CreateFileW：`std::fs` 会把 `\\.\pipe\` 规范化成 `\\?\`，管道打不开还会一直阻塞。
fn create_file_w(name: *const u16) -> HANDLE {
    unsafe {
        CreateFileW(
            name,
            GENERIC_READ | GENERIC_WRITE,
            0,
            null_mut(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    }
}

pub(super) fn bind(path: &Path) -> Result<Listener, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("无法创建 socket 目录 {}: {error}", parent.display()))?;
    }
    let pipe = pipe_name_for(path);
    let name_wide = to_wide(&pipe);
    let first = create_instance(&name_wide, true)
        .map_err(|error| format!("无法创建命名管道 {pipe}: {error}"))?;
    if let Err(error) = std::fs::write(path, format!("{pipe}\n")) {
        drop(first);
        return Err(format!("无法写入管道标记 {}: {error}", path.display()));
    }
    Ok(Listener {
        name_wide,
        next: Mutex::new(first),
    })
}

pub(super) fn connect(path: &Path) -> Result<Stream, String> {
    let pipe = std::fs::read_to_string(path)
        .map_err(|error| format!("无法读取 {}: {error}", path.display()))?;
    let pipe = pipe.trim();
    if pipe.is_empty() {
        return Err(format!("{} 不是有效的管道标记", path.display()));
    }
    open_existing_pipe(pipe)
        .map(Stream::new)
        .map_err(|error| format!("无法连接 {pipe}: {error}"))
}

pub(super) fn listener_available(path: &Path) -> bool {
    let Ok(pipe) = std::fs::read_to_string(path) else {
        return false;
    };
    let pipe = pipe.trim();
    if pipe.is_empty() {
        return false;
    }
    let wide = to_wide(pipe);
    if unsafe { WaitNamedPipeW(wide.as_ptr(), 0) } != FALSE {
        return true;
    }
    // BUSY/timeout/access-denied still prove that the pipe exists. Only an
    // absent pipe is stale. This check never consumes an accept instance.
    (unsafe { GetLastError() }) != ERROR_FILE_NOT_FOUND
}

pub(super) fn process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid);
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return GetLastError() == ERROR_ACCESS_DENIED;
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(handle, &mut code);
        CloseHandle(handle);
        ok != FALSE && code == STILL_ACTIVE as u32
    }
}

pub(super) fn detach_session() -> std::io::Result<()> {
    Ok(())
}

static TERMINATION_FLAG: OnceLock<&'static AtomicBool> = OnceLock::new();

unsafe extern "system" fn on_console_ctrl(_ctrl_type: u32) -> i32 {
    if let Some(flag) = TERMINATION_FLAG.get() {
        flag.store(true, Ordering::SeqCst);
    }
    TRUE
}

pub(super) fn install_termination_handler(flag: &'static AtomicBool) -> bool {
    let _ = TERMINATION_FLAG.set(flag);
    unsafe { SetConsoleCtrlHandler(Some(on_console_ctrl), TRUE) != FALSE }
}

impl Listener {
    pub(super) fn accept(&self) -> std::io::Result<Stream> {
        let mut next = self
            .next
            .lock()
            .map_err(|_| io::Error::other("命名管道监听锁中毒"))?;
        connect_instance(&next)?;
        let connected = std::mem::replace(&mut *next, create_instance(&self.name_wide, false)?);
        Ok(Stream::new(connected))
    }
}

impl Stream {
    pub(super) fn try_clone(&self) -> std::io::Result<Self> {
        Ok(Self {
            file: self.file.try_clone()?,
            read_timeout: Arc::clone(&self.read_timeout),
        })
    }

    pub(super) fn set_read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        *self
            .read_timeout
            .lock()
            .map_err(|_| io::Error::other("命名管道超时锁中毒"))? = timeout;
        Ok(())
    }
}

impl Read for Stream {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let timeout = *self
            .read_timeout
            .lock()
            .map_err(|_| io::Error::other("命名管道超时锁中毒"))?;
        wait_until_readable(&self.file, timeout)?;
        self.file.read(buffer)
    }
}

impl Write for Stream {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.file.write(buffer)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};

    #[test]
    fn stream_read_timeout_is_enforced() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("engine.sock");
        let listener = bind(&marker).unwrap();
        let server = std::thread::spawn(move || {
            let _stream = listener.accept().unwrap();
            std::thread::sleep(Duration::from_millis(200));
        });

        let mut client = connect(&marker).unwrap();
        client
            .set_read_timeout(Some(Duration::from_millis(30)))
            .unwrap();
        let error = client.read(&mut [0u8; 1]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        drop(client);
        server.join().unwrap();
    }

    #[test]
    fn availability_probe_does_not_consume_the_pending_instance() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("engine.sock");
        let listener = bind(&marker).unwrap();

        assert!(listener_available(&marker));
        assert!(listener_available(&marker));

        let client = connect(&marker).expect("探活后仍应能连接");
        let server = listener.accept().expect("pending instance 未被探活消费");
        drop(client);
        drop(server);
    }

    #[test]
    fn cloned_stream_round_trips_a_line() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("engine.sock");
        let listener = bind(&marker).unwrap();
        let server = std::thread::spawn(move || {
            let mut stream = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            reader.read_line(&mut request).unwrap();
            assert_eq!(request, "ping\n");
            stream.write_all(b"pong\n").unwrap();
            stream.flush().unwrap();
        });

        let mut client = connect(&marker).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut reader = BufReader::new(client.try_clone().unwrap());
        client.write_all(b"ping\n").unwrap();
        client.flush().unwrap();
        let mut response = String::new();
        reader.read_line(&mut response).unwrap();
        assert_eq!(response, "pong\n");
        server.join().unwrap();
    }
}
