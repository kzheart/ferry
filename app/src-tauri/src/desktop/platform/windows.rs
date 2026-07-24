use std::path::Path;

use super::{TerminalLaunch, TerminalPreference};

/// Windows 实现预留在平台层。会话、审批和 Tauri command 不应感知其差异。
pub(super) fn reveal_path(_path: &Path) -> Result<(), String> {
    Err("Windows 文件管理器定位尚未实现".to_owned())
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

    use super::{open_terminal, reveal_path, TerminalLaunch, TerminalPreference};

    #[test]
    fn unsupported_desktop_actions_fail_closed() {
        assert_eq!(
            reveal_path(Path::new(r"C:\fixture\session.jsonl")),
            Err("Windows 文件管理器定位尚未实现".to_owned()),
        );
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
