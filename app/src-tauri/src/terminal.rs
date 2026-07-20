use serde::Deserialize;
#[cfg(target_os = "macos")]
use std::process::Command;

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
    #[serde(default)]
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    handoff_doc: Option<String>,
}

fn allowed_executables(app: &tauri::AppHandle) -> Result<Vec<String>, String> {
    use tauri::Manager;
    let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
    let raw =
        crate::sidecar::engine_request_blocking(&resource_dir, r#"{"method":"tools"}"#)?;
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
fn terminal_command(launch: &TerminalLaunch) -> String {
    let cwd = launch.cwd.as_deref().unwrap_or(".");
    let mut action = launch
        .args
        .iter()
        .fold(launch.executable.clone(), |acc, arg| {
            format!("{acc} {}", shell_quote(arg))
        });
    if let Some(doc) = launch.handoff_doc.as_deref() {
        action = format!("{action} \"$(cat {})\"", shell_quote(doc));
    }
    format!("cd {} && {action}", shell_quote(cwd))
}

#[tauri::command]
pub(crate) async fn open_terminal(
    app: tauri::AppHandle,
    launch: TerminalLaunch,
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
        let _ = launch;
        Err("当前平台尚未实现安全的终端启动".to_string())
    }

    #[cfg(target_os = "macos")]
    {
        let command = terminal_command(&launch);
        let esc = command.replace('\\', "\\\\").replace('"', "\\\"");
        let script =
            format!("tell application \"Terminal\"\nactivate\ndo script \"{esc}\"\nend tell");
        let out = Command::new("osascript")
            .args(["-e", &script])
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).into_owned());
        }
        Ok(())
    }
}
