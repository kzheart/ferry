use std::path::Path;
use std::{
    fs,
    path::PathBuf,
    process::Command,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use super::{TerminalLaunch, TerminalPreference};

pub(super) fn reveal_path(path: &Path) -> Result<(), String> {
    // 目录:直接打开(进入项目);文件:在父目录中选中(open -R)。
    let mut command = Command::new("open");
    if path.is_file() {
        command.arg("-R");
    }
    let output = command
        .arg(path)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    Ok(())
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn terminal_action(launch: &TerminalLaunch) -> String {
    launch
        .args
        .iter()
        .fold(launch.executable.clone(), |acc, arg| {
            format!("{acc} {}", shell_quote(arg))
        })
}

fn terminal_command(launch: &TerminalLaunch) -> String {
    let cwd = launch.cwd.as_deref().unwrap_or(".");
    format!("cd {} && {}", shell_quote(cwd), terminal_action(launch))
}

fn applescript_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn run_applescript(script: String) -> Result<(), String> {
    let out = Command::new("osascript")
        .args(["-e", &script])
        .output()
        .map_err(|error| error.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    Ok(())
}

fn open_system_terminal(command: &str) -> Result<(), String> {
    let script = format!(
        "tell application \"Terminal\"\nactivate\ndo script \"{}\"\nend tell",
        applescript_escape(command)
    );
    run_applescript(script)
}

fn open_iterm(command: &str) -> Result<(), String> {
    let script = format!(
        "tell application \"iTerm2\"\nactivate\ncreate window with default profile command \"{}\"\nend tell",
        applescript_escape(command)
    );
    run_applescript(script)
}

fn toml_basic_string(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\r', "\\r")
        .replace('\n', "\\n")
}

fn warp_tab_config_path() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or_else(|| "无法定位用户主目录".to_string())?;
    let dir = PathBuf::from(home).join(".warp").join("tab_configs");
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    Ok(dir.join(format!("ferry_resume_{stamp}.toml")))
}

fn warp_cwd(launch: &TerminalLaunch) -> Result<String, String> {
    let cwd = launch.cwd.as_deref().unwrap_or(".");
    let path = PathBuf::from(cwd);
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map_err(|error| error.to_string())?
            .join(path)
    };
    Ok(absolute.to_string_lossy().into_owned())
}

fn open_warp(launch: &TerminalLaunch) -> Result<(), String> {
    let path = warp_tab_config_path()?;
    let content = format!(
        "name = \"Ferry resume\"\ntitle = \"Ferry\"\n\n[[panes]]\nid = \"resume\"\ntype = \"terminal\"\ndirectory = \"{}\"\ncommands = [\"{}\"]\nis_focused = true\n",
        toml_basic_string(&warp_cwd(launch)?),
        toml_basic_string(&terminal_action(launch)),
    );
    fs::write(&path, content).map_err(|error| error.to_string())?;
    let name = path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "Warp Tab Config 文件名无效".to_string())?;
    let uri = format!("warp://tab_config/{name}?new_window=true");
    let status = Command::new("open")
        .arg(&uri)
        .status()
        .map_err(|error| error.to_string())?;
    if !status.success() {
        let _ = fs::remove_file(&path);
        return Err("无法启动 Warp".to_string());
    }
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(30));
        let _ = fs::remove_file(path);
    });
    Ok(())
}

pub(super) fn home_dir() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "无法定位用户主目录".to_string())
}

pub(super) fn cli_link_path() -> Result<PathBuf, String> {
    Ok(home_dir()?.join(".local").join("bin").join("ferry"))
}

pub(super) fn create_cli_link(link: &Path, target: &Path) -> Result<(), String> {
    if let Some(parent) = link.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建 {} 失败: {error}", parent.display()))?;
    }
    // symlink(2) 不覆盖已有项,更新安装必须先摘掉旧入口。用 symlink_metadata 判存在,
    // 否则指向已删除二进制的断链会被 exists() 判成不存在,remove 又照样失败。
    if fs::symlink_metadata(link).is_ok() {
        fs::remove_file(link).map_err(|error| format!("移除旧的 CLI 入口失败: {error}"))?;
    }
    std::os::unix::fs::symlink(target, link).map_err(|error| format!("创建 CLI 入口失败: {error}"))
}

pub(super) fn process_alive(pid: u32) -> bool {
    // signal 0 只做权限与存在性检查,不投递信号。
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

pub(super) fn open_terminal(
    launch: &TerminalLaunch,
    preference: TerminalPreference,
) -> Result<(), String> {
    let command = terminal_command(launch);
    match preference {
        TerminalPreference::Terminal => open_system_terminal(&command),
        TerminalPreference::Iterm => {
            open_iterm(&command).or_else(|_| open_system_terminal(&command))
        }
        TerminalPreference::Warp => open_warp(launch).or_else(|_| open_system_terminal(&command)),
        TerminalPreference::Auto => open_warp(launch)
            .or_else(|_| open_iterm(&command))
            .or_else(|_| open_system_terminal(&command)),
    }
}
