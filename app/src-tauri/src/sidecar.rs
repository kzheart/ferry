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
    resource_dir.join(bundled_engine_name(cfg!(target_os = "windows")))
}

fn bundled_engine_name(is_windows: bool) -> &'static str {
    if is_windows {
        "ferry-engine.exe"
    } else {
        "ferry-engine"
    }
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
pub(crate) async fn engine_rpc(app: tauri::AppHandle, request: String) -> Result<String, String> {
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
