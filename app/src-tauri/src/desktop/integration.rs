//! 设置页「Agent 集成」的宿主侧实现。
//!
//! 三件事:把引擎二进制暴露成 PATH 里的 `ferry`、把打包的 Ferry skill 装进共享技能
//! 目录并补齐不读取共享目录的 Agent 入口、读引擎锁文件报服务状态。路径一律由宿主
//! 按契约算出,webview 只传目标 id。
//!
//! 方向上与 Runtime 的 skill *导入*正好相反:那边把别人的技能读进 Ferry 库,这边把
//! Ferry 自己的技能写进别人的目录。两条路径不共用代码。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

use super::{host_settings, platform};
use crate::contracts::agents::{AGENT_SKILL_PATHS, SHARED_SKILL_PATHS};
use crate::engine::daemon::{self, DaemonError};
use crate::process::command::sidecar_candidates;

/// 随 App 打包的 skill 目录名,同时也是各 skill 的 name。
///
/// 目标目录下装的是**一组**技能:发送侧的 `ferry`(搜/读/审计/迁移/交接)与
/// 接手侧的 `ferry-resume`。两者触发面不同,必须是两份独立的 SKILL.md;但对用户
/// 来说只有「Ferry 的技能装没装」这一件事,所以设置页只有一行,状态取组里
/// 最差的那个(未安装 > 有新版本 > 已安装)。
const BUNDLED_SKILLS: &[&str] = &["ferry", "ferry-resume"];
/// 唯一的安装真身——共享技能目录(`~/.agents/skills`)。大多数 Agent 直接读它;
/// Claude Code 这类只认自己目录的客户端由 [`skill_link_targets`] 补 symlink。
const SHARED_TARGET_ID: &str = "shared";
/// 当前支持列表里只有 Claude Code 不读 `~/.agents/skills`。这里保留 agent id 而不是
/// 硬编码路径,具体入口仍来自生成契约的 `skill_paths`。
const LINKED_SKILL_AGENTS: &[&str] = &["claude"];

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
    path: String,
    installed: bool,
    installed_version: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct IntegrationStatus {
    cli: CliStatus,
    skills: Vec<SkillTargetStatus>,
    /// 打包资源里整组 skill 的版本;资源缺失时为 None,前端据此禁用安装。
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

/// 唯一的安装目标:契约里声明的共享技能目录(取第一个能展开的)。
fn skill_target() -> Result<SkillTarget, String> {
    let path = SHARED_SKILL_PATHS
        .iter()
        .find_map(|raw| expand_home(raw).ok())
        .ok_or_else(|| "契约里没有可用的共享技能目录".to_owned())?;
    Ok(SkillTarget {
        id: SHARED_TARGET_ID.to_owned(),
        path,
    })
}

/// 不读取共享仓库的 Agent 原生技能目录。只为明确列入 [`LINKED_SKILL_AGENTS`] 的
/// 客户端建入口,避免 Codex/OpenCode 等重复发现同名技能。
fn skill_link_targets() -> Result<Vec<PathBuf>, String> {
    let mut targets = Vec::new();
    for agent_id in LINKED_SKILL_AGENTS {
        let paths = AGENT_SKILL_PATHS
            .iter()
            .find_map(|(id, paths)| (*id == *agent_id).then_some(*paths))
            .ok_or_else(|| format!("契约里没有 {agent_id} 的 skill 路径"))?;
        for raw in paths {
            targets.push(expand_home(raw)?);
        }
    }
    Ok(targets)
}

/// 打包资源里的一个 skill 源目录;开发模式(无打包资源)回退到仓库路径。
fn bundled_skill_dir(app: &AppHandle, name: &str) -> Result<PathBuf, String> {
    if let Ok(resource_dir) = app.path().resource_dir() {
        let packaged = resource_dir.join("skills").join(name);
        if packaged.join("SKILL.md").is_file() {
            return Ok(packaged);
        }
    }
    #[cfg(debug_assertions)]
    {
        let repository = crate::process::command::repository_root()
            .join("skills")
            .join(name);
        if repository.join("SKILL.md").is_file() {
            return Ok(repository);
        }
    }
    Err(format!("找不到打包的 {name} skill 资源"))
}

/// 整组打包 skill 的源目录;缺任何一份都算资源不完整(前端据此禁用安装)。
fn bundled_skill_dirs(app: &AppHandle) -> Result<Vec<PathBuf>, String> {
    BUNDLED_SKILLS
        .iter()
        .map(|name| bundled_skill_dir(app, name))
        .collect()
}

/// 整组打包 skill 的版本。两份 SKILL.md 的 `version:` 与 App 版本对齐,不一致
/// 说明发布时漏改了某一份——如实报最小的那个,让设置页显示「有新版本」而不是
/// 假装已经最新。
fn bundled_group_version(app: &AppHandle) -> Option<String> {
    let mut versions: Vec<String> = Vec::new();
    for name in BUNDLED_SKILLS {
        versions.push(skill_version_at(&bundled_skill_dir(app, name).ok()?)?);
    }
    versions.into_iter().min()
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

/// 目标目录的组状态:共享真身和所有必要的 Agent 链接都正确才算已安装。
fn skill_target_status(target: &SkillTarget, link_targets: &[PathBuf]) -> SkillTargetStatus {
    let dirs: Vec<PathBuf> = BUNDLED_SKILLS
        .iter()
        .map(|name| target.path.join(name))
        .collect();
    let links_ready = link_targets.iter().all(|link_root| {
        BUNDLED_SKILLS.iter().all(|name| {
            let source = target.path.join(name);
            let link = link_root.join(name);
            std::fs::symlink_metadata(&link)
                .map(|meta| meta.file_type().is_symlink() && same_target(&link, &source))
                .unwrap_or(false)
        })
    });
    let installed = dirs.iter().all(|dir| dir.is_dir()) && links_ready;
    // 组里任一份读不出版本就整体报 None(前端显示「已安装」但版本未知);
    // 都读得出时取最小值,漏更新的那一份因此会让这一行显示「有新版本」。
    let installed_version = installed
        .then(|| {
            dirs.iter()
                .map(|dir| skill_version_at(dir))
                .collect::<Option<Vec<String>>>()
        })
        .flatten()
        .and_then(|versions| versions.into_iter().min());
    SkillTargetStatus {
        id: target.id.clone(),
        path: target.path.display().to_string(),
        installed_version,
        installed,
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

/// 移除已安装的某个 skill 目录。目标是指向共享仓库的 symlink 时只摘链接,共享仓库原样保留。
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

/// 覆盖式安装一份 skill:先清掉旧目录再整目录复制,避免上个版本的残留文件混在里面。
fn install_skill_into(source: &Path, target_dir: &Path) -> Result<PathBuf, String> {
    let name = source
        .file_name()
        .ok_or_else(|| format!("非法的 skill 源目录: {}", source.display()))?;
    let destination = target_dir.join(name);
    std::fs::create_dir_all(target_dir)
        .map_err(|error| format!("创建 {} 失败: {error}", target_dir.display()))?;
    remove_skill_dir(&destination)?;
    copy_tree(source, &destination)?;
    Ok(destination)
}

/// 成组安装:一次装齐 [`BUNDLED_SKILLS`]。中途失败即返回,已装的那几份留在原地
/// ——组状态会如实显示成「未安装」(整组不全),用户再点一次即可。
fn install_skill_group(sources: &[PathBuf], target_dir: &Path) -> Result<Vec<PathBuf>, String> {
    sources
        .iter()
        .map(|source| install_skill_into(source, target_dir))
        .collect()
}

/// 在 Agent 原生目录创建指向共享真身的入口。只覆盖两类安全对象:
/// 1. 已经指向同一真身的链接;2. 断掉的 symlink。真实目录、文件或指向别处的链接都
/// 视为用户资产,拒绝覆盖。
fn install_skill_links(shared_dir: &Path, link_targets: &[PathBuf]) -> Result<(), String> {
    for link_root in link_targets {
        std::fs::create_dir_all(link_root)
            .map_err(|error| format!("创建 {} 失败: {error}", link_root.display()))?;
        for name in BUNDLED_SKILLS {
            let source = shared_dir.join(name);
            let link = link_root.join(name);
            match std::fs::symlink_metadata(&link) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    if same_target(&link, &source) {
                        continue;
                    }
                    if link.exists() {
                        return Err(format!(
                            "{} 已链接到其他位置,为避免覆盖已停止安装",
                            link.display()
                        ));
                    }
                    std::fs::remove_file(&link).map_err(|error| {
                        format!("移除断开的链接 {} 失败: {error}", link.display())
                    })?;
                }
                Ok(_) => {
                    return Err(format!(
                        "{} 已存在且不是 Ferry 管理的链接,为避免覆盖已停止安装",
                        link.display()
                    ));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(format!("检查 {} 失败: {error}", link.display()));
                }
            }
            platform::create_directory_link(&link, &source)?;
        }
    }
    Ok(())
}

/// 写共享真身前先做一遍只读冲突检查,避免 Agent 入口冲突时共享副本已经被更新一半。
fn validate_skill_link_slots(shared_dir: &Path, link_targets: &[PathBuf]) -> Result<(), String> {
    for link_root in link_targets {
        for name in BUNDLED_SKILLS {
            let source = shared_dir.join(name);
            let link = link_root.join(name);
            let Ok(meta) = std::fs::symlink_metadata(&link) else {
                continue;
            };
            if !meta.file_type().is_symlink() {
                return Err(format!(
                    "{} 已存在且不是 Ferry 管理的链接,为避免覆盖已停止安装",
                    link.display()
                ));
            }
            if link.exists() && !same_target(&link, &source) {
                return Err(format!(
                    "{} 已链接到其他位置,为避免覆盖已停止安装",
                    link.display()
                ));
            }
        }
    }
    Ok(())
}

/// 只摘掉确实指向 Ferry 共享真身的链接;冲突项与用户目录原样保留。
fn remove_skill_links(shared_dir: &Path, link_targets: &[PathBuf]) -> Result<(), String> {
    for link_root in link_targets {
        for name in BUNDLED_SKILLS {
            let source = shared_dir.join(name);
            let link = link_root.join(name);
            let Ok(meta) = std::fs::symlink_metadata(&link) else {
                continue;
            };
            if meta.file_type().is_symlink() && same_target(&link, &source) {
                std::fs::remove_file(&link)
                    .map_err(|error| format!("移除 skill 入口 {} 失败: {error}", link.display()))?;
            }
        }
    }
    Ok(())
}

/// 成组移除:只删 Ferry 自己装的那几个目录,同目录下别人的技能不碰。
fn remove_skill_group(target_dir: &Path) -> Result<(), String> {
    for name in BUNDLED_SKILLS {
        remove_skill_dir(&target_dir.join(name))?;
    }
    Ok(())
}

fn find_target(target_id: &str) -> Result<SkillTarget, String> {
    let target = skill_target()?;
    if target.id != target_id {
        return Err(format!("未知的 skill 安装目标: {target_id}"));
    }
    Ok(target)
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
        let target = skill_target()?;
        let link_targets = skill_link_targets()?;
        Ok(IntegrationStatus {
            cli: cli_status(&app),
            skills: vec![skill_target_status(&target, &link_targets)],
            bundled_version: bundled_group_version(&app),
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
        let sources = bundled_skill_dirs(&app)?;
        let link_targets = skill_link_targets()?;
        validate_skill_link_slots(&target.path, &link_targets)?;
        install_skill_group(&sources, &target.path)?;
        install_skill_links(&target.path, &link_targets)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn skill_uninstall(target_id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let target = find_target(&target_id)?;
        let link_targets = skill_link_targets()?;
        remove_skill_links(&target.path, &link_targets)?;
        remove_skill_group(&target.path)
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
