//! Tauri 壳:不含任何会话格式知识,只做两件事——
//! 1. engine_rpc:把前端请求转发给 Python 引擎(engine/api.py rpc)
//! 2. open_terminal:在 Terminal.app 里执行接续命令

use std::path::PathBuf;
use std::process::Command;

/// 引擎仓库根目录:优先 SESSION_BRIDGE_REPO 环境变量,
/// 否则取本 crate 上两级(app/src-tauri → 仓库根,开发形态)。
fn repo_root() -> PathBuf {
    if let Ok(p) = std::env::var("SESSION_BRIDGE_REPO") {
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

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![engine_rpc, open_terminal])
        .run(tauri::generate_context!())
        .expect("Session Bridge 启动失败");
}
