//! OpenCode 工具方言。
//!
//! 语义事实源：`engine/adapters/opencode/dialect.py`。
//!
//! 宽松模式（`strict_input=false`）：入参不是对象时保留已识别的 op、原样透传，
//! 与 claude 一致；`tool_canon::canonical_tool_input` 的探测顺序依赖这一点。

use std::sync::LazyLock;

use serde_json::{Map, Value};

use crate::adapters::shared::dialect::{FieldMap, OpBinding, ToolDialect};
use crate::adapters::shared::tool_canon::patch_operations;
use crate::tool_ops::CanonicalOp;

/// `apply_patch` 的原生入参只有补丁全文，操作清单在读端解析出来。
fn decode_patch(raw: &Map<String, Value>) -> Option<Map<String, Value>> {
    let patch = match raw.get("patchText") {
        Some(Value::String(text)) => text.clone(),
        Some(other) => crate::adapters::shared::dialect::python_str(other),
        None => String::new(),
    };
    let mut canonical = Map::new();
    canonical.insert(
        "operations".into(),
        Value::Array(
            patch_operations(&patch)
                .into_iter()
                .map(Value::Object)
                .collect(),
        ),
    );
    canonical.insert("raw_patch".into(), Value::from(patch));
    Some(canonical)
}

/// 没有补丁全文就没有原生形态：只有 operations 无法还原出补丁文本。
fn encode_patch(canonical: &Map<String, Value>) -> Option<Map<String, Value>> {
    let patch = canonical.get("raw_patch")?;
    if patch.as_str().is_none_or(|text| text.is_empty()) {
        return None;
    }
    let mut native = Map::new();
    native.insert("patchText".into(), patch.clone());
    Some(native)
}

fn text_or(raw: &Map<String, Value>, key: &str, fallback: &str) -> Value {
    match raw.get(key) {
        Some(Value::Null) | None => Value::from(fallback),
        Some(Value::String(text)) if text.is_empty() => Value::from(fallback),
        Some(Value::Bool(false)) => Value::from(fallback),
        Some(Value::String(text)) => Value::from(text.clone()),
        Some(other) => Value::from(crate::adapters::shared::dialect::python_str(other)),
    }
}

/// task 的三个字段都有兜底值，缺任何一个都不该让子 Agent 边丢失。
fn decode_task(raw: &Map<String, Value>) -> Option<Map<String, Value>> {
    let mut canonical = Map::new();
    canonical.insert(
        "description".into(),
        text_or(raw, "description", "migrated subagent"),
    );
    canonical.insert("prompt".into(), text_or(raw, "prompt", ""));
    canonical.insert(
        "subagent_type".into(),
        text_or(raw, "subagent_type", "general"),
    );
    Some(canonical)
}

fn encode_task(canonical: &Map<String, Value>) -> Option<Map<String, Value>> {
    let mut native = Map::new();
    for key in ["description", "prompt", "subagent_type"] {
        if let Some(value) = canonical.get(key) {
            native.insert(key.into(), value.clone());
        }
    }
    Some(native)
}

/// OpenCode 的工具方言。`build()` 负责把它注册进进程级方言表。
pub static DIALECT: LazyLock<ToolDialect> = LazyLock::new(|| {
    ToolDialect::new(
        "opencode",
        "opencode",
        vec![
            OpBinding::new(
                CanonicalOp::SHELL_EXEC,
                "bash",
                vec![
                    FieldMap::new("command").read_default("").write_default(""),
                    FieldMap::new("workdir"),
                    FieldMap::new("timeout_ms").native("timeout"),
                    FieldMap::new("background").native("run_in_background"),
                    FieldMap::new("description"),
                ],
            ),
            OpBinding::new(
                CanonicalOp::FS_READ,
                "read",
                vec![
                    FieldMap::new("file_path")
                        .native("filePath")
                        .read_default("")
                        .write_default(""),
                    FieldMap::new("offset"),
                    FieldMap::new("limit"),
                ],
            ),
            OpBinding::new(
                CanonicalOp::FS_WRITE,
                "write",
                vec![
                    FieldMap::new("file_path")
                        .native("filePath")
                        .read_default("")
                        .write_default(""),
                    FieldMap::new("content").read_default("").write_default(""),
                ],
            ),
            OpBinding::new(
                CanonicalOp::FS_EDIT,
                "edit",
                vec![
                    FieldMap::new("file_path")
                        .native("filePath")
                        .read_default("")
                        .write_default(""),
                    FieldMap::new("old")
                        .native("oldString")
                        .read_default("")
                        .write_default(""),
                    FieldMap::new("new")
                        .native("newString")
                        .read_default("")
                        .write_default(""),
                    FieldMap::new("replace_all").native("replaceAll"),
                ],
            ),
            OpBinding::new(
                CanonicalOp::FS_PATCH,
                "apply_patch",
                vec![FieldMap::new("operations"), FieldMap::new("raw_patch")],
            )
            .decode_hook(decode_patch)
            .encode_hook(encode_patch),
            OpBinding::new(
                CanonicalOp::FS_SEARCH,
                "grep",
                vec![
                    FieldMap::new("query")
                        .native("pattern")
                        .read_default("")
                        .write_default(""),
                    FieldMap::new("path"),
                    FieldMap::new("glob").native("include"),
                    FieldMap::new("max_results").native("limit"),
                ],
            ),
            OpBinding::new(
                CanonicalOp::FS_GLOB,
                "glob",
                vec![
                    FieldMap::new("pattern").read_default("").write_default(""),
                    FieldMap::new("path"),
                ],
            ),
            OpBinding::new(
                CanonicalOp::WEB_FETCH,
                "webfetch",
                vec![
                    FieldMap::new("url").read_default("").write_default(""),
                    FieldMap::new("format"),
                    FieldMap::new("timeout_ms").native("timeout"),
                ],
            ),
            OpBinding::new(
                CanonicalOp::WEB_SEARCH,
                "websearch",
                vec![
                    FieldMap::new("query").read_default("").write_default(""),
                    FieldMap::new("num_results").native("numResults"),
                ],
            ),
            OpBinding::new(
                CanonicalOp::AGENT_SPAWN,
                "task",
                vec![
                    FieldMap::new("description"),
                    FieldMap::new("prompt"),
                    FieldMap::new("subagent_type"),
                ],
            )
            .decode_hook(decode_task)
            .encode_hook(encode_task),
        ],
    )
});

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn native_names_are_camel_case_on_both_directions() {
        assert_eq!(DIALECT.op_for("bash"), Some(CanonicalOp::SHELL_EXEC));
        assert_eq!(DIALECT.op_for("webfetch"), Some(CanonicalOp::WEB_FETCH));
        assert_eq!(DIALECT.op_for("Bash"), None);

        let (op, canonical) = DIALECT
            .parse(
                "edit",
                &json!({"filePath": "/a", "oldString": "x", "newString": "y",
                                   "replaceAll": true}),
            )
            .unwrap();
        assert_eq!(op, CanonicalOp::FS_EDIT);
        assert_eq!(
            canonical,
            json!({"file_path": "/a", "old": "x", "new": "y", "replace_all": true})
        );

        let (name, native) = DIALECT
            .render(
                CanonicalOp::FS_SEARCH,
                &json!({"query": "todo", "glob": "*.rs"}),
            )
            .unwrap();
        assert_eq!(name, "grep");
        assert_eq!(native["pattern"], json!("todo"));
        assert_eq!(native["include"], json!("*.rs"));
    }

    #[test]
    fn missing_fields_fall_back_to_the_declared_read_defaults() {
        let (_, canonical) = DIALECT.parse("read", &json!({})).unwrap();
        assert_eq!(canonical, json!({"file_path": ""}));
        // 宽松模式：入参不是对象时原样透传。
        let (op, canonical) = DIALECT.parse("bash", &json!("ls")).unwrap();
        assert_eq!(op, CanonicalOp::SHELL_EXEC);
        assert_eq!(canonical, json!("ls"));
    }

    #[test]
    fn apply_patch_decodes_operations_and_refuses_to_encode_without_raw_patch() {
        let patch = "*** Begin Patch\n*** Add File: a.txt\n*** End Patch\n";
        let (op, canonical) = DIALECT
            .parse("apply_patch", &json!({"patchText": patch}))
            .unwrap();
        assert_eq!(op, CanonicalOp::FS_PATCH);
        assert_eq!(canonical["raw_patch"], json!(patch));
        assert_eq!(
            canonical["operations"],
            json!([{"operation": "add", "path": "a.txt"}])
        );

        assert!(DIALECT
            .render(CanonicalOp::FS_PATCH, &json!({"operations": []}))
            .is_none());
        let (name, native) = DIALECT
            .render(CanonicalOp::FS_PATCH, &json!({"raw_patch": patch}))
            .unwrap();
        assert_eq!(name, "apply_patch");
        assert_eq!(native["patchText"], json!(patch));
    }

    #[test]
    fn task_decoding_fills_every_missing_field() {
        let (op, canonical) = DIALECT.parse("task", &json!({"prompt": "review"})).unwrap();
        assert_eq!(op, CanonicalOp::AGENT_SPAWN);
        assert_eq!(
            canonical,
            json!({"description": "migrated subagent", "prompt": "review",
                   "subagent_type": "general"})
        );
        // 编码只搬运存在的键，不补默认值。
        let (_, native) = DIALECT
            .render(CanonicalOp::AGENT_SPAWN, &json!({"prompt": "review"}))
            .unwrap();
        assert_eq!(native, *json!({"prompt": "review"}).as_object().unwrap());
    }
}
