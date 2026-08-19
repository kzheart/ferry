use std::path::{Path, PathBuf};
use std::process::Command;

fn executable_name_for(stem: &str, windows: bool) -> String {
    if windows {
        format!("{stem}.exe")
    } else {
        stem.to_owned()
    }
}

pub(crate) fn sidecar_candidates(resource_dir: &Path, stem: &str) -> Vec<PathBuf> {
    let name = executable_name_for(stem, cfg!(target_os = "windows"));
    let mut candidates = Vec::new();
    if let Some(executable_dir) = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
    {
        candidates.push(executable_dir.join(&name));
    }
    candidates.push(resource_dir.join(name));
    candidates
}

pub(crate) fn bundled_sidecar_command(
    resource_dir: &Path,
    stem: &str,
) -> (Option<Command>, Vec<PathBuf>) {
    let candidates = sidecar_candidates(resource_dir, stem);
    let command = candidates
        .iter()
        .find(|path| path.is_file())
        .map(Command::new);
    (command, candidates)
}

#[cfg(not(debug_assertions))]
pub(crate) fn missing_sidecar_message(label: &str, candidates: &[PathBuf]) -> String {
    format!(
        "正式包缺少 {label} sidecar,已尝试: {}",
        candidates
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("; ")
    )
}

#[cfg(debug_assertions)]
pub(crate) fn repository_root() -> PathBuf {
    if let Ok(path) = std::env::var("FERRY_REPO") {
        return PathBuf::from(path);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from("."))
}

/// 开发模式下的原生引擎产物：`crates/ferry-engine` 是独立 package，
/// 产物落在自己的 `target/<profile>/` 下（含 `--target <triple>` 的变体）。
/// 选择交给 [`local_engine_command`] 按最新 mtime 决定，这里只列候选。
#[cfg(debug_assertions)]
pub(crate) fn local_engine_candidates(root: &Path, windows: bool) -> Vec<PathBuf> {
    let name = executable_name_for("ferry-engine", windows);
    let target = root.join("crates/ferry-engine/target");
    let mut candidates = vec![
        target.join("debug").join(&name),
        target.join("release").join(&name),
    ];
    // `cargo build --release --target <triple>`（构建脚本用的形态）落在三层目录。
    if let Ok(entries) = std::fs::read_dir(&target) {
        for entry in entries.flatten() {
            let path = entry.path().join("release").join(&name);
            if entry.file_name().to_string_lossy().contains('-') && path.is_file() {
                candidates.push(path);
            }
        }
    }
    candidates
}

/// 开发模式的引擎入口：只认仓库内构建出来的引擎产物，没有就报错。
///
/// 多个产物并存时取**最新构建**的那个：debug 引擎对真实会话库做全量 sha256
/// 规范化会慢一个数量级（首扫可超过 host 的 120s 超时），偶尔构建的 release
/// 不该被恒定压在 debug 之后；而引擎开发者刚 `cargo build` 出的 debug 产物
/// 又必须立即生效。按 mtime 取最新即可两全。
#[cfg(debug_assertions)]
pub(crate) fn local_engine_command() -> Option<Command> {
    local_engine_candidates(&repository_root(), cfg!(target_os = "windows"))
        .into_iter()
        .filter(|path| path.is_file())
        .max_by_key(|path| {
            std::fs::metadata(path)
                .and_then(|meta| meta.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        })
        .map(Command::new)
}

/// 开发模式找不到引擎产物时的报错：直接给出构建命令与找过的位置。
#[cfg(debug_assertions)]
pub(crate) fn missing_local_engine_message() -> String {
    let root = repository_root();
    let tried = local_engine_candidates(&root, cfg!(target_os = "windows"))
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join("; ");
    format!(
        "未找到 Session Engine 产物,先构建:\n  \
         cargo build --manifest-path {}/crates/ferry-engine/Cargo.toml\n已尝试: {tried}",
        root.display()
    )
}

/// Sidecar 是后台进程；平台边界统一决定是否隐藏控制台窗口。
#[cfg(target_os = "windows")]
pub(crate) fn configure_background(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x0800_0000);
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn configure_background(_command: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::executable_name_for;

    #[cfg(debug_assertions)]
    #[test]
    fn development_looks_for_the_engine_build_in_both_profiles() {
        use super::local_engine_candidates;
        use std::path::{Path, PathBuf};

        // 固定的两个 profile 候选必在（`--target <triple>` 变体按磁盘现状追加，
        // 用 /repo 这种不存在的根时不会出现）。
        let candidates = local_engine_candidates(Path::new("/repo"), false);
        assert_eq!(
            candidates,
            vec![
                PathBuf::from("/repo/crates/ferry-engine/target/debug/ferry-engine"),
                PathBuf::from("/repo/crates/ferry-engine/target/release/ferry-engine"),
            ],
        );
        assert_eq!(
            local_engine_candidates(Path::new("/repo"), true)[0],
            PathBuf::from("/repo/crates/ferry-engine/target/debug/ferry-engine.exe"),
        );
    }

    #[test]
    fn sidecar_names_keep_the_windows_executable_boundary() {
        assert_eq!(
            executable_name_for("ferry-engine", true),
            "ferry-engine.exe",
        );
        assert_eq!(
            executable_name_for("ferry-runtime", true),
            "ferry-runtime.exe",
        );
        assert_eq!(executable_name_for("ferry-runtime", false), "ferry-runtime",);
    }
}
