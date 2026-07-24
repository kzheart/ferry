use std::path::{Path, PathBuf};
use std::process::Command;

fn executable_name_for(stem: &str, windows: bool) -> String {
    if windows {
        format!("{stem}.exe")
    } else {
        stem.to_owned()
    }
}

#[cfg(debug_assertions)]
pub(crate) fn python_program() -> &'static str {
    if cfg!(target_os = "windows") {
        "python"
    } else {
        "python3"
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
