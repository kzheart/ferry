use serde::Deserialize;
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
    tool: TerminalTool,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
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

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::{shell_quote, terminal_command, TerminalLaunch, TerminalTool};

    fn resume_launch(tool: TerminalTool) -> TerminalLaunch {
        TerminalLaunch {
            tool,
            session_id: Some("session '42'".to_string()),
            cwd: Some("/tmp/project dir's".to_string()),
            handoff_doc: None,
        }
    }

    fn handoff_launch(tool: TerminalTool) -> TerminalLaunch {
        TerminalLaunch {
            tool,
            session_id: None,
            cwd: Some("/tmp/project dir's".to_string()),
            handoff_doc: Some("/tmp/handoff doc's.md".to_string()),
        }
    }

    #[test]
    fn shell_quote_safely_escapes_single_quotes_and_shell_syntax() {
        assert_eq!(shell_quote("plain text"), "'plain text'");
        assert_eq!(
            shell_quote("a'b; $(touch /tmp/pwned)"),
            "'a'\\''b; $(touch /tmp/pwned)'"
        );
    }

    #[test]
    fn constructs_resume_commands_for_each_tool() {
        let cwd = "'/tmp/project dir'\\''s'";
        let id = "'session '\\''42'\\'''";

        assert_eq!(
            terminal_command(&resume_launch(TerminalTool::Claude)).unwrap(),
            format!("cd {cwd} && claude --resume {id}")
        );
        assert_eq!(
            terminal_command(&resume_launch(TerminalTool::Codex)).unwrap(),
            format!("cd {cwd} && codex resume {id}")
        );
        assert_eq!(
            terminal_command(&resume_launch(TerminalTool::Opencode)).unwrap(),
            format!("cd {cwd} && opencode -s {id}")
        );
    }

    #[test]
    fn constructs_handoff_commands_for_each_tool() {
        let cwd = "'/tmp/project dir'\\''s'";
        let doc = "'/tmp/handoff doc'\\''s.md'";

        assert_eq!(
            terminal_command(&handoff_launch(TerminalTool::Claude)).unwrap(),
            format!("cd {cwd} && claude \"$(cat {doc})\"")
        );
        assert_eq!(
            terminal_command(&handoff_launch(TerminalTool::Codex)).unwrap(),
            format!("cd {cwd} && codex \"$(cat {doc})\"")
        );
        assert_eq!(
            terminal_command(&handoff_launch(TerminalTool::Opencode)).unwrap(),
            format!("cd {cwd} && opencode run \"$(cat {doc})\"")
        );
    }

    #[test]
    fn resume_command_requires_a_session_id() {
        let launch = TerminalLaunch {
            tool: TerminalTool::Claude,
            session_id: None,
            cwd: None,
            handoff_doc: None,
        };

        assert_eq!(
            terminal_command(&launch),
            Err("缺少 session_id".to_string())
        );
    }
}
