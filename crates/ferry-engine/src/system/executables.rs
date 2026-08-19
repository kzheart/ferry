//! 跨平台 CLI 定位：PATH（`shutil.which` 等价物）优先，常见安装目录兜底。
//!
//! macOS 上 GUI 启动的进程只继承 launchd 最小 PATH，Tauri 层已用 fix-path-env
//! 恢复登录 shell PATH；此处兜底覆盖 shell 配置异常与非标准安装位置。
//! Windows 上 npm 装的 CLI 是 `.cmd` 垫片，CreateProcess 对裸命令名不查 PATHEXT，
//! 必须先解析出完整路径再执行。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use super::paths::{expanduser, home_dir};

/// adapter 声明的兜底目录：executable -> 目录列表。
static TOOL_FALLBACK_DIRS: LazyLock<Mutex<HashMap<String, Vec<PathBuf>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// `resolve` 的记忆化缓存（等价 Python 的 `lru_cache(maxsize=None)`）。
static RESOLVE_CACHE: LazyLock<Mutex<HashMap<String, Option<String>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn fallback_dirs() -> Vec<PathBuf> {
    let home = home_dir();
    let mut dirs = vec![
        home.join(".local").join("bin"),
        home.join(".npm-global").join("bin"),
        home.join(".bun").join("bin"),
        home.join(".volta").join("bin"),
    ];
    if cfg!(windows) {
        if let Ok(appdata) = std::env::var("APPDATA") {
            if !appdata.is_empty() {
                dirs.push(PathBuf::from(appdata).join("npm"));
            }
        }
    } else {
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
        dirs.push(PathBuf::from("/usr/local/bin"));
        let nvm = home.join(".nvm").join("versions").join("node");
        if nvm.is_dir() {
            let mut versions: Vec<PathBuf> = std::fs::read_dir(&nvm)
                .into_iter()
                .flatten()
                .flatten()
                .map(|entry| entry.path().join("bin"))
                .collect();
            // Python 侧是 `sorted(..., reverse=True)`：新版本优先。
            versions.sort();
            versions.reverse();
            dirs.extend(versions);
        }
    }
    dirs
}

/// Register adapter-declared binary locations without naming an adapter here.
pub fn register_fallback_dirs(executables: &[&str], directories: &[&str]) {
    let paths: Vec<PathBuf> = directories
        .iter()
        .map(|directory| expanduser(directory))
        .collect();
    {
        let mut registry = TOOL_FALLBACK_DIRS.lock().expect("兜底目录表锁中毒");
        for executable in executables {
            registry.insert((*executable).to_string(), paths.clone());
        }
    }
    clear_resolve_cache();
}

/// 清空 `resolve` 缓存（对齐 `resolve.cache_clear()`）。
pub fn clear_resolve_cache() {
    RESOLVE_CACHE.lock().expect("解析缓存锁中毒").clear();
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// Windows 下要按 PATHEXT 逐个后缀尝试（npm 垫片是 `.cmd`）。
fn candidate_names(tool: &str) -> Vec<String> {
    if !cfg!(windows) {
        return vec![tool.to_string()];
    }
    let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
    let mut names = vec![tool.to_string()];
    let has_extension = Path::new(tool)
        .extension()
        .is_some_and(|extension| !extension.is_empty());
    if !has_extension {
        names.extend(
            pathext
                .split(';')
                .filter(|extension| !extension.is_empty())
                .map(|extension| format!("{tool}{extension}")),
        );
    }
    names
}

/// 在给定目录里查找可执行文件（等价 `shutil.which(tool, path=directory)`）。
pub fn which_in(tool: &str, directory: &Path) -> Option<String> {
    for name in candidate_names(tool) {
        let candidate = directory.join(&name);
        if is_executable(&candidate) {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }
    None
}

/// 在进程 PATH 上查找（等价 `shutil.which(tool)`）。
pub fn which(tool: &str) -> Option<String> {
    if tool.contains(std::path::MAIN_SEPARATOR) || tool.contains('/') {
        let candidate = PathBuf::from(tool);
        return is_executable(&candidate).then(|| tool.to_string());
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|directory| !directory.as_os_str().is_empty())
        .find_map(|directory| which_in(tool, &directory))
}

/// 解析 CLI 绝对路径；PATH 未命中时扫描常见安装目录。找不到返回 `None`。
///
/// 命中兜底目录时会把该目录**前插进本进程 PATH**——CLI 可能是 node 等运行时的
/// 垫片（`#!/usr/bin/env node`），同目录通常就有该运行时，前插后本进程与后续
/// 子进程都能找到它。这个副作用是 Python 侧的既有行为，必须保留。
pub fn resolve(tool: &str) -> Option<String> {
    if let Some(cached) = RESOLVE_CACHE.lock().expect("解析缓存锁中毒").get(tool) {
        return cached.clone();
    }
    let resolved = resolve_uncached(tool);
    RESOLVE_CACHE
        .lock()
        .expect("解析缓存锁中毒")
        .insert(tool.to_string(), resolved.clone());
    resolved
}

fn resolve_uncached(tool: &str) -> Option<String> {
    if let Some(found) = which(tool) {
        return Some(found);
    }
    let registered = TOOL_FALLBACK_DIRS
        .lock()
        .expect("兜底目录表锁中毒")
        .get(tool)
        .cloned()
        .unwrap_or_default();
    for directory in registered.into_iter().chain(fallback_dirs()) {
        if let Some(found) = which_in(tool, &directory) {
            prepend_to_path(&directory);
            return Some(found);
        }
    }
    None
}

fn prepend_to_path(directory: &Path) {
    let current = std::env::var_os("PATH").unwrap_or_default();
    let mut entries = vec![directory.to_path_buf()];
    entries.extend(std::env::split_paths(&current));
    if let Ok(joined) = std::env::join_paths(entries) {
        // 进程级副作用，与 Python 侧 `os.environ["PATH"] = ...` 等价。
        unsafe { std::env::set_var("PATH", joined) };
    }
}

/// 构造子进程命令行；未解析到时保留裸名，报错语义与原先一致。
pub fn argv(tool: &str, args: &[&str]) -> Vec<String> {
    let mut command = vec![resolve(tool).unwrap_or_else(|| tool.to_string())];
    command.extend(args.iter().map(|arg| (*arg).to_string()));
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_keeps_the_bare_name_when_unresolved() {
        let command = argv("ferry-definitely-not-installed", &["--version"]);
        assert_eq!(command, ["ferry-definitely-not-installed", "--version"]);
    }

    #[cfg(unix)]
    #[test]
    fn which_in_only_accepts_executable_files() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let plain = temp.path().join("plain");
        std::fs::write(&plain, "x").unwrap();
        assert_eq!(which_in("plain", temp.path()), None);
        std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            which_in("plain", temp.path()),
            Some(plain.to_string_lossy().into_owned())
        );
    }
}
