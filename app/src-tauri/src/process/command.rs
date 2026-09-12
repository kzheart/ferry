use std::path::{Path, PathBuf};
use std::process::Command;

fn executable_name_for(stem: &str, windows: bool) -> String {
    if windows {
        format!("{stem}.exe")
    } else {
        stem.to_owned()
    }
}

#[cfg(not(debug_assertions))]
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

#[cfg(not(debug_assertions))]
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

/// 问一个引擎产物它编进去的契约哈希：跑一次 `rpc health`，读 `result.contract_hash`。
/// 起不来、答非所问都算「未知」，交给选择逻辑当作不匹配处理。
#[cfg(debug_assertions)]
pub(crate) fn probe_engine_contract_hash(path: &Path) -> Option<String> {
    let request = format!(
        r#"{{"protocol":"{}","id":"probe","method":"health","params":{{}}}}"#,
        crate::contracts::ipc::FERRY_IPC_PROTOCOL
    );
    let output = Command::new(path)
        .args(["rpc", &request])
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    value
        .pointer("/result/contract_hash")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

/// 在若干候选里挑**契约哈希与本进程一致**且最新构建的那个。
///
/// 按 mtime 挑最新是必要的（刚 `cargo build` 出的产物必须立即生效），但只按
/// mtime 会选中契约已过期的旧产物，握手失败的原因还被折成一句固定文案。
/// 先用 `probe` 过滤掉哈希不一致的候选，再取最新；`probe` 抽成参数便于单测。
#[cfg(debug_assertions)]
pub(crate) fn select_matching_engine(
    candidates: &[PathBuf],
    expected_hash: &str,
    probe: &dyn Fn(&Path) -> Option<String>,
) -> Result<PathBuf, Vec<(PathBuf, Option<String>)>> {
    let mut mismatched: Vec<(PathBuf, Option<String>)> = Vec::new();
    let mut matching: Vec<PathBuf> = Vec::new();
    for path in candidates.iter().filter(|path| path.is_file()) {
        let hash = probe(path);
        if hash.as_deref() == Some(expected_hash) {
            matching.push(path.clone());
        } else {
            mismatched.push((path.clone(), hash));
        }
    }
    matching
        .into_iter()
        .max_by_key(|path| {
            std::fs::metadata(path)
                .and_then(|meta| meta.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        })
        .ok_or(mismatched)
}

/// 开发模式当前会被选中的引擎产物路径（给设置页展示用）；选不到返回 `None`。
#[cfg(debug_assertions)]
pub(crate) fn local_engine_path() -> Option<PathBuf> {
    let candidates = local_engine_candidates(&repository_root(), cfg!(target_os = "windows"));
    select_matching_engine(
        &candidates,
        crate::contracts::ipc::FERRY_CONTRACT_HASH,
        &probe_engine_contract_hash,
    )
    .ok()
}

/// 开发模式的引擎入口：只认仓库内构建出来的引擎产物，且契约哈希必须与本进程一致。
///
/// **不**看 `target/debug/` 下 tauri 复制进来的打包 sidecar：那是 `binaries/`
/// 里某次发版构建的快照，与当前源码无关，恰恰是过去「握手失败」的来源。
#[cfg(debug_assertions)]
pub(crate) fn local_engine_command() -> Result<Command, String> {
    let root = repository_root();
    let candidates = local_engine_candidates(&root, cfg!(target_os = "windows"));
    match select_matching_engine(
        &candidates,
        crate::contracts::ipc::FERRY_CONTRACT_HASH,
        &probe_engine_contract_hash,
    ) {
        Ok(path) => {
            crate::process::logging::host_log(
                "engine",
                &format!("开发模式选用引擎产物: {}", path.display()),
            );
            Ok(Command::new(path))
        }
        Err(mismatched) => Err(missing_local_engine_message(
            &root,
            &candidates,
            &mismatched,
        )),
    }
}

/// 开发模式选不到可用引擎时的报错：给出构建命令、找过的位置与各产物的契约哈希。
#[cfg(debug_assertions)]
pub(crate) fn missing_local_engine_message(
    root: &Path,
    candidates: &[PathBuf],
    mismatched: &[(PathBuf, Option<String>)],
) -> String {
    let tried = candidates
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join("; ");
    let stale = mismatched
        .iter()
        .map(|(path, hash)| {
            format!(
                "{} -> {}",
                path.display(),
                hash.as_deref().unwrap_or("无法读取契约哈希")
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!(
        "没有与当前源码契约一致的 Session Engine 产物,先构建:\n  \
         npm --prefix {root}/app run engine:build\n（等价于 cargo build --release --manifest-path \
         {root}/crates/ferry-engine/Cargo.toml）\n已尝试: {tried}\n契约不一致的产物: {stale}",
        root = root.display(),
        stale = if stale.is_empty() {
            "无".to_string()
        } else {
            stale
        },
    )
}

/// 开发模式的 runtime 入口：直接跑 `ferry-runtime/dist`（`npm run runtime:build` 的产物），
/// 同样不看 tauri 复制进 `target/debug/` 的打包 sidecar 快照。
#[cfg(debug_assertions)]
pub(crate) fn local_runtime_command() -> Result<Command, String> {
    let root = repository_root();
    let entry = root.join("ferry-runtime/dist/server/server.js");
    if !entry.is_file() {
        return Err(format!(
            "未找到 Ferry Runtime 构建产物 {}，先运行: npm --prefix {}/app run runtime:build",
            entry.display(),
            root.display()
        ));
    }
    let mut command = Command::new("node");
    command.arg(entry);
    command.current_dir(root);
    Ok(command)
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
    fn development_picks_the_newest_engine_whose_contract_hash_matches() {
        use super::select_matching_engine;
        use std::path::{Path, PathBuf};

        let temp_dir = std::env::temp_dir().join(format!(
            "ferry-engine-select-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let stale_new = temp_dir.join("debug-engine");
        let fresh_old = temp_dir.join("release-engine");
        let unreadable = temp_dir.join("broken-engine");
        std::fs::write(&fresh_old, b"old").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&stale_new, b"new").unwrap();
        std::fs::write(&unreadable, b"x").unwrap();
        let probe = |path: &Path| -> Option<String> {
            match path.file_name()?.to_str()? {
                "debug-engine" => Some("hash-old".into()),
                "release-engine" => Some("hash-now".into()),
                _ => None,
            }
        };
        let candidates: Vec<PathBuf> = vec![
            stale_new.clone(),
            fresh_old.clone(),
            unreadable.clone(),
            temp_dir.join("missing"),
        ];
        // 更新的产物契约过期，不能因为「最新」被选中。
        assert_eq!(
            select_matching_engine(&candidates, "hash-now", &probe).unwrap(),
            fresh_old
        );
        let rejected = select_matching_engine(&candidates, "hash-future", &probe).unwrap_err();
        assert_eq!(rejected.len(), 3, "缺失文件不计入，其余全部列为不一致");
        assert!(rejected
            .iter()
            .any(|(path, hash)| path == &stale_new && hash.as_deref() == Some("hash-old")));
        assert!(rejected
            .iter()
            .any(|(path, hash)| path == &unreadable && hash.is_none()));
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

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
