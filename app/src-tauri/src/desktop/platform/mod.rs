//! 桌面平台能力边界。
//!
//! 业务命令只依赖这里暴露的能力；macOS 与 Windows 的系统调用分别留在各自
//! 实现中，不把平台判断散落回 Tauri command 或会话逻辑。

use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod unsupported;
#[cfg(target_os = "windows")]
mod windows;

// 引擎 socket 的边界按 unix / windows / 其余划:unix 走 domain socket,Windows 走
// 命名管道（标记文件路径与 unix 相同），其余 fail-closed。
#[cfg(unix)]
mod socket_unix;
#[cfg(not(any(unix, target_os = "windows")))]
mod socket_unsupported;
#[cfg(target_os = "windows")]
mod socket_windows;

pub(crate) fn reveal_path(path: &Path) -> Result<(), String> {
    imp::reveal_path(path)
}

/// 已经由 Rust 边界验证的终端启动描述符。平台实现不能接受原始 shell 文本。
/// args/cwd 由 macOS 与 Windows 的终端实现读取；字段属于跨平台前端契约，
/// 不能按平台裁剪。
#[derive(Deserialize)]
pub(crate) struct TerminalLaunch {
    pub(crate) executable: String,
    #[serde(default)]
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(crate) args: Vec<String>,
    #[serde(default)]
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(crate) cwd: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerminalPreference {
    Auto,
    Terminal,
    Iterm,
    Warp,
}

impl TerminalPreference {
    pub(crate) fn from_option(value: Option<&str>) -> Self {
        match value {
            Some("terminal") => Self::Terminal,
            Some("iterm") => Self::Iterm,
            Some("warp") => Self::Warp,
            _ => Self::Auto,
        }
    }
}

pub(crate) fn open_terminal(
    launch: &TerminalLaunch,
    preference: TerminalPreference,
) -> Result<(), String> {
    imp::open_terminal(launch, preference)
}

/// 用户主目录。`~/.claude/skills` 这类契约路径的展开点,平台各自决定读哪个变量。
pub(crate) fn home_dir() -> Result<PathBuf, String> {
    imp::home_dir()
}

/// `ferry` CLI 的安装点。macOS 是 `~/.local/bin/ferry`;
/// Windows 是 `%LOCALAPPDATA%\Ferry\bin\ferry.cmd` 并写入用户 PATH。
pub(crate) fn cli_link_path() -> Result<PathBuf, String> {
    imp::cli_link_path()
}

/// 建立指向引擎二进制的 CLI 入口(macOS 是 symlink, Windows 是 .cmd 垫片)。
pub(crate) fn create_cli_link(link: &Path, target: &Path) -> Result<(), String> {
    imp::create_cli_link(link, target)
}

/// 移除 Ferry CLI 入口及平台安装时附带的环境配置。
pub(crate) fn remove_cli_link(link: &Path) -> Result<(), String> {
    imp::remove_cli_link(link)
}

/// CLI 入口实际指向的引擎路径。断链或不是 Ferry 装的入口时返回 `None`。
pub(crate) fn resolve_cli_link(link: &Path) -> Option<PathBuf> {
    imp::resolve_cli_link(link)
}

/// 已装入口的内容过期、需要覆盖写（例如 Windows 垫片还带着 `\\?\` 前缀）。
pub(crate) fn cli_link_needs_rewrite(link: &Path) -> bool {
    imp::cli_link_needs_rewrite(link)
}

/// 建立一个指向目录的入口。macOS 是 symlink, Windows 是 junction。
pub(crate) fn create_directory_link(link: &Path, target: &Path) -> Result<(), String> {
    imp::create_directory_link(link, target)
}

/// 摘掉目录入口本身,不删除它指向的真身。
pub(crate) fn remove_directory_link(link: &Path) -> Result<(), String> {
    imp::remove_directory_link(link)
}

/// 进程是否还活着。只探测,不发真实信号。
pub(crate) fn process_alive(pid: u32) -> bool {
    imp::process_alive(pid)
}

/// 引擎监听的本地 socket 路径。`Err` 表示这个平台上没有共享引擎这回事,
/// 调用方据此完全跳过 socket 相关的启动步骤。
pub(crate) fn engine_socket_path() -> Result<PathBuf, String> {
    socket_imp::engine_socket_path()
}

/// 在引擎 socket 上做一次一问一答的调用(请求必须是单行 JSONL,不含换行)。
pub(crate) fn engine_socket_call(
    socket: &Path,
    request: &str,
    timeout: Duration,
) -> Result<String, String> {
    socket_imp::engine_socket_call(socket, request, timeout)
}

#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(unix)]
use socket_unix as socket_imp;
#[cfg(not(any(unix, target_os = "windows")))]
use socket_unsupported as socket_imp;
#[cfg(target_os = "windows")]
use socket_windows as socket_imp;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
use unsupported as imp;
#[cfg(target_os = "windows")]
use windows as imp;

#[cfg(test)]
mod tests {
    use super::TerminalPreference;

    #[test]
    fn terminal_preference_defaults_to_auto() {
        assert_eq!(
            TerminalPreference::from_option(None),
            TerminalPreference::Auto
        );
        assert_eq!(
            TerminalPreference::from_option(Some("unknown")),
            TerminalPreference::Auto
        );
        assert_eq!(
            TerminalPreference::from_option(Some("warp")),
            TerminalPreference::Warp
        );
    }
}
