//! 平台能力边界。
//!
//! 业务命令只依赖这里暴露的能力；新增 Windows 实现时替换 windows.rs，
//! 不需要把平台判断散落回 Tauri command 或会话逻辑。

use std::path::Path;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod unsupported;
#[cfg(target_os = "windows")]
mod windows;

pub(crate) fn reveal_path(path: &Path) -> Result<(), String> {
    imp::reveal_path(path)
}

#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
use unsupported as imp;
#[cfg(target_os = "windows")]
use windows as imp;
