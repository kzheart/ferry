//! Codex 作为迁移目标的写入与规划能力。
//!
//! Codex 是唯一**不走方言渲染**的迁移目标：写端是记录信封级的定制渲染，
//! 因此 `preview_tool` 逐个操作手写映射。

use std::collections::BTreeSet;

use serde_json::{Map, Value};

use crate::adapters::shared::dialect::shell_quote;
use crate::adapters::shared::migration::{
    linked_agent_edge, Fidelity, MigrationTargetBase, RenderedTool, ToolVerdict,
};
use crate::errors::DomainResult;
use crate::model::{tool_result_text, Message, Session, ToolCall};
use crate::tool_ops::{annotation_inputs, has_valid_tool_input, CanonicalOp};

use super::writer;

/// Codex 迁移目标。
pub struct CodexMigrationTarget;

fn field_names(tool: &ToolCall) -> BTreeSet<String> {
    tool.input
        .as_object()
        .map(|entries| entries.keys().cloned().collect())
        .unwrap_or_default()
}

/// `set(inputs) - supported - ANNOTATION_INPUTS[op]`。
fn dropped(tool: &ToolCall, supported: &[&str]) -> BTreeSet<String> {
    let annotations = annotation_inputs(tool.op.as_deref().unwrap_or(""));
    field_names(tool)
        .into_iter()
        .filter(|field| !supported.contains(&field.as_str()))
        .filter(|field| !annotations.contains(&field.as_str()))
        .collect()
}

fn consumed(tool: &ToolCall, ignored: &BTreeSet<String>) -> BTreeSet<String> {
    field_names(tool)
        .into_iter()
        .filter(|field| !ignored.contains(field))
        .collect()
}

fn text_of(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(other) => crate::adapters::shared::dialect::python_str(other),
        None => String::new(),
    }
}

/// 原生映射：`_reason_codes` 只在有被忽略字段时出现。
fn native(
    name: &str,
    input: Value,
    output: &str,
    ignored: BTreeSet<String>,
    tool: &ToolCall,
) -> RenderedTool {
    let mut rendered = RenderedTool::tool(name, input, output)
        .conversion("native")
        .consumed_fields(consumed(tool, &ignored))
        .ignored_fields(ignored.clone());
    if !ignored.is_empty() {
        rendered = rendered.reason_codes(&["unsupported_tool_fields"]);
    }
    rendered
}

/// 形态改写：`transformed`；再有字段被丢就升级成 `lossy`。
fn transformed(
    name: &str,
    input: Value,
    output: &str,
    ignored: BTreeSet<String>,
    consumed_fields: BTreeSet<String>,
) -> RenderedTool {
    let lossy = !ignored.is_empty();
    let rendered = RenderedTool::tool(name, input, output)
        .conversion("transformed")
        .consumed_fields(consumed_fields)
        .ignored_fields(ignored)
        .fidelity(if lossy {
            Fidelity::Lossy
        } else {
            Fidelity::Transformed
        });
    if lossy {
        rendered.reason_codes(&["tool_transformed", "unsupported_tool_fields"])
    } else {
        rendered.reason_codes(&["tool_transformed"])
    }
}

fn exec_input(command: String, workdir: &str) -> Value {
    let mut input = Map::new();
    input.insert("cmd".into(), Value::from(command));
    input.insert("workdir".into(), Value::from(workdir));
    Value::Object(input)
}

impl MigrationTargetBase for CodexMigrationTarget {
    fn tool(&self) -> &str {
        "codex"
    }

    fn tool_fidelity(&self, op: &str) -> ToolVerdict {
        writer::op_fidelity(op)
    }

    fn preview_tool(
        &self,
        tool: &ToolCall,
        session: &Session,
        message: Option<&Message>,
    ) -> Option<RenderedTool> {
        if !has_valid_tool_input(tool.op.as_deref(), &tool.input) {
            return None;
        }
        let inputs = tool.input.as_object()?;
        let output = tool_result_text(tool.result.as_ref());
        match tool.op.as_deref()? {
            CanonicalOp::SHELL_EXEC => {
                let ignored = dropped(tool, &["command", "workdir"]);
                // `inputs.get("workdir", session.cwd)`：键存在就用它（含 None）。
                let workdir = match inputs.get("workdir") {
                    Some(value) => text_of(Some(value)),
                    None => session.cwd.clone(),
                };
                Some(native(
                    "exec",
                    exec_input(text_of(inputs.get("command")), &workdir),
                    &output,
                    ignored,
                    tool,
                ))
            }
            CanonicalOp::FS_READ => {
                let ignored = dropped(tool, &["file_path"]);
                let command = format!("cat {}", shell_quote(&text_of(inputs.get("file_path"))));
                Some(transformed(
                    "exec",
                    exec_input(command, &session.cwd),
                    &output,
                    ignored,
                    ["file_path".to_string()].into_iter().collect(),
                ))
            }
            op @ (CanonicalOp::FS_WRITE | CanonicalOp::FS_EDIT) => {
                let supported: &[&str] = if op == CanonicalOp::FS_WRITE {
                    &["file_path", "content"]
                } else {
                    &["file_path", "old", "new"]
                };
                let ignored = dropped(tool, supported);
                Some(native(
                    "apply_patch",
                    tool.input.clone(),
                    &output,
                    ignored,
                    tool,
                ))
            }
            CanonicalOp::FS_PATCH => {
                let raw_patch = inputs.get("raw_patch")?;
                if raw_patch.as_str().unwrap_or("").is_empty() {
                    return None;
                }
                let ignored = dropped(tool, &["operations", "raw_patch"]);
                let mut input = Map::new();
                input.insert("patch".into(), raw_patch.clone());
                Some(native(
                    "apply_patch",
                    Value::Object(input),
                    &output,
                    ignored,
                    tool,
                ))
            }
            CanonicalOp::FS_SEARCH => {
                let mut command = vec![
                    "rg".to_string(),
                    "--line-number".to_string(),
                    "--color".to_string(),
                    "never".to_string(),
                ];
                if let Some(glob) = inputs.get("glob").filter(|value| truthy(value)) {
                    command.push("-g".to_string());
                    command.push(text_of(Some(glob)));
                }
                command.push("--".to_string());
                command.push(text_of(inputs.get("query")));
                command.push(match inputs.get("path").filter(|value| truthy(value)) {
                    Some(path) => text_of(Some(path)),
                    None => ".".to_string(),
                });
                let ignored = dropped(tool, &["query", "path", "glob"]);
                let quoted = command
                    .iter()
                    .map(|part| shell_quote(part))
                    .collect::<Vec<_>>()
                    .join(" ");
                let consumed_fields = consumed(tool, &ignored);
                Some(transformed(
                    "exec",
                    exec_input(quoted, &session.cwd),
                    &output,
                    ignored,
                    consumed_fields,
                ))
            }
            CanonicalOp::FS_GLOB => {
                let command = [
                    "rg".to_string(),
                    "--files".to_string(),
                    "-g".to_string(),
                    text_of(inputs.get("pattern")),
                    "--".to_string(),
                    match inputs.get("path").filter(|value| truthy(value)) {
                        Some(path) => text_of(Some(path)),
                        None => ".".to_string(),
                    },
                ];
                let ignored = dropped(tool, &["pattern", "path"]);
                let quoted = command
                    .iter()
                    .map(|part| shell_quote(part))
                    .collect::<Vec<_>>()
                    .join(" ");
                let consumed_fields = consumed(tool, &ignored);
                Some(transformed(
                    "exec",
                    exec_input(quoted, &session.cwd),
                    &output,
                    ignored,
                    consumed_fields,
                ))
            }
            CanonicalOp::AGENT_SPAWN => {
                linked_agent_edge(session, tool, message, true)?;
                let ignored = dropped(tool, &["description", "prompt", "subagent_type"]);
                Some(native(
                    "spawn_agent",
                    tool.input.clone(),
                    &output,
                    ignored,
                    tool,
                ))
            }
            _ => None,
        }
    }

    fn write(&self, session: &Session, cwd: &str) -> DomainResult<Map<String, Value>> {
        let decider = |tool: &ToolCall, session: &Session, message: Option<&Message>| {
            self.evaluate_tool(tool, session, message)
        };
        let (session_id, dest) = writer::write(session, Some(cwd), None, None, Some(&decider))?;
        let mut result = Map::new();
        result.insert("session_id".into(), Value::from(session_id));
        result.insert(
            "dest".into(),
            Value::from(dest.to_string_lossy().into_owned()),
        );
        Ok(result)
    }
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|float| float != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(entries) => !entries.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{text_tool_result, AgentEdge, ToolResultStatus};
    use serde_json::json;

    fn tool(op: &str, input: Value) -> ToolCall {
        let mut call = ToolCall::new("t", Some(op.to_string()), input);
        call.result = Some(text_tool_result("out", ToolResultStatus::Success));
        call
    }

    fn session() -> Session {
        Session::new("claude", "s", "/work")
    }

    #[test]
    fn shell_exec_is_native_and_defaults_workdir_to_the_session() {
        let call = tool(CanonicalOp::SHELL_EXEC, json!({"command": "ls"}));
        let rendered = CodexMigrationTarget
            .preview_tool(&call, &session(), None)
            .unwrap();
        assert_eq!(rendered.block["name"], json!("exec"));
        assert_eq!(
            rendered.block["input"],
            json!({"cmd": "ls", "workdir": "/work"})
        );
        assert_eq!(rendered.conversion.as_deref(), Some("native"));
        let decision = CodexMigrationTarget
            .evaluate_tool(&call, &session(), None)
            .unwrap();
        assert_eq!(decision.fidelity, Fidelity::Exact);

        // description 是注释性字段，丢弃不算损失。
        let annotated = tool(
            CanonicalOp::SHELL_EXEC,
            json!({"command": "ls", "description": "list"}),
        );
        let decision = CodexMigrationTarget
            .evaluate_tool(&annotated, &session(), None)
            .unwrap();
        assert_eq!(decision.fidelity, Fidelity::Exact);
        assert!(decision.ignored_fields.is_empty());
    }

    #[test]
    fn fs_read_becomes_cat_and_is_disclosed_as_transformed() {
        let call = tool(CanonicalOp::FS_READ, json!({"file_path": "/a b.txt"}));
        let rendered = CodexMigrationTarget
            .preview_tool(&call, &session(), None)
            .unwrap();
        assert_eq!(
            rendered.block["input"],
            json!({"cmd": "cat '/a b.txt'", "workdir": "/work"})
        );
        let decision = CodexMigrationTarget
            .evaluate_tool(&call, &session(), None)
            .unwrap();
        assert_eq!(decision.fidelity, Fidelity::Transformed);
        assert_eq!(decision.reason_codes, ["tool_transformed"]);

        // 多出的 offset 字段被丢 → lossy。
        let lossy = tool(
            CanonicalOp::FS_READ,
            json!({"file_path": "/a.txt", "offset": 2}),
        );
        let decision = CodexMigrationTarget
            .evaluate_tool(&lossy, &session(), None)
            .unwrap();
        assert_eq!(decision.fidelity, Fidelity::Lossy);
        assert_eq!(
            decision.reason_codes,
            ["tool_transformed", "unsupported_tool_fields"]
        );
        assert_eq!(
            decision.ignored_fields.iter().cloned().collect::<Vec<_>>(),
            ["offset"]
        );
    }

    #[test]
    fn searches_and_globs_render_ripgrep_commands() {
        let search = tool(
            CanonicalOp::FS_SEARCH,
            json!({"query": "todo", "glob": "*.rs", "path": "src"}),
        );
        let rendered = CodexMigrationTarget
            .preview_tool(&search, &session(), None)
            .unwrap();
        assert_eq!(
            rendered.block["input"]["cmd"],
            json!("rg --line-number --color never -g '*.rs' -- todo src")
        );

        let glob = tool(CanonicalOp::FS_GLOB, json!({"pattern": "**/*.rs"}));
        let rendered = CodexMigrationTarget
            .preview_tool(&glob, &session(), None)
            .unwrap();
        assert_eq!(
            rendered.block["input"]["cmd"],
            json!("rg --files -g '**/*.rs' -- .")
        );
    }

    #[test]
    fn patches_require_raw_text_and_writes_reuse_apply_patch() {
        let empty = tool(CanonicalOp::FS_PATCH, json!({"operations": []}));
        assert!(CodexMigrationTarget
            .preview_tool(&empty, &session(), None)
            .is_none());

        let patch = tool(
            CanonicalOp::FS_PATCH,
            json!({"operations": [], "raw_patch": "*** Begin Patch"}),
        );
        let rendered = CodexMigrationTarget
            .preview_tool(&patch, &session(), None)
            .unwrap();
        assert_eq!(rendered.block["name"], json!("apply_patch"));
        assert_eq!(rendered.block["input"], json!({"patch": "*** Begin Patch"}));

        let write = tool(
            CanonicalOp::FS_WRITE,
            json!({"file_path": "/a.txt", "content": "x"}),
        );
        let rendered = CodexMigrationTarget
            .preview_tool(&write, &session(), None)
            .unwrap();
        assert_eq!(rendered.block["name"], json!("apply_patch"));
        assert_eq!(rendered.block["input"]["file_path"], json!("/a.txt"));
    }

    #[test]
    fn agent_spawn_requires_a_linked_edge() {
        let mut call = tool(
            CanonicalOp::AGENT_SPAWN,
            json!({"description": "d", "prompt": "p", "subagent_type": "general"}),
        );
        call.source_call_id = Some("c1".into());
        assert!(CodexMigrationTarget
            .preview_tool(&call, &session(), None)
            .is_none());

        let mut linked = session();
        let mut edge = AgentEdge::new("s", "child");
        edge.source_call_id = Some("c1".into());
        linked.agent_edges.push(edge);
        let rendered = CodexMigrationTarget
            .preview_tool(&call, &linked, None)
            .unwrap();
        assert_eq!(rendered.block["name"], json!("spawn_agent"));
        assert_eq!(rendered.conversion.as_deref(), Some("native"));
    }

    #[test]
    fn unsupported_ops_fall_through_to_narration() {
        let call = tool(
            CanonicalOp::TOOL_INVOKE,
            json!({"namespace": "x", "name": "y", "input": {}}),
        );
        assert!(CodexMigrationTarget
            .preview_tool(&call, &session(), None)
            .is_none());
        let decision = CodexMigrationTarget
            .evaluate_tool(&call, &session(), None)
            .unwrap();
        assert_eq!(decision.fidelity, Fidelity::Narrated);
        assert_eq!(decision.reason_codes, ["tool_to_history"]);
    }

    #[test]
    fn preview_reports_schema_version_three() {
        let mut session = session();
        let mut message = Message::new("user");
        message.blocks.push(crate::model::Block::text("hi"));
        session.messages.push(message);
        let preview = MigrationTargetBase::preview(&CodexMigrationTarget, &session, None).unwrap();
        assert_eq!(preview["schema_version"], json!(3));
        assert_eq!(preview["target_tool"], json!("codex"));
    }
}
