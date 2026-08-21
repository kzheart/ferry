//! 设置页「Agent 集成」的宿主侧实现。
//!
//! 三件事:把引擎二进制暴露成 PATH 里的 `ferry`、把打包的 Ferry skill 装进各 agent 的
//! skill 目录、读引擎锁文件报服务状态。路径一律由宿主按契约与固定规则算出,webview
//! 只传目标 id;唯一的例外是自定义安装目录,而它只能来自系统目录选择对话框。
//!
//! 方向上与 Runtime 的 skill *导入*正好相反:那边把别人的技能读进 Ferry 库,这边把
//! Ferry 自己的技能写进别人的目录。两条路径不共用代码。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

use super::{host_settings, platform};
use crate::contracts::agents::{AGENT_SKILL_TARGETS, SHARED_SKILL_PATHS};
use crate::engine::daemon::{self, DaemonError};
use crate::process::command::sidecar_candidates;

/// 安装到目标目录里的子目录名,同时也是 skill 的 name。
const SKILL_NAME: &str = "ferry";
/// 共享技能仓库(`~/.agents/skills`)不属于任何 agent,用固定 id 表示。
const SHARED_TARGET_ID: &str = "shared";

#[derive(Serialize)]
pub(crate) struct CliStatus {
    /// 当前平台是否实现了 CLI 安装。false 时前端只展示 unsupported 提示。
    supported: bool,
    unsupported_reason: Option<String>,
    /// 安装点(`~/.local/bin/ferry`),平台不支持时为 None。
    link_path: Option<String>,
    installed: bool,
    /// symlink 指向的路径,原样展示给用户。
    link_target: Option<String>,
    /// 指向的是不是**本 App 当前使用的**引擎二进制。false 且 installed=true 即「需要更新」。
    points_to_current_engine: bool,
    engine_path: Option<String>,
    /// 安装目录是否在 App 进程的 PATH 里。不在也算已安装,只是要提示配置 shell。
    on_path: bool,
}

#[derive(Serialize)]
pub(crate) struct SkillTargetStatus {
    id: String,
    /// agent 的展示名;共享仓库为空,由前端出文案。
    display_name: String,
    path: String,
    installed: bool,
    installed_version: Option<String>,
    /// 该目标下的 `ferry/` 其实是共享仓库里那一份(symlink 农场),不该重复安装。
    via_shared: bool,
}

#[derive(Serialize)]
pub(crate) struct IntegrationStatus {
    cli: CliStatus,
    skills: Vec<SkillTargetStatus>,
    /// 打包资源里的 skill 版本;资源缺失时为 None,前端据此禁用安装。
    bundled_version: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct EngineServiceStatus {
    /// `app-shared` = App sidecar 兼听 socket;`daemon` = CLI 拉起的独立进程;`stopped` = 无。
    state: &'static str,
    pid: Option<u32>,
    socket: Option<String>,
    /// socket 文件是否真的在。锁在但 socket 没了说明进程正在退出或异常。
    socket_ready: bool,
    version: Option<String>,
}

/// `~/.ferry/engine.lock` 的内容。引擎侧字段还在演进,除 pid 外一律可缺。
#[derive(Deserialize)]
struct EngineLock {
    pid: u32,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    socket: Option<String>,
    #[serde(default)]
    version: Option<String>,
}

struct SkillTarget {
    id: String,
    display_name: String,
    path: PathBuf,
}

/// 契约里的 `~/...` 展开成绝对路径。只认开头的 `~/`,不做 `~user` 展开。
fn expand_home(raw: &str) -> Result<PathBuf, String> {
    match raw.strip_prefix("~/") {
        Some(rest) => Ok(platform::home_dir()?.join(rest)),
        None => Ok(PathBuf::from(raw)),
    }
}

/// 两个路径是否落在同一个真实文件上。symlink 链、`.`/`..` 都在 canonicalize 里抹平;
/// 任一侧解析不了(断链、二进制不存在)就判否。
fn same_target(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// SKILL.md frontmatter 里的 `version:`。frontmatter 是首行 `---` 起、下一条 `---` 止的块;
/// 这里只认顶层的一个标量字段,不引入 YAML 解析器。
fn parse_skill_version(text: &str) -> Option<String> {
    let mut lines = text.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for line in lines {
        if line.trim() == "---" {
            return None;
        }
        // 不 trim 行首:缩进的 version 是嵌套字段,不是我们要的那一个。
        if let Some(rest) = line.strip_prefix("version:") {
            let value = rest.trim().trim_matches(|c| c == '"' || c == '\'');
            if !value.is_empty() {
                return Some(value.to_owned());
            }
        }
    }
    None
}

fn skill_version_at(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("SKILL.md")).ok()?;
    parse_skill_version(&text)
}

/// 固定目标表:契约里 skill_paths 非空的 agent + 共享技能仓库。
fn skill_targets() -> Result<Vec<SkillTarget>, String> {
    let mut targets: Vec<SkillTarget> = Vec::new();
    let declared = AGENT_SKILL_TARGETS
        .iter()
        .map(|(id, name, path)| (*id, *name, *path))
        .chain(
            SHARED_SKILL_PATHS
                .iter()
                .map(|path| (SHARED_TARGET_ID, "", *path)),
        );
    for (id, display_name, raw) in declared {
        // 同一个 id 声明了多个目录时追加序号,保证 target_id 唯一可寻址。
        let taken = targets
            .iter()
            .filter(|target| target.id == id || target.id.starts_with(&format!("{id}:")))
            .count();
        let id = if taken == 0 {
            id.to_owned()
        } else {
            format!("{id}:{taken}")
        };
        targets.push(SkillTarget {
            id,
            display_name: display_name.to_owned(),
            path: expand_home(raw)?,
        });
    }
    Ok(targets)
}

/// 共享仓库里那份 `ferry/` 的位置,用于识别「经共享仓库生效」的 symlink 农场。
fn shared_skill_dirs() -> Vec<PathBuf> {
    SHARED_SKILL_PATHS
        .iter()
        .filter_map(|raw| expand_home(raw).ok())
        .map(|path| path.join(SKILL_NAME))
        .collect()
}

/// 打包资源里的 skill 源目录;开发模式(无打包资源)回退到仓库路径。
fn bundled_skill_dir(app: &AppHandle) -> Result<PathBuf, String> {
    if let Ok(resource_dir) = app.path().resource_dir() {
        let packaged = resource_dir.join("skills").join(SKILL_NAME);
        if packaged.join("SKILL.md").is_file() {
            return Ok(packaged);
        }
    }
    #[cfg(debug_assertions)]
    {
        let repository = crate::process::command::repository_root()
            .join("skills")
            .join(SKILL_NAME);
        if repository.join("SKILL.md").is_file() {
            return Ok(repository);
        }
    }
    Err("找不到打包的 Ferry skill 资源".to_owned())
}

/// 本 App 当前会启动的那个引擎二进制。与 `engine::engine_command` 同一套查找顺序,
/// 区别只是这里要的是路径而不是 Command。
fn current_engine_path(app: &AppHandle) -> Option<PathBuf> {
    if let Ok(resource_dir) = app.path().resource_dir() {
        if let Some(path) = sidecar_candidates(&resource_dir, "ferry-engine")
            .into_iter()
            .find(|path| path.is_file())
        {
            return Some(path);
        }
    }
    #[cfg(debug_assertions)]
    {
        crate::process::command::local_engine_path()
    }
    #[cfg(not(debug_assertions))]
    None
}

/// 目录是否在 App 进程的 PATH 里。GUI 启动的 PATH 已由 fix-path-env 从登录 shell 恢复,
/// 所以这个判断反映的就是用户终端里的实际可见性。
fn dir_on_path(dir: &Path) -> bool {
    let Some(raw) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&raw).any(|entry| entry == dir || same_target(&entry, dir))
}

/// 链接指向的文件名是不是 Ferry 引擎。卸载只删我们自己装的入口,
/// 不碰用户放在同一位置的同名文件。
fn points_to_ferry_engine(target: &Path) -> bool {
    target
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name == "ferry-engine" || name == "ferry-engine.exe")
        .unwrap_or(false)
}

fn cli_status(app: &AppHandle) -> CliStatus {
    let engine = current_engine_path(app);
    let engine_path = engine.as_ref().map(|path| path.display().to_string());
    let link = match platform::cli_link_path() {
        Ok(link) => link,
        Err(reason) => {
            return CliStatus {
                supported: false,
                unsupported_reason: Some(reason),
                link_path: None,
                installed: false,
                link_target: None,
                points_to_current_engine: false,
                engine_path,
                on_path: false,
            };
        }
    };
    // 断链也算「装过」:用户看到的是一个坏掉的入口,该给的是更新按钮而不是安装按钮。
    let link_target = std::fs::read_link(&link).ok();
    let points_to_current_engine = match (link_target.as_deref(), engine.as_deref()) {
        (Some(target), Some(engine)) => same_target(target, engine),
        _ => false,
    };
    CliStatus {
        supported: true,
        unsupported_reason: None,
        on_path: link.parent().map(dir_on_path).unwrap_or(false),
        link_path: Some(link.display().to_string()),
        installed: link_target.is_some(),
        link_target: link_target.map(|path| path.display().to_string()),
        points_to_current_engine,
        engine_path,
    }
}

fn skill_target_status(target: &SkillTarget, shared: &[PathBuf]) -> SkillTargetStatus {
    let installed_dir = target.path.join(SKILL_NAME);
    // is_dir 走 stat 语义:目标目录本身或其中的 ferry/ 是 symlink 时按解析后的真实路径判断。
    let installed = installed_dir.is_dir();
    let via_shared = installed
        && target.id != SHARED_TARGET_ID
        && !target.id.starts_with(&format!("{SHARED_TARGET_ID}:"))
        && shared.iter().any(|path| same_target(&installed_dir, path));
    SkillTargetStatus {
        id: target.id.clone(),
        display_name: target.display_name.clone(),
        path: target.path.display().to_string(),
        installed_version: installed
            .then(|| skill_version_at(&installed_dir))
            .flatten(),
        installed,
        via_shared,
    }
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|error| format!("创建 {} 失败: {error}", to.display()))?;
    let entries = std::fs::read_dir(from)
        .map_err(|error| format!("读取 {} 失败: {error}", from.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let source = entry.path();
        let destination = to.join(entry.file_name());
        if source.is_dir() {
            copy_tree(&source, &destination)?;
        } else {
            std::fs::copy(&source, &destination)
                .map_err(|error| format!("写入 {} 失败: {error}", destination.display()))?;
        }
    }
    Ok(())
}

/// 移除已安装的 `ferry/`。目标是指向共享仓库的 symlink 时只摘链接,共享仓库原样保留。
fn remove_skill_dir(dir: &Path) -> Result<(), String> {
    let Ok(meta) = std::fs::symlink_metadata(dir) else {
        return Ok(());
    };
    let removed = if meta.file_type().is_symlink() || meta.is_file() {
        std::fs::remove_file(dir)
    } else {
        std::fs::remove_dir_all(dir)
    };
    removed.map_err(|error| format!("移除 {} 失败: {error}", dir.display()))
}

/// 覆盖式安装:先清掉旧的 `ferry/` 再整目录复制,避免上个版本的残留文件混在里面。
fn install_skill_into(source: &Path, target_dir: &Path) -> Result<PathBuf, String> {
    let destination = target_dir.join(SKILL_NAME);
    std::fs::create_dir_all(target_dir)
        .map_err(|error| format!("创建 {} 失败: {error}", target_dir.display()))?;
    remove_skill_dir(&destination)?;
    copy_tree(source, &destination)?;
    Ok(destination)
}

fn find_target(target_id: &str) -> Result<SkillTarget, String> {
    skill_targets()?
        .into_iter()
        .find(|target| target.id == target_id)
        .ok_or_else(|| format!("未知的 skill 安装目标: {target_id}"))
}

fn engine_lock_path() -> Result<PathBuf, String> {
    Ok(platform::home_dir()?.join(".ferry").join("engine.lock"))
}

fn parse_engine_lock(text: &str) -> Option<EngineLock> {
    serde_json::from_str::<EngineLock>(text).ok()
}

/// 锁在且 pid 活着才算在跑;其余一律 stopped(陈旧锁不该显示成运行中)。
fn service_status(
    lock: Option<EngineLock>,
    alive: bool,
    socket_ready: bool,
) -> EngineServiceStatus {
    let Some(lock) = lock.filter(|_| alive) else {
        return EngineServiceStatus {
            state: "stopped",
            pid: None,
            socket: None,
            socket_ready: false,
            version: None,
        };
    };
    EngineServiceStatus {
        state: if lock.mode.as_deref() == Some("app") {
            "app-shared"
        } else {
            "daemon"
        },
        pid: Some(lock.pid),
        socket: lock.socket,
        socket_ready,
        version: lock.version,
    }
}

#[tauri::command]
pub(crate) async fn integration_status(app: AppHandle) -> Result<IntegrationStatus, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let shared = shared_skill_dirs();
        Ok(IntegrationStatus {
            cli: cli_status(&app),
            skills: skill_targets()?
                .iter()
                .map(|target| skill_target_status(target, &shared))
                .collect(),
            bundled_version: bundled_skill_dir(&app)
                .ok()
                .and_then(|dir| skill_version_at(&dir)),
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn cli_install(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let engine = current_engine_path(&app).ok_or("找不到 Ferry 引擎二进制")?;
        platform::create_cli_link(&platform::cli_link_path()?, &engine)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn cli_uninstall() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| {
        let link = platform::cli_link_path()?;
        let Ok(target) = std::fs::read_link(&link) else {
            // 不是我们装的 symlink(或压根不存在)就什么都不做,绝不误删同名文件。
            return Ok(());
        };
        if !points_to_ferry_engine(&target) {
            return Err(format!("{} 不是 Ferry 安装的入口,已跳过", link.display()));
        }
        std::fs::remove_file(&link).map_err(|error| format!("移除 CLI 入口失败: {error}"))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn skill_install(app: AppHandle, target_id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let target = find_target(&target_id)?;
        let source = bundled_skill_dir(&app)?;
        install_skill_into(&source, &target.path).map(|_| ())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn skill_uninstall(target_id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let target = find_target(&target_id)?;
        remove_skill_dir(&target.path.join(SKILL_NAME))
    })
    .await
    .map_err(|error| error.to_string())?
}

/// 自定义目录安装。path 只应来自 `pick_skill_directory` 的返回值——webview 没有别的
/// 途径得到一个存在的绝对目录路径,这里再校验一次形状,拒绝相对路径与非目录。
#[tauri::command]
pub(crate) async fn skill_install_custom(app: AppHandle, path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let target_dir = PathBuf::from(&path);
        if !target_dir.is_absolute() {
            return Err("安装目录必须是绝对路径".to_owned());
        }
        if !target_dir.is_dir() {
            return Err("安装目录不存在".to_owned());
        }
        let source = bundled_skill_dir(&app)?;
        install_skill_into(&source, &target_dir).map(|dir| dir.display().to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn engine_service_status() -> Result<EngineServiceStatus, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let lock = std::fs::read_to_string(engine_lock_path()?)
            .ok()
            .as_deref()
            .and_then(parse_engine_lock);
        let alive = lock
            .as_ref()
            .map(|lock| platform::process_alive(lock.pid))
            .unwrap_or(false);
        let socket_ready = lock
            .as_ref()
            .and_then(|lock| lock.socket.as_deref())
            .map(|socket| std::fs::symlink_metadata(socket).is_ok())
            .unwrap_or(false);
        Ok(service_status(lock, alive, socket_ready))
    })
    .await
    .map_err(|error| error.to_string())?
}

/// 「允许 CLI 共享 App 引擎」的当前值。真值在宿主的配置文件里,不在 WebView 里。
#[tauri::command]
pub(crate) async fn get_engine_share() -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(host_settings::engine_share)
        .await
        .map_err(|error| error.to_string())
}

/// 改开关。只落盘,不动正在跑的引擎:sidecar 只在启动时决定要不要监听 socket,
/// 所以这次改动要等下次启动 App 才生效(前端负责提示)。
#[tauri::command]
pub(crate) async fn set_engine_share(enabled: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || host_settings::set_engine_share(enabled))
        .await
        .map_err(|error| error.to_string())?
}

/// 停止 CLI 拉起的独立 daemon。App 自己的引擎会以 `app_mode` 被拒。
#[tauri::command]
pub(crate) async fn engine_daemon_stop() -> Result<(), DaemonError> {
    tauri::async_runtime::spawn_blocking(daemon::stop)
        .await
        .map_err(|error| DaemonError {
            code: "unavailable",
            message: error.to_string(),
        })?
}

#[cfg(test)]
mod tests;
