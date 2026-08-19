//! Grok 工具方言。
//!
//! 本机实测（2026-07 会话）Grok 有两代工具集：
//! - 当前代（小写）：run_terminal_command / read_file / write / search_replace /
//!   grep / web_fetch / web_search，参数值一律是字符串（`"limit": "150"`）。
//! - 旧代（PascalCase）：Shell / Read / Write / StrReplace / Grep / Glob /
//!   WebFetch / WebSearch。
//!
//! 写端只产出当前代；旧代以 readonly 绑定收进来。数值字段读端 int 纠偏、写端
//! 转回字符串以匹配原生格式。守卫策略统一 fallback：没见过的键（如 grep 的
//! `-A`/`-i` 旗标）整体保留为私有调用，不做有损猜测。

use std::sync::LazyLock;

use crate::adapters::shared::dialect::{
    inline_workdir, workdir_inline_flags, Converter, Extras, FieldMap, OpBinding, ToolDialect,
};
use crate::tool_ops::CanonicalOp;

pub static DIALECT: LazyLock<ToolDialect> = LazyLock::new(|| {
    ToolDialect::new(
        "grok",
        "grok",
        vec![
            OpBinding::new(
                CanonicalOp::SHELL_EXEC,
                "run_terminal_command",
                vec![
                    FieldMap::new("command").read_default("").write_default(""),
                    FieldMap::new("description"),
                    FieldMap::new("timeout_ms")
                        .native("timeout")
                        .decode(Converter::Int)
                        .encode(Converter::Str),
                    FieldMap::new("background")
                        .native("is_background")
                        .decode(Converter::Bool),
                ],
            )
            .extras(Extras::Fallback)
            .encode_post(inline_workdir, ["workdir"])
            .render_flags(workdir_inline_flags),
            OpBinding::new(
                CanonicalOp::SHELL_EXEC,
                "Shell",
                vec![
                    FieldMap::new("command").read_default(""),
                    FieldMap::new("description"),
                    FieldMap::new("workdir").native("working_directory"),
                    FieldMap::new("timeout_ms")
                        .native("block_until_ms")
                        .decode(Converter::Int),
                ],
            )
            .extras(Extras::Fallback)
            .readonly(),
            OpBinding::new(
                CanonicalOp::FS_READ,
                "read_file",
                vec![
                    FieldMap::new("file_path")
                        .native("target_file")
                        .read_default("")
                        .write_default(""),
                    FieldMap::new("offset")
                        .decode(Converter::Int)
                        .encode(Converter::Str),
                    FieldMap::new("limit")
                        .decode(Converter::Int)
                        .encode(Converter::Str),
                ],
            )
            .extras(Extras::Fallback),
            OpBinding::new(
                CanonicalOp::FS_READ,
                "Read",
                vec![
                    FieldMap::new("file_path").native("path").read_default(""),
                    FieldMap::new("offset").decode(Converter::Int),
                    FieldMap::new("limit").decode(Converter::Int),
                ],
            )
            .extras(Extras::Fallback)
            .readonly(),
            OpBinding::new(
                CanonicalOp::FS_WRITE,
                "write",
                vec![
                    FieldMap::new("file_path")
                        .read_default("")
                        .write_default(""),
                    FieldMap::new("content").read_default("").write_default(""),
                ],
            )
            .extras(Extras::Fallback),
            OpBinding::new(
                CanonicalOp::FS_WRITE,
                "Write",
                vec![
                    FieldMap::new("file_path").native("path").read_default(""),
                    FieldMap::new("content").native("contents").read_default(""),
                ],
            )
            .extras(Extras::Fallback)
            .readonly(),
            OpBinding::new(
                CanonicalOp::FS_EDIT,
                "search_replace",
                vec![
                    FieldMap::new("file_path")
                        .read_default("")
                        .write_default(""),
                    FieldMap::new("old")
                        .native("old_string")
                        .read_default("")
                        .write_default(""),
                    FieldMap::new("new")
                        .native("new_string")
                        .read_default("")
                        .write_default(""),
                    FieldMap::new("replace_all").decode(Converter::Bool),
                ],
            )
            .extras(Extras::Fallback),
            OpBinding::new(
                CanonicalOp::FS_EDIT,
                "StrReplace",
                vec![
                    FieldMap::new("file_path").native("path").read_default(""),
                    FieldMap::new("old").native("old_string").read_default(""),
                    FieldMap::new("new").native("new_string").read_default(""),
                    FieldMap::new("replace_all"),
                ],
            )
            .extras(Extras::Fallback)
            .readonly(),
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
                    FieldMap::new("max_results")
                        .native("head_limit")
                        .decode(Converter::Int)
                        .encode(Converter::Str),
                ],
            )
            .extras(Extras::Fallback),
            OpBinding::new(
                CanonicalOp::FS_SEARCH,
                "Grep",
                vec![
                    FieldMap::new("query").native("pattern").read_default(""),
                    FieldMap::new("path"),
                    FieldMap::new("glob"),
                    FieldMap::new("max_results")
                        .native("head_limit")
                        .decode(Converter::Int),
                ],
            )
            .extras(Extras::Fallback)
            .readonly(),
            OpBinding::new(
                CanonicalOp::FS_GLOB,
                "Glob",
                vec![
                    FieldMap::new("pattern")
                        .native("glob_pattern")
                        .read_default(""),
                    FieldMap::new("path").native("target_directory"),
                ],
            )
            .extras(Extras::Fallback)
            .readonly(),
            OpBinding::new(
                CanonicalOp::WEB_FETCH,
                "web_fetch",
                vec![FieldMap::new("url").read_default("").write_default("")],
            )
            .extras(Extras::Fallback),
            OpBinding::new(
                CanonicalOp::WEB_FETCH,
                "WebFetch",
                vec![FieldMap::new("url").read_default("")],
            )
            .extras(Extras::Fallback)
            .readonly(),
            OpBinding::new(
                CanonicalOp::WEB_SEARCH,
                "web_search",
                vec![
                    FieldMap::new("query").read_default("").write_default(""),
                    FieldMap::new("domains").native("allowed_domains"),
                ],
            )
            .extras(Extras::Fallback),
            // 旧代 WebSearch 的 explanation 是给人看的检索意图说明，丢弃不算损失，
            // 因此这条绑定**不用** fallback 守卫。
            OpBinding::new(
                CanonicalOp::WEB_SEARCH,
                "WebSearch",
                vec![FieldMap::new("query")
                    .native("search_term")
                    .read_default("")],
            )
            .readonly(),
        ],
    )
    .strict_input(true)
    // updates 流的 rawInput 带 variant 判别符（chat 行没有），是格式痕迹不是参数。
    .drop_native(["variant"])
});

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    #[test]
    fn the_current_generation_decodes_string_numbers() {
        let (op, canonical) = DIALECT
            .parse(
                "read_file",
                &json!({"target_file": "/a.txt", "offset": "3", "limit": "150"}),
            )
            .unwrap();
        assert_eq!(op, CanonicalOp::FS_READ);
        assert_eq!(
            canonical,
            json!({"file_path": "/a.txt", "offset": 3, "limit": 150})
        );
        // 写端把数值转回字符串以匹配原生形态。
        let (name, native) = DIALECT.render(CanonicalOp::FS_READ, &canonical).unwrap();
        assert_eq!(name, "read_file");
        assert_eq!(
            Value::Object(native),
            json!({"target_file": "/a.txt", "offset": "3", "limit": "150"})
        );
    }

    #[test]
    fn legacy_names_read_only() {
        assert_eq!(DIALECT.op_for("Shell"), Some(CanonicalOp::SHELL_EXEC));
        assert_eq!(DIALECT.op_for("Glob"), Some(CanonicalOp::FS_GLOB));
        // fs.glob 只有 readonly 绑定 → 写端没有原生形态。
        assert!(DIALECT
            .render(CanonicalOp::FS_GLOB, &json!({"pattern": "*.rs"}))
            .is_none());
        // 写端一律落到当前代名字。
        let (name, _) = DIALECT
            .render(CanonicalOp::SHELL_EXEC, &json!({"command": "ls"}))
            .unwrap();
        assert_eq!(name, "run_terminal_command");
    }

    #[test]
    fn unknown_flags_fall_back_to_a_private_invocation() {
        // grep 的 -i/-A 旗标不在表里 → 整体退回，交给调用方兜成 tool.invoke。
        assert!(DIALECT
            .parse("grep", &json!({"pattern": "x", "-i": true}))
            .is_none());
        // variant 是传输层判别符，不触发守卫。
        let (_, canonical) = DIALECT
            .parse("grep", &json!({"pattern": "x", "variant": "updates"}))
            .unwrap();
        assert_eq!(canonical, json!({"query": "x"}));
    }

    #[test]
    fn strict_input_rejects_non_object_arguments() {
        assert!(DIALECT
            .parse("run_terminal_command", &json!("ls -la"))
            .is_none());
    }

    #[test]
    fn websearch_drops_the_explanation_without_a_guard() {
        let (op, canonical) = DIALECT
            .parse(
                "WebSearch",
                &json!({"search_term": "ferry", "explanation": "why"}),
            )
            .unwrap();
        assert_eq!(op, CanonicalOp::WEB_SEARCH);
        assert_eq!(canonical, json!({"query": "ferry"}));
    }

    #[test]
    fn shell_workdir_is_inlined_on_the_write_path() {
        let (_, native) = DIALECT
            .render(
                CanonicalOp::SHELL_EXEC,
                &json!({"command": "ls", "workdir": "/a b"}),
            )
            .unwrap();
        assert_eq!(native["command"], json!("cd '/a b' && ls"));
    }
}
