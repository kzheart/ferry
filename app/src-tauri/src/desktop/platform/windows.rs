use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ACCESS_DENIED, FALSE, INVALID_HANDLE_VALUE, STILL_ACTIVE,
};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

use super::{TerminalLaunch, TerminalPreference};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub(super) fn reveal_path(path: &Path) -> Result<(), String> {
    let target = explorer_path(path);
    let mut command = Command::new("explorer.exe");
    if path.is_file() {
        command.arg(format!("/select,{}", target.display()));
    } else {
        command.arg(target.as_os_str());
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法打开资源管理器: {error}"))
}

fn explorer_path(path: &Path) -> PathBuf {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    strip_verbatim(&canonical)
}

/// cmd.exe 跑不了 `\\?\D:\...` 这种 NT 前缀，垫片和资源管理器路径都要剥掉。
fn strip_verbatim(path: &Path) -> PathBuf {
    let raw = path.to_string_lossy();
    let stripped = raw
        .strip_prefix(r"\\?\UNC\")
        .map(|rest| format!(r"\\{rest}"))
        .or_else(|| raw.strip_prefix(r"\\?\").map(str::to_owned))
        .unwrap_or_else(|| raw.into_owned());
    PathBuf::from(stripped)
}

pub(super) fn home_dir() -> Result<PathBuf, String> {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .ok_or_else(|| "无法定位用户主目录".to_owned())
}

/// CLI 安装点：`%LOCALAPPDATA%\Ferry\bin\ferry.cmd`，并写入用户 PATH。
/// 用 cmd 垫片而不是文件符号链接，这样不需要管理员或开发人员模式。
pub(super) fn cli_link_path() -> Result<PathBuf, String> {
    let local = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| "无法定位 LOCALAPPDATA".to_owned())?;
    Ok(local.join("Ferry").join("bin").join("ferry.cmd"))
}

pub(super) fn create_cli_link(link: &Path, target: &Path) -> Result<(), String> {
    if let Some(parent) = link.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建 {} 失败: {error}", parent.display()))?;
        let stale_exe = parent.join("ferry.exe");
        if fs::symlink_metadata(&stale_exe).is_ok() {
            let _ = fs::remove_file(&stale_exe);
        }
    }
    if fs::symlink_metadata(link).is_ok() {
        fs::remove_file(link).map_err(|error| format!("移除旧的 CLI 入口失败: {error}"))?;
    }
    let target = explorer_path(target);
    let body = format!("@echo off\r\n\"{}\" %*\r\n", target.display());
    fs::write(link, body).map_err(|error| format!("创建 CLI 入口失败: {error}"))?;
    if let Some(parent) = link.parent() {
        if parent.ends_with(Path::new("Ferry").join("bin")) {
            if let Err(error) = prepend_user_path(parent) {
                let _ = fs::remove_file(link);
                return Err(error);
            }
        }
    }
    Ok(())
}

pub(super) fn remove_cli_link(link: &Path) -> Result<(), String> {
    if let Some(parent) = link.parent() {
        if parent.ends_with(Path::new("Ferry").join("bin")) {
            remove_user_path(parent)?;
        }
    }
    fs::remove_file(link).map_err(|error| format!("移除 CLI 入口失败: {error}"))
}

pub(super) fn resolve_cli_link(link: &Path) -> Option<PathBuf> {
    let raw = if let Ok(target) = fs::read_link(link) {
        target
    } else {
        parse_cmd_shim(&fs::read_to_string(link).ok()?)?
    };
    Some(strip_verbatim(&raw))
}

/// 垫片还带着 `\\?\` 前缀时需要重写，否则 `same_target` 会判已同步、cmd 却跑不起来。
pub(super) fn cli_link_needs_rewrite(link: &Path) -> bool {
    fs::read_to_string(link)
        .map(|text| text.contains(r"\\?\"))
        .unwrap_or(false)
}

fn parse_cmd_shim(text: &str) -> Option<PathBuf> {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.eq_ignore_ascii_case("@echo off") {
            continue;
        }
        let rest = line.strip_prefix('"')?;
        let (path, _) = rest.split_once('"')?;
        if !path.is_empty() {
            return Some(PathBuf::from(path));
        }
    }
    None
}

fn prepend_user_path(dir: &Path) -> Result<(), String> {
    let dir_str = dir.to_string_lossy().replace('\'', "''");
    let script = format!(
        "$d = '{dir_str}'; $p = [Environment]::GetEnvironmentVariable('Path','User'); if ($null -eq $p) {{ $p = '' }}; $parts = @($p -split ';' | Where-Object {{ $_ -ne '' }}); if ($parts -notcontains $d) {{ [Environment]::SetEnvironmentVariable('Path', $(if ($p) {{ $d + ';' + $p }} else {{ $d }}), 'User') }}"
    );
    let status = Command::new("powershell")
        .creation_flags(CREATE_NO_WINDOW)
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .status()
        .map_err(|error| error.to_string())?;
    if !status.success() {
        return Err("无法把 Ferry\\bin 写入用户 PATH".to_owned());
    }
    Ok(())
}

fn remove_user_path(dir: &Path) -> Result<(), String> {
    let dir_str = dir.to_string_lossy().replace('\'', "''");
    let script = format!(
        "$d = '{dir_str}'; $p = [Environment]::GetEnvironmentVariable('Path','User'); if ($null -eq $p) {{ exit 0 }}; $parts = @($p -split ';' | Where-Object {{ $_ -and $_ -ine $d }}); [Environment]::SetEnvironmentVariable('Path', ($parts -join ';'), 'User')"
    );
    let status = Command::new("powershell")
        .creation_flags(CREATE_NO_WINDOW)
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .status()
        .map_err(|error| error.to_string())?;
    if !status.success() {
        return Err("无法从用户 PATH 移除 Ferry\\bin".to_owned());
    }
    Ok(())
}

pub(super) fn create_directory_link(link: &Path, target: &Path) -> Result<(), String> {
    if let Some(parent) = link.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建 {} 失败: {error}", parent.display()))?;
    }
    let target = explorer_path(
        &fs::canonicalize(target)
            .map_err(|error| format!("无法解析 skill 真身 {}: {error}", target.display()))?,
    );
    // junction 不需要开发人员模式；目录符号链接在没开该模式时会失败。
    create_junction(link, &target).or_else(|_| {
        std::os::windows::fs::symlink_dir(&target, link)
            .map_err(|error| format!("创建目录链接 {} 失败: {error}", link.display()))
    })
}

pub(super) fn remove_directory_link(link: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(link)
        .map_err(|error| format!("检查 {} 失败: {error}", link.display()))?;
    // 目录符号链接必须用 RemoveDirectory（remove_dir），DeleteFile 会 ACCESS_DENIED。
    // junction 不是 is_symlink()，同样走 remove_dir，禁止 remove_dir_all（会顺着真身删）。
    let result = if meta.is_file() {
        fs::remove_file(link)
    } else {
        fs::remove_dir(link).or_else(|_| remove_reparse_via_cmd(link))
    };
    result.map_err(|error| format!("移除目录链接 {} 失败: {error}", link.display()))
}

fn remove_reparse_via_cmd(link: &Path) -> std::io::Result<()> {
    let cmdline = format!("rmdir {}", cmd_quote(&link.to_string_lossy()));
    let output = Command::new("cmd")
        .creation_flags(CREATE_NO_WINDOW)
        .args(["/C", &cmdline])
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        let detail = String::from_utf8_lossy(&output.stderr);
        Err(std::io::Error::other(detail.trim().to_owned()))
    }
}

fn create_junction(link: &Path, target: &Path) -> Result<(), String> {
    // cmd /C 只吃后面那一整段；分参数传会被当成「/C mklink」而丢掉 /J。
    // mklink /J 不需要管理员或开发人员模式。
    let cmdline = format!(
        "mklink /J {} {}",
        cmd_quote(&link.to_string_lossy()),
        cmd_quote(&target.to_string_lossy())
    );
    let output = Command::new("cmd")
        .creation_flags(CREATE_NO_WINDOW)
        .args(["/C", &cmdline])
        .output()
        .map_err(|error| format!("创建目录链接失败: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    create_junction_powershell(link, target)
}

fn create_junction_powershell(link: &Path, target: &Path) -> Result<(), String> {
    let output = Command::new("powershell")
        .creation_flags(CREATE_NO_WINDOW)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            // PS 5.1 的 Junction 参数集只有 Path/Value，没有 LiteralPath/Target。
            "New-Item -ItemType Junction -Path $env:FERRY_LINK -Value $env:FERRY_TARGET | Out-Null",
        ])
        .env("FERRY_LINK", link.as_os_str())
        .env("FERRY_TARGET", target.as_os_str())
        .output()
        .map_err(|error| format!("创建目录链接失败: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(&output.stderr);
    let detail = if detail.trim().is_empty() {
        String::from_utf8_lossy(&output.stdout).into_owned()
    } else {
        detail.into_owned()
    };
    Err(format!(
        "创建目录链接 {} 失败: {}",
        link.display(),
        detail.trim()
    ))
}

pub(super) fn process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid);
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return GetLastError() == ERROR_ACCESS_DENIED;
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(handle, &mut code);
        CloseHandle(handle);
        ok != FALSE && code == STILL_ACTIVE as u32
    }
}

fn cmd_quote(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".to_owned();
    }
    if !value.contains([' ', '\t', '"', '&', '|', '<', '>', '^', '%']) {
        return value.to_owned();
    }
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn launch_line(launch: &TerminalLaunch) -> String {
    let mut parts = vec![cmd_quote(&launch.executable)];
    parts.extend(launch.args.iter().map(|arg| cmd_quote(arg)));
    parts.join(" ")
}

fn open_windows_terminal(launch: &TerminalLaunch) -> Result<(), String> {
    let cwd = launch.cwd.as_deref().unwrap_or(".");
    let mut command = Command::new("wt.exe");
    command.arg("-d").arg(cwd).arg("--").arg(&launch.executable);
    command.args(&launch.args);
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法启动 Windows Terminal: {error}"))
}

fn open_cmd(launch: &TerminalLaunch) -> Result<(), String> {
    let cwd = launch.cwd.as_deref().unwrap_or(".");
    let inner = format!("cd /d {} && {}", cmd_quote(cwd), launch_line(launch));
    Command::new("cmd.exe")
        .args(["/C", "start", "Ferry", "cmd.exe", "/K", &inner])
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法启动命令提示符: {error}"))
}

fn open_warp(launch: &TerminalLaunch) -> Result<(), String> {
    let cwd = launch.cwd.as_deref().unwrap_or(".");
    let mut command = Command::new("warp.exe");
    command.arg("--working-directory").arg(cwd);
    command.arg(&launch.executable);
    command.args(&launch.args);
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法启动 Warp: {error}"))
}

pub(super) fn open_terminal(
    launch: &TerminalLaunch,
    preference: TerminalPreference,
) -> Result<(), String> {
    match preference {
        TerminalPreference::Warp => open_warp(launch).or_else(|_| open_cmd(launch)),
        TerminalPreference::Terminal | TerminalPreference::Iterm => {
            open_windows_terminal(launch).or_else(|_| open_cmd(launch))
        }
        TerminalPreference::Auto => open_windows_terminal(launch)
            .or_else(|_| open_warp(launch))
            .or_else(|_| open_cmd(launch)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmd_shim_round_trips_the_engine_path() {
        let dir = tempfile_dir("cli");
        let engine = dir.join("ferry-engine.exe");
        std::fs::write(&engine, b"fake").unwrap();
        let link = dir.join("bin").join("ferry.cmd");
        create_cli_link(&link, &engine).expect("写垫片");
        let body = std::fs::read_to_string(&link).unwrap();
        assert!(!body.contains(r"\\?\"), "{body}");
        assert!(!cli_link_needs_rewrite(&link));
        let resolved = resolve_cli_link(&link).expect("读垫片");
        assert_eq!(explorer_path(&engine), resolved);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn directory_junction_points_at_the_real_skill() {
        let dir = tempfile_dir("skill");
        let source = dir.join("shared").join("ferry");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("SKILL.md"), "ok").unwrap();
        let link = dir.join("claude").join("ferry");
        create_directory_link(&link, &source).expect("建 junction");
        assert!(link.join("SKILL.md").is_file());
        assert_eq!(
            explorer_path(&source.canonicalize().unwrap()),
            explorer_path(&link.canonicalize().unwrap())
        );
        remove_directory_link(&link).expect("摘入口");
        assert!(source.join("SKILL.md").is_file());
        assert!(!link.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn tempfile_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "ferry-win-{}-{}-{nanos}",
            label,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }
}
