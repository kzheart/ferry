use serde::Deserialize;
#[cfg(target_os = "macos")]
use std::{
    fs,
    path::PathBuf,
    process::Command,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// 通用终端启动描述符：由引擎按 manifest 生成，前端只透传。
/// executable 必须命中引擎 manifest 声明的白名单，拒绝前端拼装命令。
#[derive(Deserialize)]
pub(crate) struct TerminalLaunch {
    executable: String,
    #[serde(default)]
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    args: Vec<String>,
    #[serde(default)]
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    cwd: Option<String>,
}

#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TerminalApp {
    Auto,
    Terminal,
    Iterm,
    Warp,
}

#[cfg(any(target_os = "macos", test))]
impl TerminalApp {
    fn from_preference(value: Option<&str>) -> Self {
        match value {
            Some("terminal") => Self::Terminal,
            Some("iterm") => Self::Iterm,
            Some("warp") => Self::Warp,
            _ => Self::Auto,
        }
    }
}

fn allowed_executables(app: &tauri::AppHandle) -> Result<Vec<String>, String> {
    use tauri::Manager;
    let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
    let raw = crate::sidecar::engine_request_blocking(&resource_dir, r#"{"method":"tools"}"#)?;
    let response: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("manifest 解析失败: {e}"))?;
    if response.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
        return Err("引擎 manifest 获取失败".to_string());
    }
    let manifests = response
        .get("result")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "引擎 manifest 结构非法".to_string())?;
    Ok(manifests
        .iter()
        .filter_map(|m| m.get("executables").and_then(serde_json::Value::as_array))
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect())
}

#[cfg(target_os = "macos")]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(target_os = "macos")]
fn terminal_action(launch: &TerminalLaunch) -> String {
    launch
        .args
        .iter()
        .fold(launch.executable.clone(), |acc, arg| {
            format!("{acc} {}", shell_quote(arg))
        })
}

#[cfg(target_os = "macos")]
fn terminal_command(launch: &TerminalLaunch) -> String {
    let cwd = launch.cwd.as_deref().unwrap_or(".");
    format!("cd {} && {}", shell_quote(cwd), terminal_action(launch))
}

#[cfg(target_os = "macos")]
fn applescript_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(target_os = "macos")]
fn run_applescript(script: String) -> Result<(), String> {
    let out = Command::new("osascript")
        .args(["-e", &script])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn open_system_terminal(command: &str) -> Result<(), String> {
    let script = format!(
        "tell application \"Terminal\"\nactivate\ndo script \"{}\"\nend tell",
        applescript_escape(command)
    );
    run_applescript(script)
}

#[cfg(target_os = "macos")]
fn open_iterm(command: &str) -> Result<(), String> {
    let script = format!(
        "tell application \"iTerm2\"\nactivate\ncreate window with default profile command \"{}\"\nend tell",
        applescript_escape(command)
    );
    run_applescript(script)
}

#[cfg(target_os = "macos")]
fn toml_basic_string(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\r', "\\r")
        .replace('\n', "\\n")
}

#[cfg(target_os = "macos")]
fn warp_tab_config_path() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or_else(|| "无法定位用户主目录".to_string())?;
    let dir = PathBuf::from(home).join(".warp").join("tab_configs");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    Ok(dir.join(format!("ferry_resume_{stamp}.toml")))
}

#[cfg(target_os = "macos")]
fn warp_cwd(launch: &TerminalLaunch) -> Result<String, String> {
    let cwd = launch.cwd.as_deref().unwrap_or(".");
    let path = PathBuf::from(cwd);
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    Ok(absolute.to_string_lossy().into_owned())
}

#[cfg(target_os = "macos")]
fn open_warp(launch: &TerminalLaunch) -> Result<(), String> {
    // Warp 已用 Tab Config 取代旧的 Launch Configuration；使用公开 URI，
    // 不依赖键盘模拟或未公开的 GUI 自动化接口。
    let path = warp_tab_config_path()?;
    let content = format!(
        "name = \"Ferry resume\"\ntitle = \"Ferry\"\n\n[[panes]]\nid = \"resume\"\ntype = \"terminal\"\ndirectory = \"{}\"\ncommands = [\"{}\"]\nis_focused = true\n",
        toml_basic_string(&warp_cwd(launch)?),
        toml_basic_string(&terminal_action(launch)),
    );
    fs::write(&path, content).map_err(|e| e.to_string())?;
    let name = path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "Warp Tab Config 文件名无效".to_string())?;
    let uri = format!("warp://tab_config/{name}?new_window=true");
    let status = Command::new("open")
        .arg(&uri)
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        let _ = fs::remove_file(&path);
        return Err("无法启动 Warp".to_string());
    }
    // Warp 需要先发现并读取配置；随后移除一次性文件，避免污染用户的配置列表。
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(30));
        let _ = fs::remove_file(path);
    });
    Ok(())
}

#[cfg(target_os = "macos")]
fn open_macos_terminal(launch: &TerminalLaunch, preference: TerminalApp) -> Result<(), String> {
    let command = terminal_command(launch);
    match preference {
        TerminalApp::Terminal => open_system_terminal(&command),
        TerminalApp::Iterm => open_iterm(&command).or_else(|_| open_system_terminal(&command)),
        TerminalApp::Warp => open_warp(launch).or_else(|_| open_system_terminal(&command)),
        TerminalApp::Auto => open_warp(launch)
            .or_else(|_| open_iterm(&command))
            .or_else(|_| open_system_terminal(&command)),
    }
}

#[tauri::command]
pub(crate) async fn open_terminal(
    app: tauri::AppHandle,
    launch: TerminalLaunch,
    terminal_app: Option<String>,
) -> Result<(), String> {
    let allowed = tauri::async_runtime::spawn_blocking({
        let app = app.clone();
        move || allowed_executables(&app)
    })
    .await
    .map_err(|e| e.to_string())??;
    if !allowed.iter().any(|exe| exe == &launch.executable) {
        return Err(format!(
            "拒绝启动: {} 不在引擎 manifest 白名单内",
            launch.executable
        ));
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (launch, terminal_app);
        Err("当前平台尚未实现安全的终端启动".to_string())
    }

    #[cfg(target_os = "macos")]
    {
        open_macos_terminal(
            &launch,
            TerminalApp::from_preference(terminal_app.as_deref()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::TerminalApp;

    #[test]
    fn terminal_preference_defaults_to_auto() {
        assert_eq!(TerminalApp::from_preference(None), TerminalApp::Auto);
        assert_eq!(
            TerminalApp::from_preference(Some("unknown")),
            TerminalApp::Auto
        );
        assert_eq!(
            TerminalApp::from_preference(Some("warp")),
            TerminalApp::Warp
        );
    }
}
