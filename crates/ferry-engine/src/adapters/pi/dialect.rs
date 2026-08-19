//! Pi 工具方言。
//!
//! pi 的守卫语义是混合的：bash/grep/find 遇到表外字段整体退回 `tool.invoke`
//! （`extras=fallback`），read/write 只取已知字段（`extras=ignore`）。edit 的
//! 原生形态是 `edits` 列表，只有「单元素且仅含 oldText/newText」时才能无损归一，
//! 其余保留原样（`decode_hook` 返回 `None` → 退回 `tool.invoke`）。

use std::sync::LazyLock;

use serde_json::{Map, Value};

use crate::adapters::shared::dialect::{
    inline_workdir, workdir_inline_flags, Converter, Extras, FieldMap, OpBinding, ToolDialect,
};
use crate::tool_ops::CanonicalOp;

/// 只有单元素、且仅含 `oldText`/`newText` 的 `edits` 才能无损归一。
fn decode_edit(raw: &Map<String, Value>) -> Option<Map<String, Value>> {
    if !raw.keys().all(|key| key == "path" || key == "edits") {
        return None;
    }
    let edits = raw.get("edits")?.as_array()?;
    if edits.len() != 1 {
        return None;
    }
    let edit = edits[0].as_object()?;
    if !edit.keys().all(|key| key == "oldText" || key == "newText") {
        return None;
    }
    let text = |value: Option<&Value>| value.cloned().unwrap_or_else(|| Value::from(""));
    let mut canonical = Map::new();
    canonical.insert("file_path".into(), text(raw.get("path")));
    canonical.insert("old".into(), text(edit.get("oldText")));
    canonical.insert("new".into(), text(edit.get("newText")));
    Some(canonical)
}

fn encode_edit(canonical: &Map<String, Value>) -> Option<Map<String, Value>> {
    let text = |value: Option<&Value>| value.cloned().unwrap_or_else(|| Value::from(""));
    let mut edit = Map::new();
    edit.insert("oldText".into(), text(canonical.get("old")));
    edit.insert("newText".into(), text(canonical.get("new")));
    let mut native = Map::new();
    native.insert("path".into(), text(canonical.get("file_path")));
    native.insert("edits".into(), Value::Array(vec![Value::Object(edit)]));
    Some(native)
}

/// pi 的方言声明。`build()` 里 `register_dialect("pi", &DIALECT)`。
pub static DIALECT: LazyLock<ToolDialect> = LazyLock::new(|| {
    ToolDialect::new(
        "pi",
        "pi",
        vec![
            OpBinding::new(
                CanonicalOp::SHELL_EXEC,
                "bash",
                vec![
                    FieldMap::new("command").read_default("").write_default(""),
                    FieldMap::new("timeout_ms")
                        .native("timeout")
                        .decode(Converter::SToMs)
                        .encode(Converter::MsToS),
                ],
            )
            .extras(Extras::Fallback)
            .encode_post(inline_workdir, ["workdir"])
            .render_flags(workdir_inline_flags),
            OpBinding::new(
                CanonicalOp::FS_READ,
                "read",
                vec![
                    FieldMap::new("file_path")
                        .native("path")
                        .read_alt(["file_path"])
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
                        .native("path")
                        .read_alt(["file_path"])
                        .read_default("")
                        .write_default(""),
                    FieldMap::new("content"),
                ],
            ),
            OpBinding::new(
                CanonicalOp::FS_EDIT,
                "edit",
                vec![
                    FieldMap::new("file_path"),
                    FieldMap::new("old"),
                    FieldMap::new("new"),
                ],
            )
            .decode_hook(decode_edit)
            .encode_hook(encode_edit),
            OpBinding::new(
                CanonicalOp::FS_SEARCH,
                "grep",
                vec![
                    FieldMap::new("query")
                        .native("pattern")
                        .read_default("")
                        .write_default(""),
                    FieldMap::new("path"),
                    FieldMap::new("glob"),
                    FieldMap::new("max_results").native("limit"),
                ],
            )
            .extras(Extras::Fallback),
            OpBinding::new(
                CanonicalOp::FS_GLOB,
                "find",
                vec![
                    FieldMap::new("pattern").read_default("").write_default(""),
                    FieldMap::new("path"),
                ],
            )
            .extras(Extras::Fallback),
        ],
    )
    .strict_input(true)
});

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bash_converts_seconds_and_falls_back_on_unknown_fields() {
        let (op, canonical) = DIALECT
            .parse("bash", &json!({"command": "pwd", "timeout": 3}))
            .unwrap();
        assert_eq!(op, CanonicalOp::SHELL_EXEC);
        assert_eq!(canonical, json!({"command": "pwd", "timeout_ms": 3000}));
        // 表外字段整体退回 tool.invoke。
        assert!(DIALECT
            .parse("bash", &json!({"command": "pwd", "shell": "zsh"}))
            .is_none());
        // strict_input：非 dict 入参一律退回。
        assert!(DIALECT.parse("bash", &json!("pwd")).is_none());
        // 写端秒/毫秒反向换算。
        let (name, native) = DIALECT
            .render(
                CanonicalOp::SHELL_EXEC,
                &json!({"command": "ls", "timeout_ms": 3000}),
            )
            .unwrap();
        assert_eq!(name, "bash");
        assert_eq!(native["timeout"], json!(3.0));
    }

    #[test]
    fn read_and_write_ignore_unknown_fields() {
        let (op, canonical) = DIALECT
            .parse("read", &json!({"path": "/a.txt", "extra": 1}))
            .unwrap();
        assert_eq!(op, CanonicalOp::FS_READ);
        assert_eq!(canonical, json!({"file_path": "/a.txt"}));
        let (op, canonical) = DIALECT
            .parse(
                "write",
                &json!({"path": "/a.txt", "content": "x", "mode": "w"}),
            )
            .unwrap();
        assert_eq!(op, CanonicalOp::FS_WRITE);
        assert_eq!(canonical, json!({"file_path": "/a.txt", "content": "x"}));
    }

    #[test]
    fn edit_only_normalises_single_element_two_key_edits() {
        let single = json!({"path": "/a", "edits": [{"oldText": "a", "newText": "b"}]});
        let (op, canonical) = DIALECT.parse("edit", &single).unwrap();
        assert_eq!(op, CanonicalOp::FS_EDIT);
        assert_eq!(
            canonical,
            json!({"file_path": "/a", "old": "a", "new": "b"})
        );
        // 多元素 / 多余键 / 多余顶层键都必须原样保留（返回 None → tool.invoke）。
        assert!(DIALECT
            .parse(
                "edit",
                &json!({"path": "/a", "edits": [{"oldText": "a", "newText": "b"},
                                                 {"oldText": "c", "newText": "d"}]})
            )
            .is_none());
        assert!(DIALECT
            .parse(
                "edit",
                &json!({"path": "/a", "edits": [{"oldText": "a", "newText": "b", "all": true}]})
            )
            .is_none());
        assert!(DIALECT
            .parse(
                "edit",
                &json!({"path": "/a", "replaceAll": true,
                        "edits": [{"oldText": "a", "newText": "b"}]})
            )
            .is_none());
        let (name, native) = DIALECT
            .render(
                CanonicalOp::FS_EDIT,
                &json!({"file_path": "/a", "old": "a", "new": "b"}),
            )
            .unwrap();
        assert_eq!(name, "edit");
        assert_eq!(
            Value::Object(native),
            json!({"path": "/a", "edits": [{"oldText": "a", "newText": "b"}]})
        );
    }

    #[test]
    fn grep_and_find_guard_on_extra_fields() {
        let (op, canonical) = DIALECT
            .parse("grep", &json!({"pattern": "x", "limit": 5}))
            .unwrap();
        assert_eq!(op, CanonicalOp::FS_SEARCH);
        assert_eq!(canonical, json!({"query": "x", "max_results": 5}));
        assert!(DIALECT
            .parse("grep", &json!({"pattern": "x", "ignoreCase": true}))
            .is_none());
        let (op, canonical) = DIALECT.parse("find", &json!({"pattern": "*.rs"})).unwrap();
        assert_eq!(op, CanonicalOp::FS_GLOB);
        assert_eq!(canonical, json!({"pattern": "*.rs"}));
        assert!(DIALECT
            .parse("find", &json!({"pattern": "*.rs", "depth": 2}))
            .is_none());
    }

    #[test]
    fn write_ops_cover_exactly_the_six_native_tools() {
        let ops: Vec<String> = DIALECT.write_ops().into_iter().collect();
        assert_eq!(
            ops,
            [
                "fs.edit".to_string(),
                "fs.glob".into(),
                "fs.read".into(),
                "fs.search".into(),
                "fs.write".into(),
                "shell.exec".into(),
            ]
        );
    }
}
