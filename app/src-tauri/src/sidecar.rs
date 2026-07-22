use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Mutex, OnceLock};

const ENGINE_PROTOCOL: u64 = 2;

/// 常驻引擎进程:按行请求/响应,避免每次 RPC 冷启动(release 下 PyInstaller 解压开销显著)。
struct EngineProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Drop for EngineProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

static ENGINE: OnceLock<Mutex<Option<EngineProcess>>> = OnceLock::new();

fn spawn_engine(resource_dir: &Path) -> Result<EngineProcess, String> {
    let mut command = engine_command(resource_dir)?;
    command.arg("serve");
    hide_console(&mut command);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|error| format!("启动引擎失败: {error}"))?;
    let stdin = child.stdin.take().ok_or("引擎 stdin 不可用")?;
    let stdout = BufReader::new(child.stdout.take().ok_or("引擎 stdout 不可用")?);
    let mut engine = EngineProcess {
        child,
        stdin,
        stdout,
    };
    handshake(&mut engine)?;
    Ok(engine)
}

/// 协议握手作为常驻进程的首条请求完成:独立的一次性 health 子进程
/// 在 release 下会让 PyInstaller onefile 多解压一整次,冷启动时间翻倍。
fn handshake(engine: &mut EngineProcess) -> Result<(), String> {
    let line = (|| -> std::io::Result<String> {
        engine.stdin.write_all(b"{\"method\":\"health\"}\n")?;
        engine.stdin.flush()?;
        let mut line = String::new();
        if engine.stdout.read_line(&mut line)? == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "引擎进程已退出",
            ));
        }
        Ok(line)
    })()
    .map_err(|error| format!("引擎健康检查失败: {error}"))?;
    let health: Value = serde_json::from_str(&line)
        .map_err(|error| format!("引擎健康检查返回无效 JSON: {error}"))?;
    let protocol = health.get("protocol").and_then(Value::as_u64);
    if health.get("ok").and_then(Value::as_bool) != Some(true) || protocol != Some(ENGINE_PROTOCOL)
    {
        return Err(format!(
            "引擎协议不兼容: 需要 {ENGINE_PROTOCOL}，实际 {protocol:?}"
        ));
    }
    Ok(())
}

pub(crate) fn engine_request_blocking(
    resource_dir: &Path,
    request: &str,
) -> Result<String, String> {
    let slot = ENGINE.get_or_init(|| Mutex::new(None));
    let mut guard = slot.lock().map_err(|_| "引擎状态锁损坏".to_owned())?;
    let mut last_error = String::new();
    for _attempt in 0..2 {
        if guard.is_none() {
            *guard = Some(spawn_engine(resource_dir)?);
        }
        let engine = guard.as_mut().expect("engine just ensured");
        let exchange = (|| -> std::io::Result<String> {
            engine.stdin.write_all(request.as_bytes())?;
            engine.stdin.write_all(b"\n")?;
            engine.stdin.flush()?;
            let mut line = String::new();
            if engine.stdout.read_line(&mut line)? == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "引擎进程已退出",
                ));
            }
            Ok(line)
        })();
        match exchange {
            Ok(line) => return Ok(line.trim_end().to_owned()),
            Err(error) => {
                last_error = error.to_string();
                *guard = None; // Drop 会回收进程,下一轮重启
            }
        }
    }
    Err(format!("引擎通信失败: {last_error}"))
}

/// 引擎仓库根目录:优先 FERRY_REPO 环境变量,
/// 否则取本 crate 上两级(app/src-tauri → 仓库根,开发形态)。
#[cfg(debug_assertions)]
fn repo_root() -> PathBuf {
    if let Ok(p) = std::env::var("FERRY_REPO") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from("."))
}

/// externalBin 在 macOS 上被放进 Contents/MacOS(主程序旁),Windows 上在安装根目录;
/// 依次尝试可执行文件所在目录与 resource_dir,取第一个存在的。
fn bundled_engine_candidates(resource_dir: &Path) -> Vec<PathBuf> {
    let name = bundled_engine_name(cfg!(target_os = "windows"));
    let mut candidates = Vec::new();
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    {
        candidates.push(exe_dir.join(name));
    }
    candidates.push(resource_dir.join(name));
    candidates
}

fn bundled_engine_name(is_windows: bool) -> &'static str {
    if is_windows {
        "ferry-engine.exe"
    } else {
        "ferry-engine"
    }
}

fn engine_command(resource_dir: &Path) -> Result<Command, String> {
    let candidates = bundled_engine_candidates(resource_dir);
    if let Some(sidecar) = candidates.iter().find(|path| path.is_file()) {
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
    Err(format!(
        "正式包缺少引擎 sidecar,已尝试: {}",
        candidates
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("; ")
    ))
}

#[cfg(target_os = "windows")]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x0800_0000);
}

#[cfg(not(target_os = "windows"))]
fn hide_console(_command: &mut Command) {}

/// 应用启动即预热常驻引擎:PyInstaller 解压与 webview 启动并行,
/// 首个前端 RPC 到达时引擎大概率已就绪。失败静默,错误会在首个真实 RPC 上重现。
pub(crate) fn warm_up(resource_dir: PathBuf) {
    std::thread::spawn(move || {
        let _ = engine_request_blocking(&resource_dir, r#"{"method":"health"}"#);
    });
}

#[tauri::command]
pub(crate) async fn engine_rpc(app: tauri::AppHandle, request: String) -> Result<String, String> {
    use tauri::Manager;
    let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || engine_request_blocking(&resource_dir, &request))
        .await
        .map_err(|e| e.to_string())?
}
