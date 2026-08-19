//! Cross-platform locations for external session stores.
//!
//! 语义事实源：`engine/system/paths.py`。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf, MAIN_SEPARATOR};

/// 环境变量视图；测试可以注入而不污染进程环境（对齐 Python 的 `environ` 形参）。
pub type Environ = BTreeMap<String, String>;

/// 采集当前进程环境。
pub fn process_environ() -> Environ {
    std::env::vars().collect()
}

/// `os.name` 的两个取值。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OsFamily {
    /// `os.name == "nt"`
    Windows,
    /// `os.name == "posix"`
    Posix,
}

impl OsFamily {
    pub fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else {
            Self::Posix
        }
    }
}

/// 等价 `Path.home()`：优先环境变量，回落到平台约定。
pub fn home_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Ok(profile) = std::env::var("USERPROFILE") {
            if !profile.is_empty() {
                return PathBuf::from(profile);
            }
        }
        match (std::env::var("HOMEDRIVE"), std::env::var("HOMEPATH")) {
            (Ok(drive), Ok(path)) if !drive.is_empty() => {
                return PathBuf::from(format!("{drive}{path}"))
            }
            _ => {}
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(home) = std::env::var("HOME") {
            if !home.is_empty() {
                return PathBuf::from(home);
            }
        }
    }
    PathBuf::from("/")
}

/// 等价 `Path.expanduser()`：只展开前导 `~` 与 `~/`（`~user` 不支持，保持原样）。
pub fn expanduser(path: &str) -> PathBuf {
    if path == "~" {
        return home_dir();
    }
    let separators: &[char] = &['/', MAIN_SEPARATOR];
    if let Some(rest) = path.strip_prefix('~') {
        if rest.starts_with(separators) {
            return home_dir().join(rest.trim_start_matches(separators));
        }
    }
    PathBuf::from(path)
}

/// 等价 `Path(path).is_relative_to(root)`，按已规范化的路径串比较。
///
/// 注意是**前缀语义**：调用方必须先 realpath，否则 `..` 能穿透。
pub fn is_within(path: &str, root: &str) -> bool {
    if path == root {
        return true;
    }
    let prefix = if root.ends_with(MAIN_SEPARATOR) {
        root.to_string()
    } else {
        format!("{root}{MAIN_SEPARATOR}")
    };
    path.starts_with(&prefix)
}

/// 等价 `os.path.realpath(path, strict=True)`：目标不存在即失败。
pub fn realpath_strict(path: &Path) -> std::io::Result<PathBuf> {
    std::fs::canonicalize(path)
}

/// 先 `expanduser` 再 `realpath(strict=True)`，是 adapter 引用校验的标准入口。
pub fn resolved_path(raw: &str) -> Option<PathBuf> {
    realpath_strict(&expanduser(raw)).ok()
}

fn env_value<'a>(environ: &'a Environ, key: &str) -> Option<&'a str> {
    environ
        .get(key)
        .map(String::as_str)
        .filter(|value| !value.is_empty())
}

/// opencode 的只读会话库位置。
pub fn opencode_database_path(platform: OsFamily, environ: &Environ, home: &Path) -> PathBuf {
    if let Some(override_path) = env_value(environ, "FERRY_OPENCODE_DB") {
        return expanduser(override_path);
    }
    let data_home = if platform == OsFamily::Windows {
        env_value(environ, "LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData").join("Local"))
    } else {
        env_value(environ, "XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local").join("share"))
    };
    data_home.join("opencode").join("opencode.db")
}

/// Cursor 的只读会话库位置（`state.vscdb`）。
///
/// Cursor 是 VS Code 分支，globalStorage 的落位随桌面平台不同；[`OsFamily`] 只
/// 区分 nt / posix，分不开 macOS 与 Linux，所以 posix 下按 macOS、Linux 两种
/// 布局依次探测，取第一个存在的；都不存在时按当前编译目标给默认值，让
/// `session.store_unavailable` 的提示指向本平台的正确位置。
pub fn cursor_database_path(platform: OsFamily, environ: &Environ, home: &Path) -> PathBuf {
    if let Some(override_path) = env_value(environ, "FERRY_CURSOR_DB") {
        return expanduser(override_path);
    }
    let leaf = |base: PathBuf| {
        base.join("Cursor")
            .join("User")
            .join("globalStorage")
            .join("state.vscdb")
    };
    if platform == OsFamily::Windows {
        let roaming = env_value(environ, "APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData").join("Roaming"));
        return leaf(roaming);
    }
    let macos = leaf(home.join("Library").join("Application Support"));
    let linux = leaf(
        env_value(environ, "XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config")),
    );
    for candidate in [&macos, &linux] {
        if candidate.exists() {
            return candidate.clone();
        }
    }
    if cfg!(target_os = "macos") {
        macos
    } else {
        linux
    }
}

/// Return Pi session roots in runtime lookup order.
///
/// Pi 的显式 session 目录优先；否则读 `settings.json` 里的 `sessionDir`，
/// 再否则回落到默认的 project-bucket 根，这样扫描不必反推 Pi 的编码目录名。
pub fn pi_session_roots(environ: &Environ, home: &Path) -> Vec<PathBuf> {
    if let Some(explicit) = env_value(environ, "PI_CODING_AGENT_SESSION_DIR") {
        return vec![expanduser(explicit)];
    }
    let agent_dir = match env_value(environ, "PI_CODING_AGENT_DIR") {
        Some(raw) => expanduser(raw),
        None => home.join(".pi").join("agent"),
    };
    let configured = std::fs::read_to_string(agent_dir.join("settings.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|settings| {
            settings
                .get("sessionDir")
                .and_then(|value| value.as_str())
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string)
        });
    match configured {
        Some(session_dir) => vec![expanduser(&session_dir)],
        None => vec![agent_dir.join("sessions")],
    }
}

/// Grok 的数据根目录。
pub fn grok_home(environ: &Environ, home: &Path) -> PathBuf {
    match env_value(environ, "GROK_HOME") {
        Some(raw) => expanduser(raw),
        None => home.join(".grok"),
    }
}

/// 单测辅助：**crate 级**的进程环境互斥。
///
/// `HOME` / `PATH` / `GROK_HOME` / `PI_CODING_AGENT_SESSION_DIR` /
/// `FERRY_CURSOR_DB` / `FERRY_DATA_DIR` / `FERRY_BACKUP_DIR` 都是进程级状态，
/// 而 lib 测试默认多线程
/// 跑在**同一个进程**里。各模块各自持一把锁只能挡住自己模块内的并发：claude
/// 的用例把 `HOME` 指向沙箱时，grok / opencode / snapshots 里任何一个读
/// `home_dir()` 的用例都会读到那个沙箱。所以改写进程环境的测试必须共用**这一把**
/// 锁，不要在别处再造。
#[cfg(test)]
pub(crate) mod testing {
    use std::ffi::OsStr;
    use std::sync::{Mutex, MutexGuard};

    static ENVIRONMENT: Mutex<()> = Mutex::new(());

    /// 作用域内独占进程环境；析构时把改过的变量逐个恢复原值。
    #[must_use = "守卫析构即释放环境锁"]
    pub(crate) struct EnvGuard {
        _lock: MutexGuard<'static, ()>,
        restore: Vec<(String, Option<String>)>,
    }

    impl EnvGuard {
        /// 只取锁、不改任何变量（给「读环境」的用例用）。
        pub(crate) fn acquire() -> Self {
            Self {
                _lock: ENVIRONMENT
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()),
                restore: Vec::new(),
            }
        }

        pub(crate) fn set(mut self, key: &str, value: impl AsRef<OsStr>) -> Self {
            self.restore
                .push((key.to_string(), std::env::var(key).ok()));
            // SAFETY: 全 crate 改写进程环境的测试都持有 ENVIRONMENT 锁。
            unsafe { std::env::set_var(key, value) };
            self
        }

        /// 最常见的用法：把 `HOME` 指向沙箱。
        pub(crate) fn home(path: impl AsRef<OsStr>) -> Self {
            Self::acquire().set("HOME", path)
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // 逆序恢复：同一个 key 被设置多次时最终回到最初的值。
            for (key, previous) in self.restore.drain(..).rev() {
                // SAFETY: 见 set。
                unsafe {
                    match previous {
                        Some(value) => std::env::set_var(&key, value),
                        None => std::env::remove_var(&key),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environ(pairs: &[(&str, &str)]) -> Environ {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    #[test]
    fn is_within_uses_prefix_semantics_with_a_separator_guard() {
        let root = format!("{MAIN_SEPARATOR}a{MAIN_SEPARATOR}b");
        assert!(is_within(&root, &root));
        assert!(is_within(&format!("{root}{MAIN_SEPARATOR}c"), &root));
        // 同名前缀不算包含关系。
        assert!(!is_within(&format!("{root}c"), &root));
        // root 自带尾分隔符时不重复追加。
        assert!(is_within(
            &format!("{root}{MAIN_SEPARATOR}c"),
            &format!("{root}{MAIN_SEPARATOR}")
        ));
    }

    #[test]
    fn opencode_path_prefers_the_explicit_override() {
        let home = PathBuf::from("/home/u");
        assert_eq!(
            opencode_database_path(
                OsFamily::Posix,
                &environ(&[("FERRY_OPENCODE_DB", "/custom/db.sqlite")]),
                &home
            ),
            PathBuf::from("/custom/db.sqlite")
        );
        assert_eq!(
            opencode_database_path(OsFamily::Posix, &environ(&[]), &home),
            PathBuf::from("/home/u/.local/share/opencode/opencode.db")
        );
        assert_eq!(
            opencode_database_path(
                OsFamily::Posix,
                &environ(&[("XDG_DATA_HOME", "/xdg")]),
                &home
            ),
            PathBuf::from("/xdg/opencode/opencode.db")
        );
        assert_eq!(
            opencode_database_path(
                OsFamily::Windows,
                &environ(&[("LOCALAPPDATA", "C:\\Local")]),
                &home
            ),
            PathBuf::from("C:\\Local")
                .join("opencode")
                .join("opencode.db")
        );
    }

    #[test]
    fn cursor_path_probes_both_posix_layouts() {
        let home = PathBuf::from("/home/u");
        assert_eq!(
            cursor_database_path(
                OsFamily::Posix,
                &environ(&[("FERRY_CURSOR_DB", "/custom/state.vscdb")]),
                &home
            ),
            PathBuf::from("/custom/state.vscdb")
        );
        // 两种 posix 布局都不存在时按编译目标给默认值。
        let expected = if cfg!(target_os = "macos") {
            "/home/u/Library/Application Support/Cursor/User/globalStorage/state.vscdb"
        } else {
            "/home/u/.config/Cursor/User/globalStorage/state.vscdb"
        };
        assert_eq!(
            cursor_database_path(OsFamily::Posix, &environ(&[]), &home),
            PathBuf::from(expected)
        );
        assert_eq!(
            cursor_database_path(
                OsFamily::Windows,
                &environ(&[("APPDATA", "C:\\Roaming")]),
                &home
            ),
            PathBuf::from("C:\\Roaming")
                .join("Cursor")
                .join("User")
                .join("globalStorage")
                .join("state.vscdb")
        );
        // 真实存在的 macOS 布局优先于 XDG 回落。
        let root = tempfile::tempdir().unwrap();
        let macos = root
            .path()
            .join("Library/Application Support/Cursor/User/globalStorage");
        std::fs::create_dir_all(&macos).unwrap();
        std::fs::write(macos.join("state.vscdb"), b"").unwrap();
        assert_eq!(
            cursor_database_path(OsFamily::Posix, &environ(&[]), root.path()),
            macos.join("state.vscdb")
        );
    }

    #[test]
    fn pi_roots_follow_the_documented_lookup_order() {
        let home = PathBuf::from("/home/u");
        assert_eq!(
            pi_session_roots(
                &environ(&[("PI_CODING_AGENT_SESSION_DIR", "/explicit")]),
                &home
            ),
            vec![PathBuf::from("/explicit")]
        );
        assert_eq!(
            pi_session_roots(&environ(&[]), &home),
            vec![PathBuf::from("/home/u/.pi/agent/sessions")]
        );
    }

    #[test]
    fn grok_home_honours_the_environment() {
        let home = PathBuf::from("/home/u");
        assert_eq!(
            grok_home(&environ(&[("GROK_HOME", "/g")]), &home),
            PathBuf::from("/g")
        );
        assert_eq!(
            grok_home(&environ(&[]), &home),
            PathBuf::from("/home/u/.grok")
        );
    }
}
