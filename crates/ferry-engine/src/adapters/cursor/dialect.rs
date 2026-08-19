//! Cursor 工具方言。
//!
//! 工具名以 `toolFormerData.name` 为准，不看 `toolFormerData.tool`：枚举 19
//! 是全部 MCP 工具的共用值、0 是「未知/新工具」的占位，两个都是多对一。
//!
//! MCP（`mcp-<server>-<tool>`）、`todo_write` / `read_lints` / `await` /
//! `update_current_step` / `get_mcp_tools` / `create_plan` 等 Cursor 专有工具
//! 没有跨 Agent 的规范语义，读端统一降级到 `tool.invoke` 兜底（reader 负责）。
//!
//! 严格模式：Cursor 的 `params` / `rawArgs` 是内嵌 JSON 字符串，解不开时是原始
//! 文本而不是对象——这种情况必须整体退回 `tool.invoke`，不能把裸串当规范入参。

use std::sync::LazyLock;

use serde_json::{Map, Value};

use crate::adapters::shared::dialect::{python_str, FieldMap, OpBinding, ToolDialect};
use crate::adapters::shared::tool_canon::patch_operations;
use crate::tool_ops::CanonicalOp;

fn text(raw: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    for key in keys {
        match raw.get(*key) {
            Some(Value::String(value)) if !value.is_empty() => return Some(value.clone()),
            Some(Value::Null) | None => continue,
            Some(other) => return Some(python_str(other)),
        }
    }
    None
}

/// `edit_file_v2` 的补丁全文在 `streamingContent` 里，是 OpenAI 风格的
/// `*** Begin Patch`。补丁缺失（本机 22%）时至少保住「改了哪个文件」这一条。
fn decode_edit(raw: &Map<String, Value>) -> Option<Map<String, Value>> {
    let patch = text(raw, &["streamingContent", "streamContent"]).unwrap_or_default();
    let mut operations = patch_operations(&patch);
    if operations.is_empty() {
        if let Some(path) = text(raw, &["relativeWorkspacePath", "path", "targetFile"]) {
            let mut item = Map::new();
            item.insert("operation".into(), Value::from("update"));
            item.insert("path".into(), Value::from(path));
            operations.push(item);
        }
    }
    let mut canonical = Map::new();
    canonical.insert(
        "operations".into(),
        Value::Array(operations.into_iter().map(Value::Object).collect()),
    );
    canonical.insert("raw_patch".into(), Value::from(patch));
    Some(canonical)
}

/// `delete_file` 也是一次文件级补丁，只是没有补丁文本。
fn decode_delete(raw: &Map<String, Value>) -> Option<Map<String, Value>> {
    let mut item = Map::new();
    item.insert("operation".into(), Value::from("delete"));
    item.insert(
        "path".into(),
        Value::from(text(raw, &["relativeWorkspacePath", "path"]).unwrap_or_default()),
    );
    let mut canonical = Map::new();
    canonical.insert("operations".into(), Value::Array(vec![Value::Object(item)]));
    Some(canonical)
}

/// `options.timeout` 是嵌套的，平铺不出来，只能靠 hook 摘。
fn decode_shell(raw: &Map<String, Value>) -> Option<Map<String, Value>> {
    let mut canonical = Map::new();
    canonical.insert(
        "command".into(),
        Value::from(text(raw, &["command"]).unwrap_or_default()),
    );
    if let Some(workdir) = text(raw, &["cwd", "workingDirectory"]) {
        canonical.insert("workdir".into(), Value::from(workdir));
    }
    let timeout = raw
        .get("options")
        .and_then(|options| options.get("timeout"))
        .or_else(|| raw.get("timeout"))
        .and_then(Value::as_i64);
    if let Some(timeout) = timeout {
        canonical.insert("timeout_ms".into(), Value::from(timeout));
    }
    if let Some(description) = text(raw, &["commandDescription"]) {
        canonical.insert("description".into(), Value::from(description));
    }
    Some(canonical)
}

/// task_v2 的三个字段都有兜底值，缺任何一个都不该让子 Agent 边丢失。
fn decode_task(raw: &Map<String, Value>) -> Option<Map<String, Value>> {
    let mut canonical = Map::new();
    canonical.insert(
        "description".into(),
        Value::from(
            text(raw, &["description", "name"]).unwrap_or_else(|| "migrated subagent".to_string()),
        ),
    );
    canonical.insert(
        "prompt".into(),
        Value::from(text(raw, &["prompt"]).unwrap_or_default()),
    );
    canonical.insert(
        "subagent_type".into(),
        Value::from(text(raw, &["subagentType"]).unwrap_or_else(|| "general".to_string())),
    );
    Some(canonical)
}

/// Cursor 只作为迁移源：全部绑定都是读端归一，没有写回形态。
pub static DIALECT: LazyLock<ToolDialect> = LazyLock::new(|| {
    ToolDialect::new(
        "cursor",
        "cursor",
        vec![
            OpBinding::new(
                CanonicalOp::SHELL_EXEC,
                "run_terminal_command_v2",
                vec![
                    FieldMap::new("command"),
                    FieldMap::new("workdir"),
                    FieldMap::new("timeout_ms"),
                    FieldMap::new("description"),
                ],
            )
            .read_names(["run_terminal_command", "Shell", "AwaitShell"])
            .decode_hook(decode_shell)
            .readonly(),
            OpBinding::new(
                CanonicalOp::FS_READ,
                "read_file_v2",
                vec![
                    FieldMap::new("file_path")
                        .native("targetFile")
                        .read_alt(["path", "effectiveUri"])
                        .read_default(""),
                    FieldMap::new("offset"),
                    FieldMap::new("limit"),
                ],
            )
            .read_names(["read_file", "Read", "ReadFile"])
            .readonly(),
            // Cursor 的编辑一律是补丁流，没有 old/new 对，走 fs.patch 而不是 fs.edit。
            OpBinding::new(
                CanonicalOp::FS_PATCH,
                "edit_file_v2",
                vec![FieldMap::new("operations"), FieldMap::new("raw_patch")],
            )
            .read_names([
                "edit_file",
                "apply_patch",
                "StrReplace",
                "ApplyPatch",
                "Write",
                "Edit",
            ])
            .decode_hook(decode_edit)
            .readonly(),
            OpBinding::new(
                CanonicalOp::FS_PATCH,
                "delete_file",
                vec![FieldMap::new("operations")],
            )
            .read_names(["Delete"])
            .decode_hook(decode_delete)
            .readonly(),
            OpBinding::new(
                CanonicalOp::FS_SEARCH,
                "ripgrep_raw_search",
                vec![
                    FieldMap::new("query").native("pattern").read_default(""),
                    FieldMap::new("path"),
                    FieldMap::new("glob"),
                    FieldMap::new("max_results").native("headLimit"),
                ],
            )
            .read_names(["grep", "Grep", "rg", "codebase_search", "file_search"])
            .readonly(),
            OpBinding::new(
                CanonicalOp::FS_GLOB,
                "glob_file_search",
                vec![
                    FieldMap::new("pattern")
                        .native("globPattern")
                        .read_alt(["pattern"])
                        .read_default(""),
                    FieldMap::new("path")
                        .native("targetDirectory")
                        .read_alt(["path"]),
                ],
            )
            .read_names(["Glob"])
            .readonly(),
            OpBinding::new(
                CanonicalOp::WEB_FETCH,
                "web_fetch",
                vec![FieldMap::new("url").read_default("")],
            )
            .read_names(["WebFetch", "fetch_rules"])
            .readonly(),
            OpBinding::new(
                CanonicalOp::WEB_SEARCH,
                "web_search",
                vec![FieldMap::new("query")
                    .native("searchTerm")
                    .read_alt(["query"])
                    .read_default("")],
            )
            .read_names(["WebSearch"])
            .readonly(),
            OpBinding::new(
                CanonicalOp::AGENT_SPAWN,
                "task_v2",
                vec![
                    FieldMap::new("description"),
                    FieldMap::new("prompt"),
                    FieldMap::new("subagent_type"),
                ],
            )
            .read_names(["task", "Task", "Subagent"])
            .decode_hook(decode_task)
            .readonly(),
        ],
    )
    .strict_input(true)
});

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn native_names_map_to_the_canonical_operations() {
        for (name, op) in [
            ("read_file_v2", CanonicalOp::FS_READ),
            ("run_terminal_command_v2", CanonicalOp::SHELL_EXEC),
            ("edit_file_v2", CanonicalOp::FS_PATCH),
            ("delete_file", CanonicalOp::FS_PATCH),
            ("ripgrep_raw_search", CanonicalOp::FS_SEARCH),
            ("glob_file_search", CanonicalOp::FS_GLOB),
            ("web_fetch", CanonicalOp::WEB_FETCH),
            ("web_search", CanonicalOp::WEB_SEARCH),
            ("task_v2", CanonicalOp::AGENT_SPAWN),
        ] {
            assert_eq!(DIALECT.op_for(name), Some(op), "{name}");
        }
        // MCP 与 Cursor 专有工具没有规范语义，交给 reader 兜底成 tool.invoke。
        for name in [
            "mcp-ida-pro-decompile",
            "todo_write",
            "read_lints",
            "await",
            "update_current_step",
            "get_mcp_tools",
            "search_conversations",
        ] {
            assert_eq!(DIALECT.op_for(name), None, "{name}");
        }
    }

    #[test]
    fn read_and_search_use_the_native_camel_case_names() {
        let (op, canonical) = DIALECT
            .parse(
                "read_file_v2",
                &json!({"targetFile": "/a/README.md", "charsLimit": 1000,
                        "effectiveUri": "/a/README.md", "toolCallId": "call_1"}),
            )
            .unwrap();
        assert_eq!(op, CanonicalOp::FS_READ);
        assert_eq!(canonical, json!({"file_path": "/a/README.md"}));

        let (_, canonical) = DIALECT
            .parse(
                "ripgrep_raw_search",
                &json!({"pattern": "todo", "glob": "*.rs", "headLimit": 20,
                        "outputMode": "content"}),
            )
            .unwrap();
        assert_eq!(
            canonical,
            json!({"query": "todo", "glob": "*.rs", "max_results": 20})
        );

        let (_, canonical) = DIALECT
            .parse(
                "glob_file_search",
                &json!({"globPattern": "**/*.kt", "targetDirectory": "/w"}),
            )
            .unwrap();
        assert_eq!(canonical, json!({"pattern": "**/*.kt", "path": "/w"}));
    }

    #[test]
    fn shell_lifts_the_nested_timeout_and_keeps_the_working_directory() {
        let (op, canonical) = DIALECT
            .parse(
                "run_terminal_command_v2",
                &json!({"command": "ls /tmp", "cwd": "/w",
                        "options": {"timeout": 30000},
                        "parsingResult": {"executableCommands": []},
                        "commandDescription": "list"}),
            )
            .unwrap();
        assert_eq!(op, CanonicalOp::SHELL_EXEC);
        assert_eq!(
            canonical,
            json!({"command": "ls /tmp", "workdir": "/w", "timeout_ms": 30000,
                   "description": "list"})
        );
    }

    #[test]
    fn edits_decode_to_patch_operations_and_degrade_without_the_patch_text() {
        let patch = "*** Begin Patch\n*** Update File: /w/X.kt\n+import b\n*** End Patch\n";
        let (op, canonical) = DIALECT
            .parse(
                "edit_file_v2",
                &json!({"relativeWorkspacePath": "/w/X.kt", "streamingContent": patch}),
            )
            .unwrap();
        assert_eq!(op, CanonicalOp::FS_PATCH);
        assert_eq!(canonical["raw_patch"], json!(patch));
        assert_eq!(
            canonical["operations"],
            json!([{"operation": "update", "path": "/w/X.kt"}])
        );

        // 补丁文本缺失：至少保住「改了哪个文件」。
        let (_, canonical) = DIALECT
            .parse("edit_file_v2", &json!({"relativeWorkspacePath": "/w/Y.kt"}))
            .unwrap();
        assert_eq!(
            canonical["operations"],
            json!([{"operation": "update", "path": "/w/Y.kt"}])
        );
        assert_eq!(canonical["raw_patch"], json!(""));

        let (_, canonical) = DIALECT
            .parse("delete_file", &json!({"relativeWorkspacePath": "/w/Z.kt"}))
            .unwrap();
        assert_eq!(
            canonical["operations"],
            json!([{"operation": "delete", "path": "/w/Z.kt"}])
        );
    }

    #[test]
    fn task_decoding_fills_every_missing_field() {
        let (op, canonical) = DIALECT
            .parse(
                "task_v2",
                &json!({"prompt": "explore", "subagentType": "explore"}),
            )
            .unwrap();
        assert_eq!(op, CanonicalOp::AGENT_SPAWN);
        assert_eq!(
            canonical,
            json!({"description": "migrated subagent", "prompt": "explore",
                   "subagent_type": "explore"})
        );
    }

    #[test]
    fn non_object_inputs_fall_back_instead_of_becoming_canonical() {
        // 严格模式：内嵌 JSON 解不开时是裸串，不能当规范入参用。
        assert!(DIALECT.parse("read_file_v2", &json!("{broken")).is_none());
    }

    #[test]
    fn the_dialect_declares_no_write_form() {
        assert!(DIALECT.write_ops().is_empty());
        assert!(DIALECT.render(CanonicalOp::FS_READ, &json!({})).is_none());
    }
}
