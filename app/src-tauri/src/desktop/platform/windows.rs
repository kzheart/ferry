use std::path::{Path, PathBuf};

use super::{TerminalLaunch, TerminalPreference};

/// Windows 实现预留在平台层。会话、审批和 Tauri command 不应感知其差异。
pub(super) fn reveal_path(_path: &Path) -> Result<(), String> {
    Err("Windows 文件管理器定位尚未实现".to_owned())
}

pub(super) fn home_dir() -> Result<PathBuf, String> {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .ok_or_else(|| "无法定位用户主目录".to_owned())
}

/// CLI 安装形态是平台约定:Windows 要落 `%LOCALAPPDATA%\Ferry\bin` 并改用户 PATH,
/// 与 macOS 的 symlink 不是一回事,在实现之前显式 fail-closed。
pub(super) fn cli_link_path() -> Result<PathBuf, String> {
    Err("Windows 命令行工具安装尚未实现".to_owned())
}

pub(super) fn create_cli_link(_link: &Path, _target: &Path) -> Result<(), String> {
    Err("Windows 命令行工具安装尚未实现".to_owned())
}

/// 进程存活探测需要 OpenProcess/GetExitCodeProcess,尚未实现;
/// 一律当作「不在运行」,引擎服务分区因此只会显示未运行,不会误报。
pub(super) fn process_alive(_pid: u32) -> bool {
    false
}

pub(super) fn open_terminal(
    _launch: &TerminalLaunch,
    _preference: TerminalPreference,
) -> Result<(), String> {
    Err("Windows 安全终端启动尚未实现".to_owned())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        cli_link_path, create_cli_link, open_terminal, process_alive, reveal_path, TerminalLaunch,
        TerminalPreference,
    };

    #[test]
    fn unsupported_desktop_actions_fail_closed() {
        assert_eq!(
            reveal_path(Path::new(r"C:\fixture\session.jsonl")),
            Err("Windows 文件管理器定位尚未实现".to_owned()),
        );
        assert_eq!(
            cli_link_path(),
            Err("Windows 命令行工具安装尚未实现".to_owned()),
        );
        assert_eq!(
            create_cli_link(
                Path::new(r"C:\fixture\bin\ferry.exe"),
                Path::new(r"C:\fixture\ferry-engine.exe"),
            ),
            Err("Windows 命令行工具安装尚未实现".to_owned()),
        );
        assert!(!process_alive(1));
        assert_eq!(
            open_terminal(
                &TerminalLaunch {
                    executable: "codex".to_owned(),
                    args: vec!["resume".to_owned(), "session".to_owned()],
                    cwd: Some(r"C:\fixture".to_owned()),
                },
                TerminalPreference::Auto,
            ),
            Err("Windows 安全终端启动尚未实现".to_owned()),
        );
    }
}
