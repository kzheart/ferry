//! 运行时只读资源定位。
//!
//! 语义事实源：`engine/system/resources.py`。Python 侧要兼容 PyInstaller 的
//! `sys._MEIPASS`；Rust sidecar 是单文件可执行，资源根取「可执行文件所在目录」，
//! 并保留 `FERRY_RESOURCE_ROOT` 覆盖以便开发期指向仓库根。

use std::path::{Path, PathBuf};

pub fn resource_root() -> PathBuf {
    if let Ok(override_root) = std::env::var("FERRY_RESOURCE_ROOT") {
        if !override_root.is_empty() {
            return PathBuf::from(override_root);
        }
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn resource_path(parts: &[&str]) -> PathBuf {
    let mut path = resource_root();
    for part in parts {
        path.push(part);
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_path_joins_under_the_root() {
        let root = resource_root();
        assert_eq!(resource_path(&["a", "b"]), root.join("a").join("b"));
    }
}
