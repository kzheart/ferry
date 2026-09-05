//! Cross-platform locations for external session stores.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf, MAIN_SEPARATOR};

/// 环境变量视图；测试可以注入而不污染进程环境（对齐 Python 的 `environ` 形参）。
pub type Environ = BTreeMap<String, String>;

/// 采集当前进程环境。
pub fn process_environ() -> Environ {
    std::env::vars().collect()
}

/// 会影响存储布局的桌面平台。不要把 macOS 与 Linux 都压成 `posix`：两者的
/// Cursor、配置目录约定不同，压平后测试只能偷偷依赖当前编译目标。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Platform {
    Windows,
    MacOs,
    Linux,
}

impl Platform {
    pub fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Linux
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

/// 契约与 adapter 共用的路径入口：展开 `~`、`{home}` / `{config}` /
/// `{data_local}` / `{localappdata}`、以及 `%VAR%`。
///
/// `{config}` 对齐 VS Code / Cursor（`dirs::config_dir`）：macOS 是
/// `~/Library/Application Support`，Linux 是 `~/.config`，Windows 是 `%APPDATA%`。
/// `{data_local}` 对齐 XDG data 与 `%LOCALAPPDATA%`。契约里只写一份模板，
/// 运行时再落到当前平台——不要再为每个 Agent 写死 macOS 路径。
pub fn expanduser(path: &str) -> PathBuf {
    expand_location(path)
}

/// 按当前进程环境展开路径模板。
pub fn expand_location(spec: &str) -> PathBuf {
    expand_location_with(spec, Platform::current(), &process_environ(), &home_dir())
}

/// 可注入环境的路径展开，供单测与 adapter 共用同一套规则。
pub fn expand_location_with(
    spec: &str,
    platform: Platform,
    environ: &Environ,
    home: &Path,
) -> PathBuf {
    let with_env = expand_percent_vars(spec, environ);
    let with_tokens = expand_location_tokens(&with_env, platform, environ, home);
    normalize_windows_separators(expand_tilde(&with_tokens, home))
}

fn normalize_windows_separators(path: PathBuf) -> PathBuf {
    if !cfg!(windows) {
        return path;
    }
    let text = path.to_string_lossy();
    let windows_rooted = text.chars().nth(1) == Some(':') || text.starts_with(r"\\");
    if windows_rooted && text.contains('/') {
        PathBuf::from(text.replace('/', r"\"))
    } else {
        path
    }
}

/// VS Code 风格的用户配置根（Cursor 的 `globalStorage` 就挂在这下面）。
pub fn config_dir(platform: Platform, environ: &Environ, home: &Path) -> PathBuf {
    match platform {
        Platform::Windows => env_value(environ, "APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData").join("Roaming")),
        Platform::MacOs => home.join("Library").join("Application Support"),
        Platform::Linux => env_value(environ, "XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config")),
    }
}

/// 本机数据根：OpenCode 的 SQLite 库、Windows 上的 Cursor CLI 安装目录。
pub fn data_local_dir(platform: Platform, environ: &Environ, home: &Path) -> PathBuf {
    match platform {
        Platform::Windows => env_value(environ, "LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData").join("Local")),
        Platform::MacOs | Platform::Linux => env_value(environ, "XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local").join("share")),
    }
}

/// Windows `canonicalize` 会加上 `\\?\` 前缀；UI 与 SQLite 都不该看到它。
pub fn strip_verbatim_prefix(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path
    }
}

/// 给 UI / 状态栏看的路径，去掉 Windows 长路径前缀。
pub fn display_path(path: &Path) -> String {
    strip_verbatim_prefix(path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

fn expand_percent_vars(spec: &str, environ: &Environ) -> String {
    let chars: Vec<char> = spec.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '%' {
            if let Some(relative_end) = chars[index + 1..].iter().position(|&ch| ch == '%') {
                let name: String = chars[index + 1..index + 1 + relative_end].iter().collect();
                if !name.is_empty() {
                    if let Some(value) = env_value_ignore_ascii_case(environ, &name) {
                        out.push_str(value);
                        index += name.chars().count() + 2;
                        continue;
                    }
                }
            }
        }
        out.push(chars[index]);
        index += 1;
    }
    out
}

fn expand_location_tokens(
    spec: &str,
    platform: Platform,
    environ: &Environ,
    home: &Path,
) -> String {
    let home_text = home.to_string_lossy();
    let config_text = config_dir(platform, environ, home)
        .to_string_lossy()
        .into_owned();
    let data_local_text = data_local_dir(platform, environ, home)
        .to_string_lossy()
        .into_owned();
    spec.replace("{home}", home_text.as_ref())
        .replace("{config}", &config_text)
        .replace("{data_local}", &data_local_text)
        .replace("{localappdata}", &data_local_text)
}

fn expand_tilde(path: &str, home: &Path) -> PathBuf {
    if path == "~" {
        return home.to_path_buf();
    }
    let separators: &[char] = &['/', '\\', MAIN_SEPARATOR];
    if let Some(rest) = path.strip_prefix('~') {
        if rest.starts_with(separators) {
            let mut out = home.to_path_buf();
            for part in rest.split(['/', '\\']).filter(|part| !part.is_empty()) {
                out.push(part);
            }
            return out;
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

fn env_value_ignore_ascii_case<'a>(environ: &'a Environ, key: &str) -> Option<&'a str> {
    if let Some(value) = env_value(environ, key) {
        return Some(value);
    }
    environ.iter().find_map(|(candidate, value)| {
        candidate
            .eq_ignore_ascii_case(key)
            .then_some(value.as_str())
            .filter(|value| !value.is_empty())
    })
}

/// opencode 的只读会话库位置。
///
/// OpenCode CLI 在 Windows 上仍走 XDG：`~/.local/share/opencode/opencode.db`，
/// 不是 `%LOCALAPPDATA%`。桌面安装器有时会写 Local/Roaming，所以按「文件在不在」探测。
pub fn opencode_database_path(platform: Platform, environ: &Environ, home: &Path) -> PathBuf {
    if let Some(override_path) = env_value(environ, "FERRY_OPENCODE_DB") {
        return expanduser(override_path);
    }
    if let Some(override_path) = env_value(environ, "OPENCODE_DB") {
        return expanduser(override_path);
    }
    let xdg = home
        .join(".local")
        .join("share")
        .join("opencode")
        .join("opencode.db");
    if platform != Platform::Windows {
        return data_local_dir(platform, environ, home)
            .join("opencode")
            .join("opencode.db");
    }
    let local = data_local_dir(platform, environ, home)
        .join("opencode")
        .join("opencode.db");
    let roaming = config_dir(platform, environ, home)
        .join("opencode")
        .join("opencode.db");
    for candidate in [&xdg, &local, &roaming] {
        if candidate.exists() {
            return candidate.clone();
        }
    }
    xdg
}

/// Cursor 的只读会话库位置（`state.vscdb`）。
///
/// Cursor 是 VS Code 分支，globalStorage 的落位随桌面平台不同。平台由调用方
/// 显式传入，Windows 上也能覆盖 macOS 布局测试，不再偷看当前编译目标。
pub fn cursor_database_path(platform: Platform, environ: &Environ, home: &Path) -> PathBuf {
    if let Some(override_path) = env_value(environ, "FERRY_CURSOR_DB") {
        return expanduser(override_path);
    }
    let leaf = |base: PathBuf| {
        base.join("Cursor")
            .join("User")
            .join("globalStorage")
            .join("state.vscdb")
    };
    if platform == Platform::Windows {
        return leaf(config_dir(platform, environ, home));
    }
    let macos = leaf(home.join("Library").join("Application Support"));
    let linux = leaf(
        env_value(environ, "XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config")),
    );
    match platform {
        Platform::MacOs => macos,
        Platform::Linux => linux,
        Platform::Windows => unreachable!(),
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
            let guard = Self::acquire().set("HOME", path.as_ref());
            #[cfg(windows)]
            let guard = guard.set("USERPROFILE", path.as_ref());
            guard
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
    fn config_roots_are_selected_by_injected_platform_not_build_host() {
        let home = PathBuf::from("/home/u");
        assert_eq!(
            config_dir(Platform::MacOs, &environ(&[]), &home),
            PathBuf::from("/home/u/Library/Application Support")
        );
        assert_eq!(
            config_dir(
                Platform::Linux,
                &environ(&[("XDG_CONFIG_HOME", "/xdg/config")]),
                &home
            ),
            PathBuf::from("/xdg/config")
        );
        assert_eq!(
            config_dir(
                Platform::Windows,
                &environ(&[("APPDATA", r"C:\Roaming")]),
                &home
            ),
            PathBuf::from(r"C:\Roaming")
        );
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
                Platform::Linux,
                &environ(&[("FERRY_OPENCODE_DB", "/custom/db.sqlite")]),
                &home
            ),
            PathBuf::from("/custom/db.sqlite")
        );
        assert_eq!(
            opencode_database_path(Platform::Linux, &environ(&[]), &home),
            PathBuf::from("/home/u/.local/share/opencode/opencode.db")
        );
        assert_eq!(
            opencode_database_path(
                Platform::Linux,
                &environ(&[("XDG_DATA_HOME", "/xdg")]),
                &home
            ),
            PathBuf::from("/xdg/opencode/opencode.db")
        );
        assert_eq!(
            opencode_database_path(
                Platform::Windows,
                &environ(&[("LOCALAPPDATA", "C:\\Local")]),
                &home
            ),
            home.join(".local")
                .join("share")
                .join("opencode")
                .join("opencode.db")
        );
    }

    #[test]
    fn opencode_windows_uses_an_existing_localappdata_store() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let local = root.path().join("local");
        std::fs::create_dir_all(local.join("opencode")).unwrap();
        let database = local.join("opencode").join("opencode.db");
        std::fs::write(&database, b"").unwrap();
        assert_eq!(
            opencode_database_path(
                Platform::Windows,
                &environ(&[("LOCALAPPDATA", local.to_str().unwrap())]),
                &home
            ),
            database
        );
    }

    #[test]
    fn cursor_path_follows_the_explicit_platform() {
        let home = PathBuf::from("/home/u");
        assert_eq!(
            cursor_database_path(
                Platform::Linux,
                &environ(&[("FERRY_CURSOR_DB", "/custom/state.vscdb")]),
                &home
            ),
            PathBuf::from("/custom/state.vscdb")
        );
        assert_eq!(
            cursor_database_path(Platform::Linux, &environ(&[]), &home),
            PathBuf::from("/home/u/.config/Cursor/User/globalStorage/state.vscdb")
        );
        assert_eq!(
            cursor_database_path(Platform::MacOs, &environ(&[]), &home),
            PathBuf::from(
                "/home/u/Library/Application Support/Cursor/User/globalStorage/state.vscdb"
            )
        );
        assert_eq!(
            cursor_database_path(
                Platform::Windows,
                &environ(&[("APPDATA", "C:\\Roaming")]),
                &home
            ),
            PathBuf::from("C:\\Roaming")
                .join("Cursor")
                .join("User")
                .join("globalStorage")
                .join("state.vscdb")
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

    #[test]
    fn location_templates_expand_posix_home_and_xdg() {
        let home = PathBuf::from("/home/u");
        assert_eq!(
            expand_location_with("~/.claude/projects", Platform::Linux, &environ(&[]), &home),
            PathBuf::from("/home/u/.claude/projects")
        );
        assert_eq!(
            expand_location_with(
                "{data_local}/opencode",
                Platform::Linux,
                &environ(&[]),
                &home
            ),
            PathBuf::from("/home/u/.local/share/opencode")
        );
    }

    #[cfg(windows)]
    #[test]
    fn location_templates_resolve_windows_roots() {
        let home = PathBuf::from(r"C:\Users\u");
        assert_eq!(
            expand_location_with(
                "{config}/Cursor/User/globalStorage",
                Platform::Windows,
                &environ(&[("APPDATA", r"C:\Roaming")]),
                &home
            ),
            PathBuf::from(r"C:\Roaming")
                .join("Cursor")
                .join("User")
                .join("globalStorage")
        );
        assert_eq!(
            expand_location_with(
                "{data_local}/opencode",
                Platform::Windows,
                &environ(&[("LOCALAPPDATA", r"C:\Local")]),
                &home
            ),
            PathBuf::from(r"C:\Local").join("opencode")
        );
        assert_eq!(
            expand_location_with(
                r"%APPDATA%/Cursor/User/globalStorage",
                Platform::Windows,
                &environ(&[("APPDATA", r"C:\Roaming")]),
                &home
            ),
            PathBuf::from(r"C:\Roaming")
                .join("Cursor")
                .join("User")
                .join("globalStorage")
        );
    }

    #[test]
    fn strip_verbatim_prefix_drops_windows_long_path_marker() {
        assert_eq!(
            strip_verbatim_prefix(PathBuf::from(r"\\?\C:\Users\a\state.vscdb")),
            PathBuf::from(r"C:\Users\a\state.vscdb")
        );
        assert_eq!(
            strip_verbatim_prefix(PathBuf::from(r"\\?\UNC\server\share\db")),
            PathBuf::from(r"\\server\share\db")
        );
        assert_eq!(
            strip_verbatim_prefix(PathBuf::from("/tmp/db")),
            PathBuf::from("/tmp/db")
        );
    }
}
