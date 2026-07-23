use crate::contracts::agents::ALLOWED_EXECUTABLES;
use crate::platform::{TerminalLaunch, TerminalPreference};

/// Tauri command 只执行桌面白名单校验；所有平台细节由 platform/ 持有。
#[tauri::command]
pub(crate) async fn open_terminal(
    _app: tauri::AppHandle,
    launch: TerminalLaunch,
    terminal_app: Option<String>,
) -> Result<(), String> {
    if !ALLOWED_EXECUTABLES.contains(&launch.executable.as_str()) {
        return Err(format!(
            "拒绝启动: {} 不在桌面端可执行文件白名单内",
            launch.executable
        ));
    }
    let preference = TerminalPreference::from_option(terminal_app.as_deref());
    tauri::async_runtime::spawn_blocking(move || {
        crate::platform::open_terminal(&launch, preference)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::ALLOWED_EXECUTABLES;

    #[test]
    fn terminal_executables_use_static_policy() {
        assert_eq!(ALLOWED_EXECUTABLES, ["claude", "codex", "opencode"]);
    }
}
