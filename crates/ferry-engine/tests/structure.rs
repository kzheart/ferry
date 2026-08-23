//! 源码级结构守卫：分层方向、mod 声明完整性、损耗目录闭环。
//!
//! `adapters` 不得引用 `operations` 与 `sessions`：共享助手放进
//! `adapters::shared`，由 sessions 复用，依赖方向单向向下。
//!
//! 损耗目录闭环是**源码形态**约束，编译器管不到，只能靠本文件扫描源码守住。

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

// ---------------------------------------------------------------------------
// 通用源码扫描助手
// ---------------------------------------------------------------------------

fn source_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// 跳过字符串字面量与行注释的花括号配对，返回 `header` 之后那对花括号里的内容。
fn braced_body(source: &str, header: &str) -> Option<String> {
    let start = source.find(header)? + header.len();
    let bytes = source.as_bytes();
    let mut depth = 1usize;
    let mut index = start;
    while index < bytes.len() {
        match bytes[index] {
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'"' => {
                index += 1;
                while index < bytes.len() && bytes[index] != b'"' {
                    index += if bytes[index] == b'\\' { 2 } else { 1 };
                }
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(source[start..index].to_string());
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

// ---------------------------------------------------------------------------
// 损耗目录闭环
// ---------------------------------------------------------------------------

/// 剔除 `#[cfg(test)] mod ... { ... }`：测试里的桩 code 不是产品语义。
fn strip_test_modules(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(index) = rest.find("#[cfg(test)]") {
        let (head, tail) = rest.split_at(index);
        out.push_str(head);
        // 只吞掉 `#[cfg(test)]` 后面紧跟的 mod 块；作用在 use / fn 上的照常保留。
        match tail
            .find("mod ")
            .filter(|position| !tail[..*position].contains(';'))
            .and_then(|_| braced_body(tail, "{"))
        {
            Some(body) => rest = &tail[tail.find('{').unwrap() + body.len() + 2..],
            None => {
                out.push_str(&tail[.."#[cfg(test)]".len()]);
                rest = &tail["#[cfg(test)]".len()..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// 抽取 `session.lose("code", ...)` / `losing(session, "code", ...)` 里的 code。
fn produced_loss_codes(source: &str) -> Vec<String> {
    let bytes = source.as_bytes();
    let mut codes = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] != b'(' {
            index += 1;
            continue;
        }
        let head = &source[..index];
        let free_standing = head.ends_with("losing")
            && !head[..head.len() - "losing".len()]
                .ends_with(|character: char| character.is_ascii_alphanumeric() || character == '_');
        if !(head.ends_with(".lose") || free_standing) {
            index += 1;
            continue;
        }
        // 括号配对确定实参范围，再取范围内第一个形如 loss code 的字面量。
        let Some(body) = braced_paren(source, index + 1) else {
            index += 1;
            continue;
        };
        if let Some(code) = first_loss_literal(&body) {
            codes.push(code);
        }
        index += 1;
    }
    codes
}

/// 从 `start`（左括号之后）开始配对，返回括号内的实参文本。
fn braced_paren(source: &str, start: usize) -> Option<String> {
    let bytes = source.as_bytes();
    let mut depth = 1usize;
    let mut index = start;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                index += 1;
                while index < bytes.len() && bytes[index] != b'"' {
                    index += if bytes[index] == b'\\' { 2 } else { 1 };
                }
            }
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(source[start..index].to_string());
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn first_loss_literal(arguments: &str) -> Option<String> {
    let mut rest = arguments;
    while let Some(open) = rest.find('"') {
        let tail = &rest[open + 1..];
        let close = tail.find('"')?;
        let literal = &tail[..close];
        let shaped = literal.contains('.')
            && literal.starts_with(|character: char| character.is_ascii_lowercase())
            && literal.chars().all(|character| {
                character.is_ascii_lowercase() || character == '.' || character == '_'
            });
        if shaped {
            return Some(literal.to_string());
        }
        rest = &tail[close + 1..];
    }
    None
}

#[test]
fn every_produced_loss_code_is_declared_by_its_owner() {
    // 读取期告警，不构成迁移差异，因此刻意不声明后果。
    const INFORMATIONAL: &[&str] = &[
        "migration.tool_degraded", // 由 RenderDecision 单独统计
        "session.malformed_record",
        "session.orphan_tool_result",
        "session.subagent_unlinked",
    ];
    // 声明是 `build()` 的副作用，必须先装配再查表。
    ferry_engine::adapters::registry::create_registry().expect("内置 adapter 可装配");

    let mut undeclared = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for path in rust_sources(&source_root()) {
        let source = strip_test_modules(&std::fs::read_to_string(&path).expect("源文件可读"));
        for code in produced_loss_codes(&source) {
            seen.insert(code.clone());
            if INFORMATIONAL.contains(&code.as_str()) {
                continue;
            }
            if ferry_engine::loss::outcome_for_code(&code).is_none() {
                undeclared.push(format!("{}: {code}", path.display()));
            }
        }
    }
    // 扫描器一旦失灵就会静默变成空跑，这里给一条下限。
    assert!(seen.len() >= 10, "扫描到的 loss code 太少: {seen:?}");
    undeclared.sort();
    undeclared.dedup();
    assert!(
        undeclared.is_empty(),
        "这些 loss code 产出了却没人声明后果:\n{}",
        undeclared.join("\n")
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
        // 平台边界模块（`socket/platform/{unix,windows,unsupported}.rs`）是
        // cfg 门控的私有 mod，只对边界文件可见；这里要守的是「文件必须被某个
        // mod 树声明」，不是「必须 pub」。
        assert!(
            source.contains(&format!("mod {stem};")),
            "{} 未在 {} 里声明",
            path.display(),
            declaring.display()
        );
    }
}
