//! Canonical tool-operation contract shared by every agent adapter.
//!
//! 语义事实源：`engine/sessions/tool_ops.py`。
//!
//! Adapter 把原生调用归一到这些操作；Writer 声明自己能原生保留还是只能降级渲染，
//! 迁移预览与实际写入共用同一套词表。

use serde_json::Value;

/// 11 个规范操作。
pub struct CanonicalOp;

impl CanonicalOp {
    pub const SHELL_EXEC: &'static str = "shell.exec";
    pub const FS_READ: &'static str = "fs.read";
    pub const FS_WRITE: &'static str = "fs.write";
    pub const FS_EDIT: &'static str = "fs.edit";
    pub const FS_PATCH: &'static str = "fs.patch";
    pub const FS_SEARCH: &'static str = "fs.search";
    pub const FS_GLOB: &'static str = "fs.glob";
    pub const WEB_FETCH: &'static str = "web.fetch";
    pub const WEB_SEARCH: &'static str = "web.search";
    pub const TOOL_INVOKE: &'static str = "tool.invoke";
    pub const AGENT_SPAWN: &'static str = "agent.spawn";
}

/// 输入字段的期望 JSON 类型；对齐 Python 的 `isinstance` 检查。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JsonType {
    /// Python `str`
    Str,
    /// Python `int`（**排除 bool**，也排除 float）
    Int,
    /// Python `bool`
    Bool,
    /// Python `dict`
    Dict,
    /// Python `list`
    List,
}

impl JsonType {
    fn matches(self, value: &Value) -> bool {
        match self {
            Self::Str => value.is_string(),
            // serde_json 里 bool 与 number 是两个变体，天然不会互相命中；
            // Python 需要额外拦 bool 是因为 `isinstance(True, int)` 为真。
            Self::Int => value.is_i64() || value.is_u64(),
            Self::Bool => value.is_boolean(),
            Self::Dict => value.is_object(),
            Self::List => value.is_array(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ToolOpSpec {
    pub required_inputs: &'static [&'static str],
    pub optional_inputs: &'static [&'static str],
    pub nonempty_inputs: &'static [&'static str],
    pub input_types: &'static [(&'static str, &'static [JsonType])],
}

const STR: &[JsonType] = &[JsonType::Str];
const INT: &[JsonType] = &[JsonType::Int];
const BOOL: &[JsonType] = &[JsonType::Bool];
const DICT: &[JsonType] = &[JsonType::Dict];
const LIST: &[JsonType] = &[JsonType::List];
const STR_DICT_LIST: &[JsonType] = &[JsonType::Str, JsonType::Dict, JsonType::List];
const DICT_STR: &[JsonType] = &[JsonType::Dict, JsonType::Str];

/// 每个规范操作的输入契约；顺序与 Python 的 `TOOL_OP_SPECS` 一致。
pub const TOOL_OP_SPECS: &[(&str, ToolOpSpec)] = &[
    (
        CanonicalOp::SHELL_EXEC,
        ToolOpSpec {
            required_inputs: &["command"],
            optional_inputs: &[
                "workdir",
                "timeout_ms",
                "background",
                "sandbox_policy",
                "description",
            ],
            nonempty_inputs: &["command"],
            input_types: &[
                ("command", STR),
                ("workdir", STR),
                ("timeout_ms", INT),
                ("background", BOOL),
                ("sandbox_policy", STR),
                ("description", STR),
            ],
        },
    ),
    (
        CanonicalOp::FS_READ,
        ToolOpSpec {
            required_inputs: &["file_path"],
            optional_inputs: &["offset", "limit"],
            nonempty_inputs: &["file_path"],
            input_types: &[("file_path", STR), ("offset", INT), ("limit", INT)],
        },
    ),
    (
        CanonicalOp::FS_WRITE,
        ToolOpSpec {
            required_inputs: &["file_path", "content"],
            optional_inputs: &[],
            nonempty_inputs: &["file_path"],
            input_types: &[("file_path", STR), ("content", STR)],
        },
    ),
    (
        CanonicalOp::FS_EDIT,
        ToolOpSpec {
            required_inputs: &["file_path", "old", "new"],
            optional_inputs: &["replace_all"],
            nonempty_inputs: &["file_path"],
            input_types: &[
                ("file_path", STR),
                ("old", STR),
                ("new", STR),
                ("replace_all", BOOL),
            ],
        },
    ),
    (
        CanonicalOp::FS_PATCH,
        ToolOpSpec {
            required_inputs: &["operations"],
            optional_inputs: &["raw_patch", "workdir"],
            nonempty_inputs: &[],
            input_types: &[("operations", LIST), ("raw_patch", STR), ("workdir", STR)],
        },
    ),
    (
        CanonicalOp::FS_SEARCH,
        ToolOpSpec {
            required_inputs: &["query"],
            optional_inputs: &["path", "glob", "max_results"],
            nonempty_inputs: &["query"],
            input_types: &[
                ("query", STR),
                ("path", STR),
                ("glob", STR),
                ("max_results", INT),
            ],
        },
    ),
    (
        CanonicalOp::FS_GLOB,
        ToolOpSpec {
            required_inputs: &["pattern"],
            optional_inputs: &["path"],
            nonempty_inputs: &["pattern"],
            input_types: &[("pattern", STR), ("path", STR)],
        },
    ),
    (
        CanonicalOp::WEB_FETCH,
        ToolOpSpec {
            required_inputs: &["url"],
            optional_inputs: &[
                "method",
                "headers",
                "body",
                "prompt",
                "format",
                "timeout_ms",
            ],
            nonempty_inputs: &["url"],
            input_types: &[
                ("url", STR),
                ("method", STR),
                ("headers", DICT),
                ("body", STR_DICT_LIST),
                ("prompt", STR),
                ("format", STR),
                ("timeout_ms", INT),
            ],
        },
    ),
    (
        CanonicalOp::WEB_SEARCH,
        ToolOpSpec {
            required_inputs: &["query"],
            optional_inputs: &["domains", "recency_days", "num_results"],
            nonempty_inputs: &["query"],
            input_types: &[
                ("query", STR),
                ("domains", LIST),
                ("recency_days", INT),
                ("num_results", INT),
            ],
        },
    ),
    (
        CanonicalOp::TOOL_INVOKE,
        ToolOpSpec {
            required_inputs: &["namespace", "name", "input"],
            optional_inputs: &["structure_summary", "children"],
            nonempty_inputs: &["namespace", "name"],
            input_types: &[
                ("namespace", STR),
                ("name", STR),
                ("input", DICT_STR),
                ("structure_summary", DICT_STR),
                ("children", LIST),
            ],
        },
    ),
    (
        CanonicalOp::AGENT_SPAWN,
        ToolOpSpec {
            required_inputs: &["description", "prompt", "subagent_type"],
            optional_inputs: &[
                "task_name",
                "model",
                "fork_mode",
                "fork_turns",
                "reasoning_effort",
            ],
            nonempty_inputs: &["description", "subagent_type"],
            input_types: &[
                ("description", STR),
                ("prompt", STR),
                ("subagent_type", STR),
                ("task_name", STR),
                ("model", STR),
                ("fork_mode", STR),
                ("fork_turns", STR),
                ("reasoning_effort", STR),
            ],
        },
    ),
];

/// 注释性字段：丢弃它们不构成信息损失，迁移预览不计入 ignored_fields。
/// `description` 是模型给人看的一句话说明，不影响调用语义。
pub const ANNOTATION_INPUTS: &[(&str, &[&str])] = &[(CanonicalOp::SHELL_EXEC, &["description"])];

/// 规范操作全集。
pub fn canonical_ops() -> impl Iterator<Item = &'static str> {
    TOOL_OP_SPECS.iter().map(|(op, _)| *op)
}

pub fn is_canonical_op(op: &str) -> bool {
    spec(op).is_some()
}

pub fn spec(op: &str) -> Option<&'static ToolOpSpec> {
    TOOL_OP_SPECS
        .iter()
        .find(|(name, _)| *name == op)
        .map(|(_, spec)| spec)
}

pub fn annotation_inputs(op: &str) -> &'static [&'static str] {
    ANNOTATION_INPUTS
        .iter()
        .find(|(name, _)| *name == op)
        .map(|(_, fields)| *fields)
        .unwrap_or(&[])
}

/// Python `bool(value)` 的 JSON 等价：空串/空容器/0/null/false 都为假。
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

/// Return whether a canonical tool call has the fields its writer needs.
pub fn has_valid_tool_input(op: Option<&str>, value: &Value) -> bool {
    let Some(spec) = op.and_then(spec) else {
        return false;
    };
    let Some(entries) = value.as_object() else {
        return false;
    };
    if spec
        .required_inputs
        .iter()
        .any(|field| entries.get(*field).is_none_or(Value::is_null))
    {
        return false;
    }
    if !spec
        .nonempty_inputs
        .iter()
        .all(|field| entries.get(*field).is_some_and(truthy))
    {
        return false;
    }
    for (field, expected) in spec.input_types {
        let Some(actual) = entries.get(*field) else {
            continue;
        };
        if actual.is_null() {
            continue;
        }
        // Python 里 `isinstance(True, int)` 为真，所以期望 int 时必须显式拒 bool。
        if expected.contains(&JsonType::Int) && actual.is_boolean() {
            return false;
        }
        if !expected.iter().any(|kind| kind.matches(actual)) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn there_are_exactly_eleven_canonical_ops() {
        assert_eq!(TOOL_OP_SPECS.len(), 11);
        assert!(is_canonical_op(CanonicalOp::AGENT_SPAWN));
        assert!(!is_canonical_op("shell.run"));
    }

    #[test]
    fn missing_or_empty_required_inputs_are_invalid() {
        assert!(has_valid_tool_input(
            Some(CanonicalOp::SHELL_EXEC),
            &json!({"command": "ls"})
        ));
        assert!(!has_valid_tool_input(
            Some(CanonicalOp::SHELL_EXEC),
            &json!({})
        ));
        assert!(!has_valid_tool_input(
            Some(CanonicalOp::SHELL_EXEC),
            &json!({"command": null})
        ));
        assert!(!has_valid_tool_input(
            Some(CanonicalOp::SHELL_EXEC),
            &json!({"command": ""})
        ));
        assert!(!has_valid_tool_input(None, &json!({"command": "ls"})));
        assert!(!has_valid_tool_input(
            Some(CanonicalOp::SHELL_EXEC),
            &json!("ls")
        ));
    }

    #[test]
    fn int_fields_reject_booleans() {
        assert!(has_valid_tool_input(
            Some(CanonicalOp::SHELL_EXEC),
            &json!({"command": "ls", "timeout_ms": 5})
        ));
        assert!(!has_valid_tool_input(
            Some(CanonicalOp::SHELL_EXEC),
            &json!({"command": "ls", "timeout_ms": true})
        ));
        // float 也不是 Python int。
        assert!(!has_valid_tool_input(
            Some(CanonicalOp::SHELL_EXEC),
            &json!({"command": "ls", "timeout_ms": 1.5})
        ));
        // 可选字段为 null 时跳过类型检查。
        assert!(has_valid_tool_input(
            Some(CanonicalOp::SHELL_EXEC),
            &json!({"command": "ls", "timeout_ms": null})
        ));
    }

    #[test]
    fn union_typed_fields_accept_every_declared_shape() {
        for body in [json!("x"), json!({"a": 1}), json!([1])] {
            assert!(has_valid_tool_input(
                Some(CanonicalOp::WEB_FETCH),
                &json!({"url": "https://x", "body": body})
            ));
        }
        assert!(!has_valid_tool_input(
            Some(CanonicalOp::WEB_FETCH),
            &json!({"url": "https://x", "body": 1})
        ));
    }

    #[test]
    fn patch_has_no_nonempty_requirement() {
        // operations 必须存在，但可以是空数组（nonempty_inputs 为空）。
        assert!(has_valid_tool_input(
            Some(CanonicalOp::FS_PATCH),
            &json!({"operations": []})
        ));
    }

    #[test]
    fn annotation_inputs_only_cover_shell_exec_description() {
        assert_eq!(annotation_inputs(CanonicalOp::SHELL_EXEC), ["description"]);
        assert!(annotation_inputs(CanonicalOp::FS_READ).is_empty());
    }
}
