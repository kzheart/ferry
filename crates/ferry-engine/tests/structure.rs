//! 分层规则守卫（替代 `scripts/check-engine-layering.py`）。
//!
//! `adapters` 不得引用 `operations` 与 `sessions`：Python 现状里存在
//! adapters → sessions.usage / sessions.topology 的倒置依赖，Rust 侧把这些共享
//! 助手放进 `adapters::shared`，由 sessions 复用，方向反转。

use std::path::{Path, PathBuf};

fn rust_sources(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(rust_sources(&path));
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
    files
}

#[test]
fn adapters_do_not_depend_on_operations_or_sessions() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/adapters");
    let mut offenders = Vec::new();
    for path in rust_sources(&root) {
        let source = std::fs::read_to_string(&path).expect("源文件可读");
        // 注释里可以提到这两个包（分层规则本身就写在 adapters/mod.rs 的文档里）。
        let code: String = source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for forbidden in ["crate::operations", "crate::sessions"] {
            if code.contains(forbidden) {
                offenders.push(format!("{} -> {forbidden}", path.display()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "adapters 层出现倒置依赖:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn every_module_file_is_declared() {
    // mod 树在 WP-A 一次性定型；漏声明的文件不会被编译，靠本测试暴露。
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for path in rust_sources(&src) {
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        if ["lib", "main", "mod"].contains(&stem.as_str()) {
            continue;
        }
        let parent = path.parent().unwrap();
        let declaring = if parent == src {
            src.join("lib.rs")
        } else {
            parent.join("mod.rs")
        };
        let source = std::fs::read_to_string(&declaring)
            .unwrap_or_else(|_| panic!("缺少 mod 声明文件: {}", declaring.display()));
        assert!(
            source.contains(&format!("pub mod {stem};")),
            "{} 未在 {} 里声明",
            path.display(),
            declaring.display()
        );
    }
}
