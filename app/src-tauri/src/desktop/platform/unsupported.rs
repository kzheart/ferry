use std::path::{Path, PathBuf};

use super::{TerminalLaunch, TerminalPreference};

pub(super) fn reveal_path(_path: &Path) -> Result<(), String> {
    Err("当前平台尚未支持在文件管理器中定位".to_owned())
}

pub(super) fn open_terminal(
    _launch: &TerminalLaunch,
    _preference: TerminalPreference,
) -> Result<(), String> {
    Err("当前平台尚未实现安全的终端启动".to_owned())
}

pub(super) fn home_dir() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "无法定位用户主目录".to_owned())
}

pub(super) fn cli_link_path() -> Result<PathBuf, String> {
    Err("当前平台尚未实现命令行工具安装".to_owned())
}

pub(super) fn create_cli_link(_link: &Path, _target: &Path) -> Result<(), String> {
    Err("当前平台尚未实现命令行工具安装".to_owned())
}

pub(super) fn process_alive(_pid: u32) -> bool {
    false
}
