//! AI 重置标题的用户风格设置，存 `<state_dir>/title-style.json`。
//!
//! 校验在写入前一次做完并返回**归一化后**的 style：文件缺失、损坏、字段缺失都
//! 回落默认值，读取路径不报错——风格只影响提示词，不该让整条生成链路失败。

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use crate::errors::DomainError;
use crate::operations::types::EngineResult;

const FILE_NAME: &str = "title-style.json";

pub const PRESETS: &[&str] = &["ferry", "english", "bracket", "custom"];
pub const LANGUAGES: &[&str] = &["follow", "zh", "en"];

const MAX_CHARS_MIN: i64 = 6;
const MAX_CHARS_MAX: i64 = 60;
const MAX_TYPES: usize = 16;
const MAX_EMOJI_CHARS: usize = 4;
const MAX_TYPE_NAME_CHARS: usize = 12;
const MAX_INSTRUCTIONS_CHARS: usize = 2000;
const MAX_EXAMPLES: usize = 3;
const MAX_EXAMPLE_CHARS: usize = 120;

/// 默认类型表（方案 D）。
const DEFAULT_TYPES: &[(&str, &str, &str)] = &[
    ("✨", "实现", "feat"),
    ("🐛", "修复", "fix"),
    ("♻️", "重构", "refactor"),
    ("🔍", "调研", "research"),
    ("🧪", "评估", "eval"),
    ("🩺", "排查", "debug"),
    ("⚙️", "配置", "config"),
    ("🚀", "发布", "release"),
    ("📖", "梳理", "docs"),
    ("💬", "讨论", "discuss"),
];

fn invalid(message: impl Into<String>) -> DomainError {
    DomainError::agent_request_invalid(message)
}

pub fn default_types() -> Value {
    Value::Array(
        DEFAULT_TYPES
            .iter()
            .map(|(emoji, zh, en)| json!({"emoji": emoji, "zh": zh, "en": en}))
            .collect(),
    )
}

/// 规格里的默认 style。
pub fn default_style() -> Value {
    json!({
        "preset": "ferry",
        "language": "follow",
        "max_chars": 16,
        "type_prefix": true,
        "types": default_types(),
        "instructions": "",
        "examples": [],
    })
}

fn path_of(state_dir: impl AsRef<Path>) -> PathBuf {
    state_dir.as_ref().join(FILE_NAME)
}

/// 读盘；文件不存在 / 不是合法 JSON / 校验不过一律回默认值。
pub fn get(state_dir: impl AsRef<Path>) -> Value {
    let Ok(text) = std::fs::read_to_string(path_of(state_dir)) else {
        return default_style();
    };
    serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|value| normalize(&value).ok())
        .unwrap_or_else(default_style)
}

/// 校验并落盘，返回归一化后的 style。
pub fn set(style: &Value, state_dir: impl AsRef<Path>) -> EngineResult<Value> {
    let normalized = normalize(style)?;
    let path = path_of(state_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| invalid(format!("无法创建 state 目录: {error}")))?;
    }
    let text = serde_json::to_string_pretty(&normalized).unwrap_or_else(|_| normalized.to_string());
    std::fs::write(&path, text).map_err(|error| invalid(format!("写入标题风格失败: {error}")))?;
    Ok(normalized)
}

fn enum_field(object: &Map<String, Value>, key: &str, allowed: &[&str]) -> EngineResult<String> {
    let value = match object.get(key) {
        None | Some(Value::Null) => return Ok(allowed[0].to_string()),
        Some(value) => value,
    };
    let text = value
        .as_str()
        .filter(|text| allowed.contains(text))
        .ok_or_else(|| invalid(format!("style.{key} 必须是 {} 之一", allowed.join("/"))))?;
    Ok(text.to_string())
}

fn bounded_name(value: Option<&Value>, key: &str, max: usize) -> EngineResult<String> {
    let text = value
        .and_then(Value::as_str)
        .map(str::trim)
        .ok_or_else(|| invalid(format!("style.types[].{key} 必须是字符串")))?;
    let length = text.chars().count();
    if length == 0 || length > max {
        return Err(invalid(format!("style.types[].{key} 长度须在 1..{max}")).into());
    }
    Ok(text.to_string())
}

/// 校验 + 补默认；未知字段直接丢弃，输出恒为规格里的七个键。
pub fn normalize(style: &Value) -> EngineResult<Value> {
    let object = style
        .as_object()
        .ok_or_else(|| invalid("style 必须是 object"))?;

    let preset = enum_field(object, "preset", PRESETS)?;
    let language = enum_field(object, "language", LANGUAGES)?;

    let max_chars = match object.get("max_chars") {
        None | Some(Value::Null) => 16,
        Some(value) => value
            .as_i64()
            .filter(|number| (MAX_CHARS_MIN..=MAX_CHARS_MAX).contains(number))
            .ok_or_else(|| {
                invalid(format!(
                    "style.max_chars 须是 {MAX_CHARS_MIN}..{MAX_CHARS_MAX} 的整数"
                ))
            })?,
    };

    let type_prefix = match object.get("type_prefix") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(flag)) => *flag,
        Some(_) => return Err(invalid("style.type_prefix 必须是布尔值").into()),
    };

    let types = match object.get("types") {
        None | Some(Value::Null) => default_types(),
        Some(Value::Array(items)) => {
            if items.is_empty() || items.len() > MAX_TYPES {
                return Err(invalid(format!("style.types 长度须在 1..{MAX_TYPES}")).into());
            }
            let mut normalized = Vec::with_capacity(items.len());
            for item in items {
                let entry = item
                    .as_object()
                    .ok_or_else(|| invalid("style.types[] 必须是 object"))?;
                let emoji = bounded_name(entry.get("emoji"), "emoji", MAX_EMOJI_CHARS)?;
                let zh = bounded_name(entry.get("zh"), "zh", MAX_TYPE_NAME_CHARS)?;
                let en = bounded_name(entry.get("en"), "en", MAX_TYPE_NAME_CHARS)?;
                normalized.push(json!({"emoji": emoji, "zh": zh, "en": en}));
            }
            Value::Array(normalized)
        }
        Some(_) => return Err(invalid("style.types 必须是数组").into()),
    };

    let instructions = match object.get("instructions") {
        None | Some(Value::Null) => String::new(),
        Some(value) => {
            let text = value
                .as_str()
                .ok_or_else(|| invalid("style.instructions 必须是字符串"))?
                .trim()
                .to_string();
            if text.chars().count() > MAX_INSTRUCTIONS_CHARS {
                return Err(invalid(format!(
                    "style.instructions 超过 {MAX_INSTRUCTIONS_CHARS} 字符"
                ))
                .into());
            }
            text
        }
    };

    let examples = match object.get("examples") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => {
            if items.len() > MAX_EXAMPLES {
                return Err(invalid(format!("style.examples 最多 {MAX_EXAMPLES} 条")).into());
            }
            let mut normalized = Vec::with_capacity(items.len());
            for item in items {
                let text = item
                    .as_str()
                    .ok_or_else(|| invalid("style.examples[] 必须是字符串"))?
                    .trim()
                    .to_string();
                if text.is_empty() || text.chars().count() > MAX_EXAMPLE_CHARS {
                    return Err(invalid(format!(
                        "style.examples[] 长度须在 1..{MAX_EXAMPLE_CHARS}"
                    ))
                    .into());
                }
                normalized.push(Value::from(text));
            }
            normalized
        }
        Some(_) => return Err(invalid("style.examples 必须是数组").into()),
    };

    Ok(json!({
        "preset": preset,
        "language": language,
        "max_chars": max_chars,
        "type_prefix": type_prefix,
        "types": types,
        "instructions": instructions,
        "examples": examples,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_object_normalizes_to_the_documented_defaults() {
        assert_eq!(normalize(&json!({})).unwrap(), default_style());
    }

    #[test]
    fn missing_file_reads_as_default_and_a_round_trip_returns_normalized() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(get(dir.path()), default_style());
        let saved = set(
            &json!({"preset": "bracket", "language": "en", "max_chars": 24, "unknown": 1}),
            dir.path(),
        )
        .unwrap();
        assert_eq!(saved["preset"], json!("bracket"));
        assert_eq!(saved["max_chars"], json!(24));
        assert!(saved.get("unknown").is_none());
        assert_eq!(saved["types"], default_types());
        assert_eq!(get(dir.path()), saved);
    }

    #[test]
    fn a_corrupt_file_degrades_to_defaults_instead_of_failing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE_NAME), "{ not json").unwrap();
        assert_eq!(get(dir.path()), default_style());
        std::fs::write(dir.path().join(FILE_NAME), r#"{"preset": "nope"}"#).unwrap();
        assert_eq!(get(dir.path()), default_style());
    }

    #[test]
    fn every_validation_rule_is_load_bearing() {
        for bad in [
            json!([]),
            json!({"preset": "nope"}),
            json!({"language": "fr"}),
            json!({"max_chars": 5}),
            json!({"max_chars": 61}),
            json!({"max_chars": "16"}),
            json!({"type_prefix": "yes"}),
            json!({"types": []}),
            json!({"types": {}}),
            json!({"types": [{"emoji": "", "zh": "实现", "en": "feat"}]}),
            json!({"types": [{"emoji": "✨", "zh": "实现"}]}),
            json!({"types": [{"emoji": "✨✨✨✨✨", "zh": "实现", "en": "feat"}]}),
            json!({"instructions": 1}),
            json!({"instructions": "字".repeat(2001)}),
            json!({"examples": ["a", "b", "c", "d"]}),
            json!({"examples": [""]}),
            json!({"examples": [1]}),
        ] {
            assert!(normalize(&bad).is_err(), "应当拒绝: {bad}");
        }
        // 16 条类型是上限本身，必须通过。
        let full: Vec<Value> = (0..16)
            .map(|index| json!({"emoji": "✨", "zh": format!("类{index}"), "en": "feat"}))
            .collect();
        assert!(normalize(&json!({"types": full})).is_ok());
    }
}
