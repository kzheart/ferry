//! 目标会话文件的落盘原语，以及与 Python `json.dumps` 逐字节一致的序列化。
//!
//! 语义事实源：`engine/adapters/shared/writing.py`。
//!
//! 注意本模块的 [`write_jsonl`] 与 [`super::editing::write_jsonl`] 是**两套**实现：
//! 迁移写入面向"目标 Agent 可能正在扫描该目录"，必须建父目录 + fsync；
//! 就地编辑写入是同目录带 pid 的临时文件、不 fsync。两者的临时文件命名、
//! 落盘时机都被 golden 基线逐字节比对，**不可合并**。

use std::fs::{self, File};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// 原子写 JSONL：同目录临时文件写完并 fsync 后再 replace 到目标。
///
/// 目标 Agent 可能正在扫描该目录，半截文件会被当成损坏会话。
pub fn write_jsonl(path: &Path, rows: &[Value]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    // Python `path.with_suffix(path.suffix + ".tmp")` 等价于整体追加 ".tmp"：
    // with_suffix 用「原后缀 + .tmp」替换原后缀，结果就是原路径串接 ".tmp"。
    let temporary = temp_path(path);
    {
        let mut stream = File::create(&temporary)?;
        for row in rows {
            stream.write_all(python_json_dumps(row).as_bytes())?;
            stream.write_all(b"\n")?;
        }
        stream.flush()?;
        stream.sync_all()?;
    }
    fs::rename(&temporary, path)
}

fn temp_path(path: &Path) -> PathBuf {
    let mut raw = path.as_os_str().to_os_string();
    raw.push(".tmp");
    PathBuf::from(raw)
}

/// 等价 `json.dumps(value, ensure_ascii=False)`：**默认分隔符带空格**（`, ` 与 `: `）。
///
/// 这不是 canonical_json（那套排序 key 且无空格，用于摘要）。写进目标 Agent
/// 会话文件的每一行都必须用这套形状，否则与 Python 引擎产出的文件不同字节。
pub fn python_json_dumps(value: &Value) -> String {
    let mut out = String::new();
    write_compact(&mut out, value);
    out
}

/// 等价 `json.dumps(value, ensure_ascii=False, indent=n)`。
///
/// 缩进模式下 Python 的分隔符退化为 `,`（后跟换行）与 `: `。
pub fn python_json_dumps_indented(value: &Value, indent: usize) -> String {
    let mut out = String::new();
    write_indented(&mut out, value, indent, 0);
    out
}

fn write_compact(out: &mut String, value: &Value) {
    match value {
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_compact(out, item);
            }
            out.push(']');
        }
        Value::Object(entries) => {
            out.push('{');
            for (index, (key, item)) in entries.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_json_string(out, key);
                out.push_str(": ");
                write_compact(out, item);
            }
            out.push('}');
        }
        scalar => write_scalar(out, scalar),
    }
}

fn write_indented(out: &mut String, value: &Value, indent: usize, level: usize) {
    match value {
        Value::Array(items) if !items.is_empty() => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                push_newline_indent(out, indent, level + 1);
                write_indented(out, item, indent, level + 1);
            }
            push_newline_indent(out, indent, level);
            out.push(']');
        }
        Value::Object(entries) if !entries.is_empty() => {
            out.push('{');
            for (index, (key, item)) in entries.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                push_newline_indent(out, indent, level + 1);
                write_json_string(out, key);
                out.push_str(": ");
                write_indented(out, item, indent, level + 1);
            }
            push_newline_indent(out, indent, level);
            out.push('}');
        }
        // 空容器在 Python 里不换行：`json.dumps({"a": {}}, indent=2)` -> `{\n  "a": {}\n}`
        Value::Array(_) => out.push_str("[]"),
        Value::Object(_) => out.push_str("{}"),
        scalar => write_scalar(out, scalar),
    }
}

fn push_newline_indent(out: &mut String, indent: usize, level: usize) {
    out.push('\n');
    for _ in 0..indent * level {
        out.push(' ');
    }
}

fn write_scalar(out: &mut String, value: &Value) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(number) => out.push_str(&number.to_string()),
        Value::String(text) => write_json_string(out, text),
        _ => unreachable!("容器由调用方处理"),
    }
}

/// `ensure_ascii=False` 的转义表：只转义 `"`、`\` 与 C0 控制字符。
///
/// 与 `jsonutil::canonical_json` 内部的转义规则一致（那份是私有的，不能复用）。
fn write_json_string(out: &mut String, text: &str) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            control if control < ' ' => {
                out.push_str(&format!("\\u{:04x}", control as u32));
            }
            other => out.push(other),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 期望值取自 `python3 -c 'import json; print(json.dumps(...))'`。
    #[test]
    fn compact_dumps_keeps_pythons_spaced_separators() {
        assert_eq!(
            python_json_dumps(&json!({"a": 1, "b": [1, 2], "c": "中文"})),
            r#"{"a": 1, "b": [1, 2], "c": "中文"}"#
        );
        assert_eq!(python_json_dumps(&json!({})), "{}");
        assert_eq!(python_json_dumps(&json!([])), "[]");
        assert_eq!(python_json_dumps(&json!(null)), "null");
        assert_eq!(python_json_dumps(&json!(5.0)), "5.0");
        assert_eq!(python_json_dumps(&json!("a\tb\u{1}")), "\"a\\tb\\u0001\"");
    }

    #[test]
    fn indented_dumps_matches_python_indent_two() {
        assert_eq!(
            python_json_dumps_indented(&json!({"a": {"b": 1}, "c": []}), 2),
            "{\n  \"a\": {\n    \"b\": 1\n  },\n  \"c\": []\n}"
        );
        assert_eq!(
            python_json_dumps_indented(&json!([1, 2]), 2),
            "[\n  1,\n  2\n]"
        );
        assert_eq!(python_json_dumps_indented(&json!("x"), 2), "\"x\"");
    }

    #[test]
    fn migration_write_jsonl_is_atomic_and_creates_parents() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("nested/dir/session.jsonl");
        write_jsonl(&path, &[json!({"a": 1}), json!({"b": "中"})]).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "{\"a\": 1}\n{\"b\": \"中\"}\n"
        );
        // 临时文件必须已被 rename 掉。
        assert!(!path.with_extension("jsonl.tmp").exists());
    }

    #[test]
    fn empty_rows_produce_an_empty_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("empty.jsonl");
        write_jsonl(&path, &[]).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "");
    }
}
