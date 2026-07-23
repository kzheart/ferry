use std::path::Path;

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
