//! Codex 工具方言（读端归一）。
//!
//! 语义事实源：`engine/adapters/codex/dialect.py`。
//!
//! Codex 的写端是记录信封级的定制渲染（exec 事件对），不走 render；
//! 这份方言只负责把 rollout 里的 function_call 归一成规范操作。
//! shell 家族四个名字共享一个解码钩子：command 可能是字符串或
//! `["bash", "-lc", ...]` 列表，timeout 字段名也有两种写法。

use std::sync::LazyLock;

use serde_json::{Map, Value};

use crate::adapters::shared::dialect::{python_str, FieldMap, OpBinding, ToolDialect};
use crate::tool_ops::CanonicalOp;

/// `decode_shell`：把 shell 家族的原生入参归一成 `shell.exec` 的规范入参。
pub fn decode_shell(args: &Map<String, Value>) -> Option<Map<String, Value>> {
    // Python 的 `args.get(...)` 对「缺键」与「值为 None」不作区分。
    let command = args
        .get("cmd")
        .filter(|value| !value.is_null())
        .or_else(|| args.get("command").filter(|value| !value.is_null()))?;
    let command = match command {
        Value::Array(parts) => {
            let head: Vec<&Value> = parts.iter().take(2).collect();
            let bash_lc = head.len() == 2
                && head[0].as_str() == Some("bash")
                && head[1].as_str() == Some("-lc");
            let tail = if bash_lc { &parts[2..] } else { &parts[..] };
            tail.iter().map(python_str).collect::<Vec<_>>().join(" ")
        }
        other => python_str(other),
    };
    let mut result = Map::new();
    result.insert("command".into(), Value::String(command));
    for field in ["workdir", "timeout_ms", "background"] {
        if let Some(value) = args.get(field).filter(|value| !value.is_null()) {
            result.insert(field.into(), value.clone());
        }
    }
    if !result.contains_key("timeout_ms") {
        if let Some(value) = args.get("timeout").filter(|value| !value.is_null()) {
            result.insert("timeout_ms".into(), value.clone());
        }
    }
    Some(result)
}

/// Codex 的静态方言；由 `adapter::build()` 注册进进程级注册表。
pub static DIALECT: LazyLock<ToolDialect> = LazyLock::new(|| {
    ToolDialect::new(
        "codex",
        "codex",
        vec![
            OpBinding::new(
                CanonicalOp::SHELL_EXEC,
                "shell",
                vec![
                    FieldMap::new("command"),
                    FieldMap::new("workdir"),
                    FieldMap::new("timeout_ms"),
                    FieldMap::new("background"),
                ],
            )
            .read_names(["shell_command", "exec", "exec_command"])
            .decode_hook(decode_shell),
            OpBinding::new(
                CanonicalOp::FS_READ,
                "read_file",
                vec![
                    FieldMap::new("file_path").native("path").read_default(""),
                    FieldMap::new("offset").native("start_line"),
                    FieldMap::new("limit"),
                ],
            )
            .readonly(),
        ],
    )
    .strict_input(true)
});

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap()
    }

    #[test]
    fn shell_decoder_accepts_both_command_spellings() {
        assert_eq!(
            decode_shell(&map(json!({"cmd": "ls"}))),
            Some(map(json!({"command": "ls"})))
        );
        assert_eq!(
            decode_shell(&map(json!({"command": "ls -la"}))),
            Some(map(json!({"command": "ls -la"})))
        );
        assert_eq!(decode_shell(&map(json!({"workdir": "/a"}))), None);
        // null 等价缺席。
        assert_eq!(decode_shell(&map(json!({"cmd": null}))), None);
    }

    #[test]
    fn bash_lc_wrappers_are_unwrapped() {
        assert_eq!(
            decode_shell(&map(json!({"cmd": ["bash", "-lc", "echo", "hi"]}))),
            Some(map(json!({"command": "echo hi"})))
        );
        // 非 bash -lc 前缀 → 全部拼接。
        assert_eq!(
            decode_shell(&map(json!({"cmd": ["sh", "-c", "echo"]}))),
            Some(map(json!({"command": "sh -c echo"})))
        );
        // 非字符串元素走 Python 的 str()。
        assert_eq!(
            decode_shell(&map(json!({"cmd": [1, true]}))),
            Some(map(json!({"command": "1 True"})))
        );
    }

    #[test]
    fn timeout_falls_back_to_the_legacy_field_name() {
        assert_eq!(
            decode_shell(&map(json!({"cmd": "ls", "timeout": 500}))),
            Some(map(json!({"command": "ls", "timeout_ms": 500})))
        );
        // timeout_ms 已经存在时不覆盖。
        assert_eq!(
            decode_shell(&map(json!({"cmd": "ls", "timeout_ms": 1, "timeout": 500}))),
            Some(map(json!({"command": "ls", "timeout_ms": 1})))
        );
    }

    #[test]
    fn every_shell_alias_maps_to_shell_exec() {
        for name in ["shell", "shell_command", "exec", "exec_command"] {
            let (op, canonical) = DIALECT.parse(name, &json!({"cmd": "pwd"})).unwrap();
            assert_eq!(op, CanonicalOp::SHELL_EXEC);
            assert_eq!(canonical, json!({"command": "pwd"}));
        }
    }

    #[test]
    fn read_file_is_read_only_and_strict() {
        let (op, canonical) = DIALECT
            .parse("read_file", &json!({"path": "/a", "start_line": 3}))
            .unwrap();
        assert_eq!(op, CanonicalOp::FS_READ);
        assert_eq!(canonical, json!({"file_path": "/a", "offset": 3}));
        // strict_input：非对象入参整体退回 tool.invoke。
        assert!(DIALECT.parse("read_file", &json!("x")).is_none());
        // readonly：写端没有 fs.read 的原生形态。
        assert!(DIALECT.render(CanonicalOp::FS_READ, &json!({})).is_none());
    }
}
