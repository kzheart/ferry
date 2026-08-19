//! Claude Code 工具方言。
//!
//! `strict_input` 保持默认的宽松档：入参不是对象时保留已识别的 op、原样透传。

use std::sync::LazyLock;

use serde_json::{Map, Value};

use crate::adapters::shared::dialect::{
    inline_workdir, workdir_inline_flags, Converter, FieldMap, OpBinding, RenderFlags, ToolDialect,
};
use crate::adapters::shared::migration::Fidelity;
use crate::tool_ops::CanonicalOp;

/// WebFetch 的 `prompt` 缺省时由 write_default 补一句通用提示词，
/// 形态变了但语义保住，如实标成 transformed。
fn webfetch_flags(canonical: &Map<String, Value>, _native: &Map<String, Value>) -> RenderFlags {
    if canonical.contains_key("prompt") {
        return RenderFlags::none();
    }
    RenderFlags::new(Fidelity::Transformed, &["default_fetch_prompt"])
}

/// 9 个原生工具绑定；顺序与 Python 声明一致。
pub static DIALECT: LazyLock<ToolDialect> = LazyLock::new(|| {
    ToolDialect::new(
        "claude",
        "claude",
        vec![
            OpBinding::new(
                CanonicalOp::SHELL_EXEC,
                "Bash",
                vec![
                    FieldMap::new("command").read_default("").write_default(""),
                    FieldMap::new("timeout_ms").native("timeout"),
                    FieldMap::new("background").native("run_in_background"),
                    FieldMap::new("sandbox_policy")
                        .native("dangerouslyDisableSandbox")
                        .decode(Converter::SandboxFlag)
                        .encode(Converter::SandboxUnflag),
                    FieldMap::new("description"),
                ],
            )
            .encode_post(inline_workdir, ["workdir"])
            .render_flags(workdir_inline_flags),
            OpBinding::new(
                CanonicalOp::FS_READ,
                "Read",
                vec![
                    FieldMap::new("file_path")
                        .read_default("")
                        .write_default(""),
                    FieldMap::new("offset"),
                    FieldMap::new("limit"),
                ],
            ),
            OpBinding::new(
                CanonicalOp::FS_WRITE,
                "Write",
                vec![
                    FieldMap::new("file_path")
                        .read_default("")
                        .write_default(""),
                    FieldMap::new("content").read_default("").write_default(""),
                ],
            ),
            OpBinding::new(
                CanonicalOp::FS_EDIT,
                "Edit",
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
                    FieldMap::new("replace_all"),
                ],
            ),
            OpBinding::new(
                CanonicalOp::FS_SEARCH,
                "Grep",
                vec![
                    FieldMap::new("query")
                        .native("pattern")
                        .read_default("")
                        .write_default(""),
                    FieldMap::new("path"),
                    FieldMap::new("glob"),
                    FieldMap::new("max_results").native("head_limit"),
                ],
            ),
            OpBinding::new(
                CanonicalOp::FS_GLOB,
                "Glob",
                vec![
                    FieldMap::new("pattern").read_default("").write_default(""),
                    FieldMap::new("path"),
                ],
            ),
            OpBinding::new(
                CanonicalOp::WEB_FETCH,
                "WebFetch",
                vec![
                    FieldMap::new("url").read_default("").write_default(""),
                    FieldMap::new("prompt")
                        .write_default("Fetch this URL and preserve its relevant content."),
                ],
            )
            .render_flags(webfetch_flags),
            OpBinding::new(
                CanonicalOp::WEB_SEARCH,
                "WebSearch",
                vec![
                    FieldMap::new("query").read_default("").write_default(""),
                    FieldMap::new("domains").native("allowed_domains"),
                ],
            ),
            OpBinding::new(
                CanonicalOp::AGENT_SPAWN,
                "Agent",
                vec![
                    FieldMap::new("description")
                        .read_default("")
                        .write_default(""),
                    FieldMap::new("prompt").read_default("").write_default(""),
                    FieldMap::new("subagent_type").read_default(""),
                    FieldMap::new("task_name").native("name"),
                    FieldMap::new("model"),
                    FieldMap::new("fork_mode").native("mode"),
                    FieldMap::new("reasoning_effort"),
                ],
            ),
        ],
    )
});

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap()
    }

    #[test]
    fn bash_round_trips_sandbox_and_workdir() {
        let (op, canonical) = DIALECT
            .parse(
                "Bash",
                &json!({"command": "ls", "dangerouslyDisableSandbox": true}),
            )
            .unwrap();
        assert_eq!(op, CanonicalOp::SHELL_EXEC);
        assert_eq!(
            canonical,
            json!({"command": "ls", "sandbox_policy": "dangerously-disable"})
        );

        let (name, native) = DIALECT
            .render(
                CanonicalOp::SHELL_EXEC,
                &json!({"command": "ls", "workdir": "/w s", "sandbox_policy": "default"}),
            )
            .unwrap();
        assert_eq!(name, "Bash");
        assert_eq!(
            Value::Object(native),
            json!({"command": "cd '/w s' && ls", "dangerouslyDisableSandbox": false})
        );
    }

    #[test]
    fn webfetch_without_prompt_is_transformed() {
        let flags = webfetch_flags(&map(json!({"url": "https://x"})), &Map::new());
        assert_eq!(flags.fidelity, Some(Fidelity::Transformed));
        assert_eq!(flags.reason_codes, ["default_fetch_prompt"]);
        let quiet = webfetch_flags(
            &map(json!({"url": "https://x", "prompt": "p"})),
            &Map::new(),
        );
        assert!(quiet.fidelity.is_none());
        assert!(quiet.reason_codes.is_empty());

        let (_, native) = DIALECT
            .render(CanonicalOp::WEB_FETCH, &json!({"url": "https://x"}))
            .unwrap();
        assert_eq!(
            native["prompt"],
            Value::from("Fetch this URL and preserve its relevant content.")
        );
    }

    #[test]
    fn nine_write_ops_are_declared() {
        let ops = DIALECT.write_ops();
        assert_eq!(ops.len(), 9);
        for op in [
            CanonicalOp::SHELL_EXEC,
            CanonicalOp::FS_READ,
            CanonicalOp::FS_WRITE,
            CanonicalOp::FS_EDIT,
            CanonicalOp::FS_SEARCH,
            CanonicalOp::FS_GLOB,
            CanonicalOp::WEB_FETCH,
            CanonicalOp::WEB_SEARCH,
            CanonicalOp::AGENT_SPAWN,
        ] {
            assert!(ops.contains(op), "缺少 {op}");
        }
        assert!(!DIALECT.is_strict_input());
    }

    /// 宽松档：入参不是对象时保留 op、原样透传。
    #[test]
    fn non_object_inputs_pass_through() {
        assert_eq!(
            DIALECT.parse("Read", &json!("raw")),
            Some((CanonicalOp::FS_READ, json!("raw")))
        );
    }

    #[test]
    fn grep_and_edit_rename_fields() {
        let (_, canonical) = DIALECT
            .parse("Grep", &json!({"pattern": "x", "head_limit": 5}))
            .unwrap();
        assert_eq!(canonical, json!({"query": "x", "max_results": 5}));
        let (_, native) = DIALECT
            .render(
                CanonicalOp::FS_EDIT,
                &json!({"file_path": "/a", "old": "1", "new": "2"}),
            )
            .unwrap();
        assert_eq!(
            Value::Object(native),
            json!({"file_path": "/a", "old_string": "1", "new_string": "2"})
        );
    }
}
