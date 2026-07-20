use serde::Deserialize;
#[cfg(target_os = "macos")]
use std::process::Command;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum TerminalTool {
    Claude,
    Codex,
    Opencode,
}

#[derive(Deserialize)]
pub(crate) struct TerminalLaunch {
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    tool: TerminalTool,
    #[serde(default)]
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    session_id: Option<String>,
    #[serde(default)]
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    cwd: Option<String>,
    #[serde(default)]
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    handoff_doc: Option<String>,
}

#[cfg(target_os = "macos")]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(target_os = "macos")]
fn terminal_command(launch: &TerminalLaunch) -> Result<String, String> {
    let executable = match launch.tool {
        TerminalTool::Claude => "claude",
        TerminalTool::Codex => "codex",
        TerminalTool::Opencode => "opencode",
    };
    let cwd = launch.cwd.as_deref().unwrap_or(".");
    let action = if let Some(doc) = launch.handoff_doc.as_deref() {
        let subcommand = match launch.tool {
            TerminalTool::Opencode => " run",
            _ => "",
        };
        format!("{executable}{subcommand} \"$(cat {})\"", shell_quote(doc))
    } else {
        let id = launch
            .session_id
            .as_deref()
            .ok_or_else(|| "缺少 session_id".to_string())?;
        match launch.tool {
            TerminalTool::Claude => format!("claude --resume {}", shell_quote(id)),
            TerminalTool::Codex => format!("codex resume {}", shell_quote(id)),
            TerminalTool::Opencode => format!("opencode -s {}", shell_quote(id)),
        }
    };
    Ok(format!("cd {} && {action}", shell_quote(cwd)))
}

#[tauri::command]
pub(crate) async fn open_terminal(launch: TerminalLaunch) -> Result<(), String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = launch;
        return Err("当前平台尚未实现安全的终端启动".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        let command = terminal_command(&launch)?;
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
