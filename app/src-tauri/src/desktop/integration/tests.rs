use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use super::{
    copy_tree, expand_home, find_target, install_skill_group, install_skill_into,
    parse_engine_lock, parse_skill_version, points_to_ferry_engine, remove_skill_dir,
    remove_skill_group, same_target, service_status, skill_target, skill_target_status,
    skill_version_at, SkillTarget, BUNDLED_SKILLS, SHARED_TARGET_ID,
};

static SCRATCH_SEQUENCE: AtomicU32 = AtomicU32::new(0);

/// 一次性工作目录。文件系统行为(symlink、覆盖、递归删除)只能对着真实目录测,
/// 但绝不能碰用户的 `~/.claude` 之类真实安装点。
struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let unique = SCRATCH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "ferry-integration-{label}-{}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("创建临时目录");
        Self(path)
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("创建父目录");
    }
    std::fs::write(path, contents).expect("写入文件");
}

fn skill_source(scratch: &Scratch, version: &str) -> PathBuf {
    skill_group(scratch, version).remove(0)
}

/// 打包资源里的**一组** skill:与 `BUNDLED_SKILLS` 一一对应。
fn skill_group(scratch: &Scratch, version: &str) -> Vec<PathBuf> {
    BUNDLED_SKILLS
        .iter()
        .map(|name| {
            let source = scratch.join(&format!("bundled/{name}"));
            write(
                &source.join("SKILL.md"),
                &format!("---\nname: {name}\nversion: {version}\n---\n\n# {name}\n"),
            );
            source
        })
        .collect()
}

#[test]
fn skill_version_comes_from_the_frontmatter_top_level() {
    assert_eq!(
        parse_skill_version("---\nname: ferry\nversion: 0.7.0\n---\n# Ferry\n"),
        Some("0.7.0".to_owned()),
    );
    assert_eq!(
        parse_skill_version("---\nversion: \"1.2.3\"\n---\n"),
        Some("1.2.3".to_owned()),
    );
    // 没有 frontmatter、缩进字段、正文里的同名行都不算版本。
    assert_eq!(parse_skill_version("# Ferry\nversion: 9.9.9\n"), None);
    assert_eq!(parse_skill_version("---\n  version: 9.9.9\n---\n"), None);
    assert_eq!(
        parse_skill_version("---\nname: ferry\n---\nversion: 9.9.9\n"),
        None
    );
}

#[test]
fn engine_lock_without_a_live_process_reads_as_stopped() {
    let lock = parse_engine_lock(
        r#"{"pid": 4242, "mode": "daemon", "socket": "/tmp/engine.sock", "version": "0.7.0"}"#,
    );
    assert!(lock.is_some());
    let stopped = service_status(lock, false, false);
    assert_eq!(stopped.state, "stopped");
    assert!(stopped.pid.is_none());
    assert!(stopped.socket.is_none());
    assert_eq!(service_status(None, true, true).state, "stopped");
}

#[test]
fn engine_lock_mode_decides_between_the_app_and_a_daemon() {
    let app = parse_engine_lock(r#"{"pid": 7, "mode": "app", "socket": "/tmp/a.sock"}"#);
    let status = service_status(app, true, true);
    assert_eq!(status.state, "app-shared");
    assert_eq!(status.pid, Some(7));
    assert_eq!(status.socket.as_deref(), Some("/tmp/a.sock"));
    assert!(status.socket_ready);

    // mode 缺失按 daemon 处理:App 模式一定是我们自己写的锁,不会漏字段。
    let daemon = parse_engine_lock(r#"{"pid": 9}"#);
    assert_eq!(service_status(daemon, true, false).state, "daemon");
    assert!(parse_engine_lock("not json").is_none());
    assert!(parse_engine_lock(r#"{"mode": "daemon"}"#).is_none());
}

#[test]
fn uninstall_only_recognises_links_that_point_at_the_ferry_engine() {
    assert!(points_to_ferry_engine(Path::new(
        "/Apps/Ferry.app/ferry-engine"
    )));
    // 用当前平台的分隔符写 Windows 产物名:file_name() 只在原生平台上认 `\`。
    assert!(points_to_ferry_engine(Path::new("/Ferry/ferry-engine.exe")));
    assert!(!points_to_ferry_engine(Path::new("/usr/local/bin/ferry")));
    assert!(!points_to_ferry_engine(Path::new("/opt/other/engine")));
}

#[cfg(unix)]
#[test]
fn a_symlink_and_its_target_resolve_to_the_same_file() {
    let scratch = Scratch::new("same-target");
    let binary = scratch.join("ferry-engine");
    write(&binary, "#!/bin/sh\n");
    let link = scratch.join("bin/ferry");
    std::fs::create_dir_all(link.parent().unwrap()).expect("创建 bin");
    std::os::unix::fs::symlink(&binary, &link).expect("建立 symlink");

    assert!(same_target(&link, &binary));
    assert!(!same_target(&link, &scratch.join("other")));
    // 断链两侧都解析不了,判否而不是 panic。
    assert!(!same_target(
        &scratch.join("missing"),
        &scratch.join("missing")
    ));
}

#[test]
fn installing_replaces_the_previous_copy_instead_of_merging_into_it() {
    let scratch = Scratch::new("install");
    let source = skill_source(&scratch, "0.7.0");
    let target_dir = scratch.join("agent/skills");

    let installed = install_skill_into(&source, &target_dir).expect("首次安装");
    assert_eq!(installed, target_dir.join("ferry"));
    assert_eq!(skill_version_at(&installed).as_deref(), Some("0.7.0"));

    // 上一版留下的文件必须随覆盖式更新一起消失。
    write(&installed.join("legacy.md"), "旧版残留");
    let newer = skill_source(&scratch, "0.8.0");
    install_skill_into(&newer, &target_dir).expect("覆盖更新");
    assert_eq!(skill_version_at(&installed).as_deref(), Some("0.8.0"));
    assert!(!installed.join("legacy.md").exists());
}

/// 安装是成组的:一次装齐 `BUNDLED_SKILLS`,少一份就不算装好。
#[test]
fn the_bundled_skills_are_installed_and_removed_as_one_group() {
    let scratch = Scratch::new("group");
    let sources = skill_group(&scratch, "0.7.0");
    let target_dir = scratch.join("agent/skills");
    let neighbour = target_dir.join("other-skill/SKILL.md");
    write(&neighbour, "---\nname: other\n---\n");

    let installed = install_skill_group(&sources, &target_dir).expect("成组安装");
    assert_eq!(installed.len(), BUNDLED_SKILLS.len());
    assert!(BUNDLED_SKILLS.len() >= 2, "至少 ferry + ferry-resume");
    for name in BUNDLED_SKILLS {
        assert!(target_dir.join(name).join("SKILL.md").is_file(), "{name}");
    }

    let target = SkillTarget {
        id: SHARED_TARGET_ID.to_owned(),
        path: target_dir.clone(),
    };
    let status = skill_target_status(&target);
    assert!(status.installed);
    assert_eq!(status.installed_version.as_deref(), Some("0.7.0"));

    // 组里少一份就整体报「未安装」:半装状态不该显示成已安装。
    remove_skill_dir(&target_dir.join(BUNDLED_SKILLS[1])).expect("摘掉其中一份");
    let partial = skill_target_status(&target);
    assert!(!partial.installed);
    assert!(partial.installed_version.is_none());

    // 版本不齐时取最小值,让设置页显示「有新版本」而不是假装最新。
    install_skill_group(&skill_group(&scratch, "0.6.0"), &target_dir).expect("装一份旧的");
    install_skill_into(&skill_group(&scratch, "0.9.0")[0], &target_dir).expect("只更新第一份");
    assert_eq!(
        skill_target_status(&target).installed_version.as_deref(),
        Some("0.6.0")
    );

    // 移除只删 Ferry 自己装的那几个目录。
    remove_skill_group(&target_dir).expect("成组移除");
    for name in BUNDLED_SKILLS {
        assert!(!target_dir.join(name).exists(), "{name}");
    }
    assert!(neighbour.exists());
    // 已经不在了再删一次也不该报错(前端可能重复点)。
    remove_skill_group(&target_dir).expect("重复移除");
}

#[test]
fn uninstall_removes_only_the_ferry_directory() {
    let scratch = Scratch::new("uninstall");
    let source = skill_source(&scratch, "0.7.0");
    let target_dir = scratch.join("agent/skills");
    let neighbour = target_dir.join("other-skill/SKILL.md");
    write(&neighbour, "---\nname: other\n---\n");

    install_skill_into(&source, &target_dir).expect("安装");
    remove_skill_dir(&target_dir.join("ferry")).expect("卸载");
    assert!(!target_dir.join("ferry").exists());
    assert!(neighbour.exists());
    // 已经不在了再删一次也不该报错(前端可能重复点)。
    remove_skill_dir(&target_dir.join("ferry")).expect("重复卸载");
}

#[test]
fn a_target_without_the_skill_reports_no_version() {
    let scratch = Scratch::new("empty");
    let target = SkillTarget {
        id: SHARED_TARGET_ID.to_owned(),
        path: scratch.join("agents/skills"),
    };
    let status = skill_target_status(&target);
    assert!(!status.installed);
    assert!(status.installed_version.is_none());
}

/// 安装目标只有一个:契约里的共享技能目录。各 agent 都读它,不再按 agent 分行。
#[test]
fn the_only_target_is_the_shared_skill_directory() {
    let target = skill_target().expect("目标");
    assert_eq!(target.id, SHARED_TARGET_ID);
    assert!(target.path.is_absolute());
    assert!(target.path.ends_with(".agents/skills"));
    // 别的 id 一律拒绝:webview 只该拿到状态里回报的那一个。
    assert_eq!(
        find_target(SHARED_TARGET_ID).expect("按 id 找回").path,
        target.path
    );
    assert!(find_target("claude").is_err());
}

#[test]
fn only_a_leading_tilde_slash_expands() {
    assert_eq!(
        expand_home("/absolute/path").expect("绝对路径原样"),
        PathBuf::from("/absolute/path"),
    );
    let expanded = expand_home("~/.claude/skills").expect("展开 ~");
    assert!(expanded.is_absolute());
    assert!(expanded.ends_with(".claude/skills"));
}

#[test]
fn copying_a_tree_keeps_nested_files() {
    let scratch = Scratch::new("copy");
    let source = scratch.join("src");
    write(&source.join("SKILL.md"), "---\nversion: 1\n---\n");
    write(&source.join("references/notes.md"), "note");
    let destination = scratch.join("dst");
    copy_tree(&source, &destination).expect("复制");
    assert!(destination.join("SKILL.md").is_file());
    assert!(destination.join("references/notes.md").is_file());
}
