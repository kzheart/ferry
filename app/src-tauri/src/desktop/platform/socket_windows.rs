//! 本地引擎 socket 的 Windows 客户端（命名管道标记文件）。
//!
//! 引擎 `bind` 会在路径上写下 `\\.\pipe\...`；宿主必须用 CreateFileW 打开——
//! `std::fs` 会把管道路径规范化成 `\\?\`，既打不开还会阻塞。

use std::fs::File;
use std::io::{Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::{Path, PathBuf};
use std::ptr::null_mut;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    GetLastError, ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::System::Pipes::{PeekNamedPipe, WaitNamedPipeW};

const CONNECT_RETRIES: usize = 50;
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

pub(super) fn engine_socket_path() -> Result<PathBuf, String> {
    match std::env::var_os("FERRY_ENGINE_SOCKET") {
        Some(value) if !value.is_empty() => Ok(PathBuf::from(value)),
        _ => Ok(super::home_dir()?.join(".ferry").join("engine.sock")),
    }
}

fn to_wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn open_pipe(name: &str) -> Result<File, String> {
    let wide = to_wide(name);
    for _ in 0..CONNECT_RETRIES {
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                null_mut(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        if handle != INVALID_HANDLE_VALUE && !handle.is_null() {
            return Ok(unsafe { File::from_raw_handle(handle) });
        }
        match unsafe { GetLastError() } {
            ERROR_PIPE_BUSY => {
                let _ = unsafe { WaitNamedPipeW(wide.as_ptr(), WAIT_PIPE_MS) };
            }
            ERROR_FILE_NOT_FOUND => {
                return Err(format!("命名管道不存在: {name}"));
            }
            other => {
                return Err(format!(
                    "无法连接 {name}: {}",
                    std::io::Error::from_raw_os_error(other as i32)
                ));
            }
        }
    }
    Err(format!("命名管道忙，连接超时: {name}"))
}

fn read_line_with_timeout(stream: &mut File, timeout: Duration) -> Result<String, String> {
    let deadline = Instant::now() + timeout;
    let mut line = Vec::new();
    loop {
        let mut available = 0u32;
        let ok = unsafe {
            PeekNamedPipe(
                stream.as_raw_handle(),
                null_mut(),
                0,
                null_mut(),
                &mut available,
                null_mut(),
            )
        };
        if ok == 0 {
            return Err(format!(
                "读取引擎管道失败: {}",
                std::io::Error::last_os_error()
            ));
        }
        if available == 0 {
            if Instant::now() >= deadline {
                return Err("读取引擎管道超时".to_owned());
            }
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }
        let mut buffer = [0u8; 4096];
        let size = usize::try_from(available)
            .unwrap_or(buffer.len())
            .min(buffer.len());
        let read = stream
            .read(&mut buffer[..size])
            .map_err(|error| format!("读取引擎管道失败: {error}"))?;
        if read == 0 {
            return Err("引擎管道未应答就关闭了连接".to_owned());
        }
        line.extend_from_slice(&buffer[..read]);
        if line.contains(&b'\n') {
            let end = line
                .iter()
                .position(|byte| *byte == b'\n')
                .unwrap_or(line.len());
            return String::from_utf8(line[..end].to_vec())
                .map(|text| text.trim_end_matches('\r').to_owned())
                .map_err(|error| format!("引擎管道返回了非法 UTF-8: {error}"));
        }
    }
}

pub(super) fn engine_socket_call(
    socket: &Path,
    request: &str,
    timeout: Duration,
) -> Result<String, String> {
    let pipe = std::fs::read_to_string(socket)
        .map_err(|error| format!("无法读取 {}: {error}", socket.display()))?;
    let pipe = pipe.trim();
    if pipe.is_empty() {
        return Err(format!("{} 不是有效的管道标记", socket.display()));
    }
    let mut stream = open_pipe(pipe)?;
    writeln!(stream, "{request}")
        .and_then(|_| stream.flush())
        .map_err(|error| format!("写入引擎管道失败: {error}"))?;
    read_line_with_timeout(&mut stream, timeout)
}
