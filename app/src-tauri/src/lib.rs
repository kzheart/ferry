//! Tauri 壳不含会话格式知识，只转发引擎 RPC 和启动受限的接续命令。

use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

const ENGINE_PROTOCOL: u64 = 1;
static ENGINE_HANDSHAKE: OnceLock<Result<(), String>> = OnceLock::new();

/// 引擎仓库根目录:优先 FERRY_REPO 环境变量,
/// 否则取本 crate 上两级(app/src-tauri → 仓库根,开发形态)。
fn repo_root() -> PathBuf {
    if let Ok(p) = std::env::var("FERRY_REPO") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn bundled_engine(resource_dir: &Path) -> PathBuf {
    let name = if cfg!(target_os = "windows") {
        "ferry-engine.exe"
    } else {
        "ferry-engine"
    };
    resource_dir.join(name)
}

fn engine_command(resource_dir: &Path) -> Result<Command, String> {
    let sidecar = bundled_engine(resource_dir);
    if sidecar.is_file() {
        return Ok(Command::new(sidecar));
    }

    #[cfg(debug_assertions)]
    {
        let mut command = Command::new(if cfg!(target_os = "windows") {
            "python"
        } else {
            "python3"
        });
        command.args(["-m", "engine.api"]);
        command.current_dir(repo_root());
        return Ok(command);
    }

    #[cfg(not(debug_assertions))]
    Err(format!("正式包缺少引擎 sidecar: {}", sidecar.display()))
}

#[cfg(target_os = "windows")]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x0800_0000);
}

#[cfg(not(target_os = "windows"))]
fn hide_console(_command: &mut Command) {}

fn run_engine(resource_dir: &Path, args: &[&str]) -> Result<std::process::Output, String> {
    let mut command = engine_command(resource_dir)?;
    command.args(args);
    hide_console(&mut command);
    command
        .output()
        .map_err(|error| format!("启动引擎失败: {error}"))
}

fn check_engine(resource_dir: &Path) -> Result<(), String> {
    let output = run_engine(resource_dir, &["health"])?;
    if !output.status.success() {
        return Err(format!(
            "引擎健康检查失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let health: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("引擎健康检查返回无效 JSON: {error}"))?;
    let protocol = health.get("protocol").and_then(Value::as_u64);
    if health.get("status").and_then(Value::as_str) != Some("ok")
        || protocol != Some(ENGINE_PROTOCOL)
    {
        return Err(format!(
            "引擎协议不兼容: 需要 {ENGINE_PROTOCOL}，实际 {protocol:?}"
        ));
    }
    Ok(())
}

#[tauri::command]
async fn engine_rpc(app: tauri::AppHandle, request: String) -> Result<String, String> {
    use tauri::Manager;
    let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        ENGINE_HANDSHAKE
            .get_or_init(|| check_engine(&resource_dir))
            .clone()?;
        let output = run_engine(&resource_dir, &["rpc", &request])?;
        if !output.status.success() && output.stdout.is_empty() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
        }
        String::from_utf8(output.stdout).map_err(|error| format!("引擎输出不是 UTF-8: {error}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum TerminalTool {
    Claude,
    Codex,
    Opencode,
}

#[derive(Deserialize)]
struct TerminalLaunch {
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
async fn open_terminal(launch: TerminalLaunch) -> Result<(), String> {
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

/// 标题栏高度(与前端 App.jsx 里的 44px 保持一致),红绿灯左边距。
#[cfg(target_os = "macos")]
const TITLEBAR_HEIGHT: f64 = 44.0;
#[cfg(target_os = "macos")]
const TRAFFIC_LIGHT_X: f64 = 14.0;

/// macOS 在窗口显示/聚焦/缩放时会把红绿灯重置回默认位置,
/// 因此不用 tauri.conf 的 trafficLightPosition,而是在窗口事件里反复重摆:
/// 把标题栏容器撑到 TITLEBAR_HEIGHT 高,再把三个按钮垂直居中。
#[cfg(target_os = "macos")]
fn align_traffic_lights(window: &tauri::Window) {
    use objc2_app_kit::{NSWindow, NSWindowButton};
    let Ok(ptr) = window.ns_window() else { return };
    unsafe {
        let ns_window = &*(ptr as *const NSWindow);
        let Some(close) = ns_window.standardWindowButton(NSWindowButton::CloseButton) else {
            return;
        };
        let Some(mini) = ns_window.standardWindowButton(NSWindowButton::MiniaturizeButton) else {
            return;
        };
        let zoom = ns_window.standardWindowButton(NSWindowButton::ZoomButton);
        let Some(container) = close.superview().and_then(|v| v.superview()) else {
            return;
        };

        let mut rect = container.frame();
        rect.size.height = TITLEBAR_HEIGHT;
        rect.origin.y = ns_window.frame().size.height - TITLEBAR_HEIGHT;
        container.setFrame(rect);

        let spacing = mini.frame().origin.x - close.frame().origin.x;
        let mut buttons = vec![close, mini];
        buttons.extend(zoom);
        for (i, button) in buttons.iter().enumerate() {
            let mut frame = button.frame();
            frame.origin.x = TRAFFIC_LIGHT_X + i as f64 * spacing;
            frame.origin.y = (TITLEBAR_HEIGHT - frame.size.height) / 2.0;
            button.setFrameOrigin(frame.origin);
        }
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![engine_rpc, open_terminal])
        .on_window_event(|_window, _event| {
            #[cfg(target_os = "macos")]
            if matches!(
                _event,
                tauri::WindowEvent::Resized(_)
                    | tauri::WindowEvent::Focused(_)
                    | tauri::WindowEvent::ThemeChanged(_)
            ) {
                align_traffic_lights(_window);
            }
        })
        .run(tauri::generate_context!())
        .expect("Ferry 启动失败");
}
