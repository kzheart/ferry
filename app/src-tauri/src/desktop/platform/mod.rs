//! 桌面平台能力边界。
//!
//! 业务命令只依赖这里暴露的能力；新增 Windows 实现时替换 windows.rs，
//! 不需要把平台判断散落回 Tauri command 或会话逻辑。

use serde::Deserialize;
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

/// 已经由 Rust 边界验证的终端启动描述符。平台实现不能接受原始 shell 文本。
#[derive(Deserialize)]
pub(crate) struct TerminalLaunch {
    pub(crate) executable: String,
    #[serde(default)]
    pub(crate) args: Vec<String>,
    #[serde(default)]
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

#[cfg(target_os = "macos")]
use macos as imp;
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
