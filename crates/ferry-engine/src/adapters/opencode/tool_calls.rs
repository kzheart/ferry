//! Canonical ToolCall 到 OpenCode 当前原生工具 part 的转换。
//!
//! 每个 writer 返回 `false` 表示「这次调用没有原生形态」，调用方据此降级成
//! 历史叙述（`narrate`）并记一条 `migration.tool_degraded`。

use serde_json::{Map, Value};

use crate::adapters::shared::dialect::python_str;
use crate::adapters::shared::migration::ToolVerdict;
use crate::model::{tool_result_text, ToolCall};
use crate::tool_ops::CanonicalOp;

/// `add_tool_part(tool, native_input, output, title, metadata, canonical_tool)`。
pub type AddToolPart<'a> =
    &'a mut dyn FnMut(&str, Value, &str, &str, Map<String, Value>, &ToolCall) -> bool;

/// 目标端能原生表达的规范操作（`OP_FIDELITY`）。
///
/// `agent.spawn` 不在 `OP_WRITERS` 里（它由 `_task_part` 单独构造），但保真度
/// 同样是 native。
pub const NATIVE_OPS: &[&str] = &[
    CanonicalOp::SHELL_EXEC,
    CanonicalOp::FS_READ,
    CanonicalOp::FS_WRITE,
    CanonicalOp::FS_EDIT,
    CanonicalOp::FS_PATCH,
    CanonicalOp::FS_SEARCH,
    CanonicalOp::FS_GLOB,
    CanonicalOp::WEB_FETCH,
    CanonicalOp::WEB_SEARCH,
    CanonicalOp::TOOL_INVOKE,
    CanonicalOp::AGENT_SPAWN,
];

/// `OP_FIDELITY` 查表。
pub fn op_fidelity(op: &str) -> ToolVerdict {
    if NATIVE_OPS.contains(&op) {
        ToolVerdict::Native
    } else {
        ToolVerdict::Degrade
    }
}

/// 该规范操作是否有 writer（`agent.spawn` 没有）。
pub fn has_writer(op: &str) -> bool {
    NATIVE_OPS.contains(&op) && op != CanonicalOp::AGENT_SPAWN
}

fn inputs_of(tool: &ToolCall) -> Map<String, Value> {
    tool.input.as_object().cloned().unwrap_or_else(Map::new)
}

/// Python `bool(value)` 的 JSON 等价。
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|float| float != 0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(entries)) => !entries.is_empty(),
    }
}

fn map_of(pairs: Vec<(&str, Value)>) -> Map<String, Value> {
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

fn truncated_flag(tool: &ToolCall) -> Value {
    Value::Bool(
        tool.result
            .as_ref()
            .and_then(|result| result.truncated)
            .unwrap_or(false),
    )
}

/// 把一次规范工具调用写成原生 tool part；无 writer 或入参不全时返回 `false`。
pub fn write_tool_part(op: &str, add: AddToolPart<'_>, tool: &ToolCall) -> bool {
    match op {
        CanonicalOp::SHELL_EXEC => write_shell_exec(add, tool),
        CanonicalOp::FS_READ => write_fs_read(add, tool),
        CanonicalOp::FS_WRITE => write_fs_write(add, tool),
        CanonicalOp::FS_EDIT => write_fs_edit(add, tool),
        CanonicalOp::FS_PATCH => write_fs_patch(add, tool),
        CanonicalOp::FS_SEARCH => write_renamed(
            add,
            tool,
            "grep",
            ("query", "pattern"),
            &[("path", "path"), ("glob", "include")],
        ),
        CanonicalOp::FS_GLOB => write_renamed(
            add,
            tool,
            "glob",
            ("pattern", "pattern"),
            &[("path", "path")],
        ),
        CanonicalOp::WEB_FETCH => write_renamed(
            add,
            tool,
            "webfetch",
            ("url", "url"),
            &[("format", "format"), ("timeout_ms", "timeout")],
        ),
        CanonicalOp::WEB_SEARCH => write_renamed(
            add,
            tool,
            "websearch",
            ("query", "query"),
            &[("num_results", "numResults")],
        ),
        CanonicalOp::TOOL_INVOKE => write_tool_invoke(add, tool),
        _ => false,
    }
}

fn write_shell_exec(add: AddToolPart<'_>, tool: &ToolCall) -> bool {
    let inputs = inputs_of(tool);
    if !truthy(inputs.get("command")) {
        return false;
    }
    let command = inputs["command"].clone();
    let mut native = map_of(vec![("command", command.clone())]);
    for (canonical, key) in [
        ("workdir", "workdir"),
        ("timeout_ms", "timeout"),
        ("background", "run_in_background"),
    ] {
        if let Some(value) = inputs.get(canonical) {
            native.insert(key.into(), value.clone());
        }
    }
    let output = tool_result_text(tool.result.as_ref());
    let metadata = map_of(vec![
        ("output", Value::from(output.clone())),
        (
            "exit",
            Value::from(
                tool.result
                    .as_ref()
                    .and_then(|result| result.exit_code)
                    .unwrap_or(0),
            ),
        ),
        ("truncated", truncated_flag(tool)),
    ]);
    add(
        "bash",
        Value::Object(native),
        &output,
        &python_str(&command),
        metadata,
        tool,
    )
}

fn write_fs_read(add: AddToolPart<'_>, tool: &ToolCall) -> bool {
    let inputs = inputs_of(tool);
    if !truthy(inputs.get("file_path")) {
        return false;
    }
    let path = inputs["file_path"].clone();
    let mut native = map_of(vec![("filePath", path.clone())]);
    for key in ["offset", "limit"] {
        if let Some(value) = inputs.get(key) {
            native.insert(key.into(), value.clone());
        }
    }
    let output = tool_result_text(tool.result.as_ref());
    add(
        "read",
        Value::Object(native),
        &output,
        &python_str(&path),
        map_of(vec![("truncated", truncated_flag(tool))]),
        tool,
    )
}

fn write_fs_write(add: AddToolPart<'_>, tool: &ToolCall) -> bool {
    let inputs = inputs_of(tool);
    if !truthy(inputs.get("file_path")) {
        return false;
    }
    let path = inputs["file_path"].clone();
    let native = map_of(vec![
        ("filePath", path.clone()),
        (
            "content",
            inputs
                .get("content")
                .cloned()
                .unwrap_or_else(|| Value::from("")),
        ),
    ]);
    let output = tool_result_text(tool.result.as_ref());
    let output = if output.is_empty() {
        "Wrote file successfully.".to_string()
    } else {
        output
    };
    let metadata = map_of(vec![
        ("filepath", path.clone()),
        ("exists", Value::Bool(false)),
        ("truncated", Value::Bool(false)),
        ("diagnostics", Value::Object(Map::new())),
    ]);
    add(
        "write",
        Value::Object(native),
        &output,
        &python_str(&path),
        metadata,
        tool,
    )
}

fn write_fs_edit(add: AddToolPart<'_>, tool: &ToolCall) -> bool {
    let inputs = inputs_of(tool);
    if !truthy(inputs.get("file_path")) {
        return false;
    }
    let path = inputs["file_path"].clone();
    let native = map_of(vec![
        ("filePath", path.clone()),
        (
            "oldString",
            inputs
                .get("old")
                .cloned()
                .unwrap_or_else(|| Value::from("")),
        ),
        (
            "newString",
            inputs
                .get("new")
                .cloned()
                .unwrap_or_else(|| Value::from("")),
        ),
    ]);
    let output = tool_result_text(tool.result.as_ref());
    let output = if output.is_empty() {
        "Edited file.".to_string()
    } else {
        output
    };
    add(
        "edit",
        Value::Object(native),
        &output,
        &python_str(&path),
        map_of(vec![("truncated", Value::Bool(false))]),
        tool,
    )
}

fn write_fs_patch(add: AddToolPart<'_>, tool: &ToolCall) -> bool {
    let inputs = inputs_of(tool);
    if !truthy(inputs.get("raw_patch")) {
        return false;
    }
    let native = map_of(vec![("patchText", inputs["raw_patch"].clone())]);
    let output = tool_result_text(tool.result.as_ref());
    let output = if output.is_empty() {
        "Applied patch.".to_string()
    } else {
        output
    };
    add(
        "apply_patch",
        Value::Object(native),
        &output,
        "apply patch",
        map_of(vec![("truncated", Value::Bool(false))]),
        tool,
    )
}

/// canonical 输入到 opencode 原生输入只差字段改名的一类工具。
fn write_renamed(
    add: AddToolPart<'_>,
    tool: &ToolCall,
    native_name: &str,
    required: (&str, &str),
    optional: &[(&str, &str)],
) -> bool {
    let inputs = inputs_of(tool);
    let (source_key, native_key) = required;
    if !truthy(inputs.get(source_key)) {
        return false;
    }
    let value = inputs[source_key].clone();
    let mut native = map_of(vec![(native_key, value.clone())]);
    for (canonical, key) in optional {
        if let Some(item) = inputs.get(*canonical) {
            native.insert((*key).to_string(), item.clone());
        }
    }
    let output = tool_result_text(tool.result.as_ref());
    add(
        native_name,
        Value::Object(native),
        &output,
        &python_str(&value),
        map_of(vec![("truncated", Value::Bool(false))]),
        tool,
    )
}

fn write_tool_invoke(add: AddToolPart<'_>, tool: &ToolCall) -> bool {
    let inputs = inputs_of(tool);
    if !truthy(inputs.get("name")) {
        return false;
    }
    let native_input = inputs.get("input");
    if !matches!(
        native_input,
        Some(Value::Object(_)) | Some(Value::String(_))
    ) {
        return false;
    }
    let name = python_str(&inputs["name"]);
    let output = tool_result_text(tool.result.as_ref());
    add(
        &name,
        native_input.cloned().unwrap_or(Value::Null),
        &output,
        &name,
        map_of(vec![
            ("historical", Value::Bool(true)),
            ("truncated", Value::Bool(false)),
        ]),
        tool,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{text_tool_result, ToolResultStatus};
    use serde_json::json;

    struct Capture {
        name: String,
        input: Value,
        output: String,
        title: String,
        metadata: Map<String, Value>,
    }

    fn run(op: &str, tool: &ToolCall) -> Option<Capture> {
        let mut captured: Option<Capture> = None;
        let mut add = |name: &str,
                       input: Value,
                       output: &str,
                       title: &str,
                       metadata: Map<String, Value>,
                       _tool: &ToolCall| {
            captured = Some(Capture {
                name: name.to_string(),
                input,
                output: output.to_string(),
                title: title.to_string(),
                metadata,
            });
            true
        };
        let ok = write_tool_part(op, &mut add, tool);
        assert_eq!(ok, captured.is_some());
        captured
    }

    fn call(op: &str, input: Value, output: &str) -> ToolCall {
        let mut tool = ToolCall::new("x", Some(op.to_string()), input);
        tool.result = Some(text_tool_result(output, ToolResultStatus::Success));
        tool
    }

    #[test]
    fn shell_exec_carries_exit_code_and_optional_fields() {
        let mut tool = call(
            CanonicalOp::SHELL_EXEC,
            json!({"command": "ls", "workdir": "/w", "timeout_ms": 500, "background": true}),
            "listing",
        );
        tool.result.as_mut().unwrap().exit_code = Some(3);
        let captured = run(CanonicalOp::SHELL_EXEC, &tool).unwrap();
        assert_eq!(captured.name, "bash");
        assert_eq!(
            captured.input,
            json!({"command": "ls", "workdir": "/w", "timeout": 500,
                   "run_in_background": true})
        );
        assert_eq!(captured.title, "ls");
        assert_eq!(captured.metadata["exit"], json!(3));
        assert_eq!(captured.metadata["output"], json!("listing"));
        assert_eq!(captured.metadata["truncated"], json!(false));
    }

    #[test]
    fn empty_required_inputs_refuse_to_render() {
        assert!(run(
            CanonicalOp::SHELL_EXEC,
            &call(CanonicalOp::SHELL_EXEC, json!({}), "")
        )
        .is_none());
        assert!(run(
            CanonicalOp::SHELL_EXEC,
            &call(CanonicalOp::SHELL_EXEC, json!({"command": ""}), "")
        )
        .is_none());
        assert!(run(
            CanonicalOp::FS_READ,
            &call(CanonicalOp::FS_READ, json!({}), "")
        )
        .is_none());
        assert!(run(
            CanonicalOp::FS_PATCH,
            &call(CanonicalOp::FS_PATCH, json!({}), "")
        )
        .is_none());
        // 入参不是 dict 也不行。
        assert!(run(
            CanonicalOp::SHELL_EXEC,
            &ToolCall::new("x", Some(CanonicalOp::SHELL_EXEC.into()), json!("ls"))
        )
        .is_none());
    }

    #[test]
    fn write_and_edit_have_default_outputs() {
        let tool = call(
            CanonicalOp::FS_WRITE,
            json!({"file_path": "/a", "content": "c"}),
            "",
        );
        let captured = run(CanonicalOp::FS_WRITE, &tool).unwrap();
        assert_eq!(captured.output, "Wrote file successfully.");
        assert_eq!(captured.metadata["filepath"], json!("/a"));
        assert_eq!(captured.metadata["exists"], json!(false));
        assert_eq!(captured.metadata["diagnostics"], json!({}));

        let tool = call(
            CanonicalOp::FS_EDIT,
            json!({"file_path": "/a", "old": "x", "new": "y"}),
            "",
        );
        let captured = run(CanonicalOp::FS_EDIT, &tool).unwrap();
        assert_eq!(captured.output, "Edited file.");
        assert_eq!(
            captured.input,
            json!({"filePath": "/a", "oldString": "x", "newString": "y"})
        );

        let tool = call(
            CanonicalOp::FS_PATCH,
            json!({"raw_patch": "*** Begin Patch"}),
            "",
        );
        let captured = run(CanonicalOp::FS_PATCH, &tool).unwrap();
        assert_eq!(captured.output, "Applied patch.");
        assert_eq!(captured.title, "apply patch");
    }

    #[test]
    fn renaming_writers_rename_required_and_optional_fields() {
        let tool = call(
            CanonicalOp::FS_SEARCH,
            json!({"query": "todo", "path": "/p", "glob": "*.rs"}),
            "hits",
        );
        let captured = run(CanonicalOp::FS_SEARCH, &tool).unwrap();
        assert_eq!(captured.name, "grep");
        assert_eq!(
            captured.input,
            json!({"pattern": "todo", "path": "/p", "include": "*.rs"})
        );
        assert_eq!(captured.title, "todo");

        let tool = call(
            CanonicalOp::WEB_SEARCH,
            json!({"query": "rust", "num_results": 5}),
            "",
        );
        let captured = run(CanonicalOp::WEB_SEARCH, &tool).unwrap();
        assert_eq!(captured.input, json!({"query": "rust", "numResults": 5}));
    }

    #[test]
    fn tool_invoke_requires_a_name_and_a_dict_or_string_input() {
        let tool = call(
            CanonicalOp::TOOL_INVOKE,
            json!({"namespace": "mcp", "name": "lookup", "input": {"q": 1}}),
            "done",
        );
        let captured = run(CanonicalOp::TOOL_INVOKE, &tool).unwrap();
        assert_eq!(captured.name, "lookup");
        assert_eq!(captured.input, json!({"q": 1}));
        assert_eq!(captured.metadata["historical"], json!(true));

        let broken = call(
            CanonicalOp::TOOL_INVOKE,
            json!({"name": "lookup", "input": 5}),
            "",
        );
        assert!(run(CanonicalOp::TOOL_INVOKE, &broken).is_none());
    }

    /// `NATIVE_OPS` 里除 `agent.spawn` 外的每个 op 都要真有 writer：给最小
    /// 合法入参必须写出原生 part。新增 op 忘了配 writer 时这里会红。
    #[test]
    fn every_native_op_but_agent_spawn_renders_from_minimal_input() {
        let minimal: &[(&str, Value)] = &[
            (CanonicalOp::SHELL_EXEC, json!({"command": "ls"})),
            (CanonicalOp::FS_READ, json!({"file_path": "/a"})),
            (CanonicalOp::FS_WRITE, json!({"file_path": "/a"})),
            (CanonicalOp::FS_EDIT, json!({"file_path": "/a"})),
            (
                CanonicalOp::FS_PATCH,
                json!({"raw_patch": "*** Begin Patch"}),
            ),
            (CanonicalOp::FS_SEARCH, json!({"query": "todo"})),
            (CanonicalOp::FS_GLOB, json!({"pattern": "*.rs"})),
            (CanonicalOp::WEB_FETCH, json!({"url": "https://x"})),
            (CanonicalOp::WEB_SEARCH, json!({"query": "rust"})),
            (CanonicalOp::TOOL_INVOKE, json!({"name": "n", "input": {}})),
        ];
        for op in NATIVE_OPS {
            if *op == CanonicalOp::AGENT_SPAWN {
                assert!(!has_writer(op));
                continue;
            }
            let input = minimal
                .iter()
                .find(|(key, _)| key == op)
                .unwrap_or_else(|| panic!("{op} 缺最小入参样例"))
                .1
                .clone();
            assert!(run(op, &call(op, input, "")).is_some(), "{op}");
        }
    }
}
