//! Tauri 壳:不含任何会话格式知识,只做两件事——
//! 1. engine_rpc:把前端请求转发给 Python 引擎(engine/api.py rpc)
//! 2. open_terminal:在 Terminal.app 里执行接续命令

use std::path::PathBuf;
use std::process::Command;

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

#[tauri::command]
async fn engine_rpc(request: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let out = Command::new("python3")
            .args(["-m", "engine.api", "rpc", &request])
            .current_dir(repo_root())
            .output()
            .map_err(|e| format!("启动引擎失败: {e}"))?;
        if !out.status.success() && out.stdout.is_empty() {
            return Err(String::from_utf8_lossy(&out.stderr).into_owned());
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn open_terminal(command: String) -> Result<(), String> {
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
