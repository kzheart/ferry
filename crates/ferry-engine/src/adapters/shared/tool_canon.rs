//! 工具调用规范化的兼容入口：实现已收敛到各 adapter 的 dialect 声明。
//!
//! 语义事实源：`engine/adapters/shared/tool_canon.py`。
//!
//! 映射的唯一事实源是各 adapter 的 `dialect.rs`；本模块保留原有函数签名，
//! 供仍按旧接口调用的 reader 与测试使用。[`canonical_tool_input`] 的历史签名
//! 不带 adapter 参数（claude 与 opencode 工具名天然不冲突），按
//! claude → opencode 顺序探测。

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value};

use super::dialect::get_dialect;

/// `canonical_tool_input` 的探测顺序，不可调换。
pub const CANONICAL_INPUT_ADAPTERS: &[&str] = &["claude", "opencode"];

/// `*** Add|Update|Delete File: <path>` 头部。
///
/// `(?m)` 下 `$` 匹配换行前的位置；`[^\r\n]+` 不吃 `\r`，所以 CRLF 补丁不命中
/// ——与 Python 的 `re.MULTILINE` 逐条一致。
static PATCH_HEADER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\*\*\* (Add|Update|Delete) File: ([^\r\n]+)$").expect("补丁头正则是常量")
});

/// 某 adapter 的原生工具名对应的规范操作。
pub fn canonical_tool_op(adapter: &str, tool_name: &str) -> Option<&'static str> {
    get_dialect(adapter)?.op_for(tool_name)
}

/// 解析 codex 风格补丁的文件操作清单。
pub fn patch_operations(patch: &str) -> Vec<Map<String, Value>> {
    PATCH_HEADER_RE
        .captures_iter(patch)
        .map(|captures| {
            let mut item = Map::new();
            item.insert("operation".into(), Value::from(captures[1].to_lowercase()));
            item.insert("path".into(), Value::from(captures[2].trim()));
            item
        })
        .collect()
}

/// 原生入参 → 规范入参；没有任何方言认识它时原样返回。
pub fn canonical_tool_input(tool_name: &str, raw: &Value) -> Value {
    canonical_tool_input_via(CANONICAL_INPUT_ADAPTERS, tool_name, raw)
}

/// [`canonical_tool_input`] 的显式探测顺序版本。
pub fn canonical_tool_input_via(adapters: &[&str], tool_name: &str, raw: &Value) -> Value {
    for adapter in adapters {
        let Some(dialect) = get_dialect(adapter) else {
            continue;
        };
        if let Some((_, canonical)) = dialect.parse(tool_name, raw) {
            return canonical;
        }
    }
    raw.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::shared::dialect::{FieldMap, OpBinding, ToolDialect};
    use crate::tool_ops::CanonicalOp;
    use serde_json::json;

    #[test]
    fn patch_operations_reads_the_three_headers() {
        let patch = "*** Begin Patch\n\
                     *** Add File: a/b.txt \n\
                     +hello\n\
                     *** Update File: c.rs\n\
                     *** Delete File: d.rs\n\
                     *** End Patch\n";
        assert_eq!(
            patch_operations(patch),
            vec![
                json!({"operation": "add", "path": "a/b.txt"})
                    .as_object()
                    .cloned()
                    .unwrap(),
                json!({"operation": "update", "path": "c.rs"})
                    .as_object()
                    .cloned()
                    .unwrap(),
                json!({"operation": "delete", "path": "d.rs"})
                    .as_object()
                    .cloned()
                    .unwrap(),
            ]
        );
    }

    #[test]
    fn patch_headers_must_start_the_line_and_avoid_cr() {
        assert!(patch_operations("prefix *** Add File: x").is_empty());
        // CRLF 补丁不命中（`[^\r\n]+` 后必须紧跟行尾）。
        assert!(patch_operations("*** Add File: x\r\n").is_empty());
        assert!(patch_operations("*** Rename File: x\n").is_empty());
    }

    #[test]
    fn canonical_tool_input_probes_claude_then_opencode() {
        // 探测顺序本身是契约；真实方言由 C1/C3 注册，这里用同构的替身，
        // 避免污染进程级注册表里 "claude"/"opencode" 两个槽位。
        assert_eq!(CANONICAL_INPUT_ADAPTERS, ["claude", "opencode"]);
        static CLAUDE: LazyLock<ToolDialect> = LazyLock::new(|| {
            ToolDialect::new(
                "claude",
                "claude",
                vec![OpBinding::new(
                    CanonicalOp::FS_READ,
                    "Read",
                    vec![FieldMap::new("file_path")],
                )],
            )
        });
        static OPENCODE: LazyLock<ToolDialect> = LazyLock::new(|| {
            ToolDialect::new(
                "opencode",
                "opencode",
                vec![OpBinding::new(
                    CanonicalOp::FS_READ,
                    "read",
                    vec![FieldMap::new("file_path").native("filePath")],
                )],
            )
        });
        super::super::dialect::register_dialect("wp-b2-claude", &CLAUDE);
        super::super::dialect::register_dialect("wp-b2-opencode", &OPENCODE);
        let probe = ["wp-b2-claude", "wp-b2-opencode"];

        assert_eq!(canonical_tool_op("wp-b2-claude", "Read"), Some("fs.read"));
        assert_eq!(canonical_tool_op("wp-b2-claude", "read"), None);
        assert_eq!(canonical_tool_op("nope", "Read"), None);

        assert_eq!(
            canonical_tool_input_via(&probe, "Read", &json!({"file_path": "/a"})),
            json!({"file_path": "/a"})
        );
        assert_eq!(
            canonical_tool_input_via(&probe, "read", &json!({"filePath": "/a"})),
            json!({"file_path": "/a"})
        );
        // 谁都不认识 -> 原样返回。
        assert_eq!(
            canonical_tool_input_via(&probe, "Weird", &json!({"x": 1})),
            json!({"x": 1})
        );
    }
}
