//! 规范化 JSON 与摘要。
//!
//! 三套摘要**不可混用**：
//! - `digest_value` / `digest_json`：sha256 小写 hex，**无前缀**（操作计划摘要）；
//! - `hash_bytes`：`"sha256:" + hex`（编辑 revision）；
//! - `stat_digest`：`"stat:" + sha256("{label}:{dev}:{ino}:{mtime_ns}:{size}")`。

use std::fmt::Write as _;

use serde_json::Value;
use sha2::{Digest, Sha256};

/// canonical_json 的失败原因；对齐 Python `json.dumps(allow_nan=False)`。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CanonicalJsonError {
    /// NaN / ±Infinity 不允许出现在 canonical JSON 里。
    NonFinite,
}

impl std::fmt::Display for CanonicalJsonError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NonFinite => {
                formatter.write_str("Out of range float values are not JSON compliant")
            }
        }
    }
}

impl std::error::Error for CanonicalJsonError {}

/// 摘要用的规范化序列化：同一份数据必须永远得到同一串字节。
///
/// 规则：key 递归排序（按 code point）、分隔符 `,` 与 `:` 无空格、
/// 非 ASCII 原样输出（不转义）、拒绝 NaN/Inf。
pub fn canonical_json(value: &Value) -> Result<String, CanonicalJsonError> {
    let mut out = String::new();
    write_value(&mut out, value)?;
    Ok(out)
}

fn write_value(out: &mut String, value: &Value) -> Result<(), CanonicalJsonError> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(number) => {
            // serde_json::Number 正常构造不出 NaN/Inf（from_f64 会返回 None），
            // 这里仍显式拦一道，语义与 Python allow_nan=False 对齐。
            if let Some(float) = number.as_f64() {
                if !float.is_finite() {
                    return Err(CanonicalJsonError::NonFinite);
                }
            }
            out.push_str(&number.to_string());
        }
        Value::String(text) => write_string(out, text),
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_value(out, item)?;
            }
            out.push(']');
        }
        Value::Object(entries) => {
            // 启用了 preserve_order，插入序不等于排序序，必须显式排序。
            let mut keys: Vec<&String> = entries.keys().collect();
            keys.sort_unstable();
            out.push('{');
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_string(out, key);
                out.push(':');
                write_value(out, &entries[key])?;
            }
            out.push('}');
        }
    }
    Ok(())
}

/// 与 Python `json.dumps(ensure_ascii=False)` 相同的字符串转义表：
/// 只转义 `"`、`\` 与 C0 控制字符（其中 5 个用短写），其余原样输出。
fn write_string(out: &mut String, text: &str) {
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
                let _ = write!(out, "\\u{:04x}", control as u32);
            }
            other => out.push(other),
        }
    }
    out.push('"');
}

/// sha256 小写 hex，无前缀。
pub fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    let mut out = String::with_capacity(64);
    for byte in digest {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// 对已经 canonical 化的 JSON 串取摘要（`digest_json`）。
pub fn digest_json(value_json: &str) -> String {
    sha256_hex(value_json.as_bytes())
}

/// 对任意 JSON 值取 canonical 摘要（`digest_value`）。
pub fn digest_value(value: &Value) -> Result<String, CanonicalJsonError> {
    Ok(digest_json(&canonical_json(value)?))
}

/// 编辑 revision 用的字节摘要：`"sha256:" + hex`。
pub fn hash_bytes(data: &[u8]) -> String {
    format!("sha256:{}", sha256_hex(data))
}

/// 文件身份的最小快照，等价 Python `os.stat_result` 里被用到的四个字段。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileStat {
    pub dev: u64,
    pub ino: u64,
    /// epoch 纳秒（对齐 `st_mtime_ns`，可为负）。
    pub mtime_ns: i128,
    pub size: u64,
}

impl FileStat {
    #[cfg(unix)]
    pub fn from_metadata(metadata: &std::fs::Metadata) -> Self {
        use std::os::unix::fs::MetadataExt;
        Self {
            dev: metadata.dev(),
            ino: metadata.ino(),
            mtime_ns: i128::from(metadata.mtime()) * 1_000_000_000
                + i128::from(metadata.mtime_nsec()),
            size: metadata.size(),
        }
    }
}

/// `st_dev` 的 Python 文本形式。
///
/// macOS 的 `dev_t` 是 **`int32_t`**（Linux 是无符号 64 位）。`MetadataExt::dev()`
/// 统一按 `as u64` 返回，负设备号会被抬成 `18446744073709551615` 之类的巨数，而
/// Python 的 `os.stat().st_dev` 打印的是 `-1`。stat_digest 的输入是纯文本拼接，
/// 这一位不同整条摘要就对不上，故按有符号格式化（正数不受影响）。
fn python_dev(dev: u64) -> i64 {
    dev as i64
}

/// 把文件 stat 折成稳定的修订标记：`"stat:" + sha256(...)`。
///
/// `label` 对应 Python 侧的 `f"{label}"`——调用方传路径时用 `Path` 的字符串形式。
pub fn stat_digest(label: &str, stat: &FileStat) -> String {
    let marker = format!(
        "{label}:{}:{}:{}:{}",
        python_dev(stat.dev),
        stat.ino,
        stat.mtime_ns,
        stat.size
    );
    format!("stat:{}", sha256_hex(marker.as_bytes()))
}

/// Python 的真值判定（`if value:` / `value or default`）。
///
/// 与 JSON 没有关系：`0`、`""`、`[]`、`{}`、`false`、`null` 都是假。
pub fn python_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|float| float != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(entries) => !entries.is_empty(),
    }
}

/// `str(value)` 的 Python 语义（只覆盖 JSON 标量）。
///
/// Python 把 `None` 写成 `None`、布尔写成 `True`/`False`，与 JSON 字面量不同；
/// 这些文本会进错误消息与落库字段，不能用 `Value::to_string()` 代替。
pub fn python_str(value: &Value) -> String {
    match value {
        Value::Null => "None".to_string(),
        Value::Bool(true) => "True".to_string(),
        Value::Bool(false) => "False".to_string(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 冻结的字节形状：任何差异都意味着已落盘的摘要整体失效。
    #[test]
    fn canonical_json_keeps_its_frozen_byte_shape() {
        let cases: &[(Value, &str)] = &[
            (json!({}), "{}"),
            (json!([]), "[]"),
            (json!(null), "null"),
            (json!(true), "true"),
            (json!(0), "0"),
            (json!(-1), "-1"),
            (json!(1234567890123456789i64), "1234567890123456789"),
            (json!(1.5), "1.5"),
            (json!("中文"), "\"中文\""),
            (json!({"b": 1, "a": 2}), r#"{"a":2,"b":1}"#),
            (
                json!({"会话": {"标题": "你好", "轮次": 3}, "a": [1, {"z": 0, "y": null}]}),
                r#"{"a":[1,{"y":null,"z":0}],"会话":{"标题":"你好","轮次":3}}"#,
            ),
            (
                json!({"emoji": "🚢", "tab": "a\tb", "quote": "\"q\"", "back": "a\\b"}),
                r#"{"back":"a\\b","emoji":"🚢","quote":"\"q\"","tab":"a\tb"}"#,
            ),
            (
                json!({"ctrl": "\u{1}\u{8}\u{c}\n\r\t\u{1f}"}),
                r#"{"ctrl":"\u0001\b\f\n\r\t\u001f"}"#,
            ),
            (
                json!({"Z": 1, "a": 2, "A": 3, "_": 4, "ä": 5}),
                r#"{"A":3,"Z":1,"_":4,"a":2,"ä":5}"#,
            ),
        ];
        for (value, expected) in cases {
            assert_eq!(&canonical_json(value).unwrap(), expected, "value={value}");
        }
    }

    /// 期望值同样来自 Python：`digest_value` = sha256(canonical_json) 的小写 hex。
    #[test]
    fn digests_match_python() {
        assert_eq!(
            digest_value(&json!({"a": 1, "b": "中文"})).unwrap(),
            "db1e1d174330db0f00974178407b16d090326f28edd3798338fe91c275dc5466"
        );
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hash_bytes(b"ferry"),
            "sha256:e9087d0b20d80d3e12bc8530d883d7ad9c1eb3ebc5cb61824a2b460816503797"
        );
    }

    #[test]
    fn stat_digest_uses_the_stat_prefix() {
        let stat = FileStat {
            dev: 1,
            ino: 2,
            mtime_ns: 3,
            size: 4,
        };
        assert_eq!(
            stat_digest("/tmp/a.jsonl", &stat),
            format!("stat:{}", sha256_hex(b"/tmp/a.jsonl:1:2:3:4"))
        );
    }

    #[test]
    fn stat_digest_prints_negative_dev_like_python() {
        // macOS 的 dev_t 是 int32_t：`MetadataExt::dev()` 把 -1 抬成 u64::MAX，
        // Python 的 st_dev 仍然打印 -1。
        let stat = FileStat {
            dev: u64::MAX,
            ino: 2,
            mtime_ns: 3,
            size: 4,
        };
        assert_eq!(
            stat_digest("/tmp/a.jsonl", &stat),
            format!("stat:{}", sha256_hex(b"/tmp/a.jsonl:-1:2:3:4"))
        );
    }

    #[test]
    fn python_str_matches_the_interpreter_for_scalars() {
        assert_eq!(python_str(&json!(null)), "None");
        assert_eq!(python_str(&json!(true)), "True");
        assert_eq!(python_str(&json!(false)), "False");
        assert_eq!(python_str(&json!("x")), "x");
        assert_eq!(python_str(&json!(3)), "3");
        assert_eq!(python_str(&json!(1.5)), "1.5");
    }

    #[test]
    fn revision_shape_matches_python_tuple_serialization() {
        // index.py:30-47 的 revision：file_identity 的嵌套 tuple 序列化为 JSON 数组。
        let payload = json!({
            "tool": "claude",
            "ref": "/tmp/a.jsonl",
            "updated": 1,
            "size": 2,
            "file_identity": [["a.jsonl", ["sha256", "deadbeef"]]],
        });
        assert_eq!(
            canonical_json(&payload).unwrap(),
            r#"{"file_identity":[["a.jsonl",["sha256","deadbeef"]]],"ref":"/tmp/a.jsonl","size":2,"tool":"claude","updated":1}"#
        );
    }
}
