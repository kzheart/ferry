//! 格式无关的会话编辑契约与通用事务工具。
//!
//! 通用层只编排 preview/apply；每个 Agent 包内实现自己的 [`EditBackend`]，
//! 公共模块不引用任何具体 Agent 实现。

use std::any::Any;
use std::fs;
use std::path::Path;

use serde_json::{Map, Value};

use crate::errors::{DomainError, DomainResult};

// Python 的 `editing` 转出 codec 的这三个名字（`from .codec import ...`），
// 各 adapter 按旧路径引用，这里保持同样的再导出。
pub use super::codec::{positive_turn, select_span, TurnSpan};
pub use crate::jsonutil::hash_bytes;

/// Python 的 `EditBackend` 抽象基类在 Rust 里就是 WP-A 定型的
/// `contracts::SessionEditor`：两者方法集合与默认行为一一对应
/// （`replace_reply` 默认抛 `edit.operation_unsupported`、`snapshot` 默认 `None`）。
/// 唯一没法写进 trait 默认方法的是 `saved_revision`（需要 `self.name`），
/// 由 [`default_saved_revision`] 提供，adapter 直接转调即可。
pub use crate::adapters::contracts::SessionEditor as EditBackend;

/// `EditBackend.operations` 的类级默认值。
pub const DEFAULT_EDIT_OPERATIONS: &[&str] = &["delete-turn", "rewrite"];

/// 一次编辑事务里流转的文档。
///
/// `handle` / `data` / `context` 是 adapter 私有结构（Python 侧标注为 `object`），
/// 通用层只搬运不解释，因此用 `Box<dyn Any>` 承载。
pub struct EditDocument {
    pub tool: String,
    /// 对应 Python 的 `ref` 字段（Rust 里 `ref` 是关键字）。
    pub reference: String,
    pub handle: Box<dyn Any + Send>,
    pub data: Box<dyn Any + Send>,
    pub revision: String,
    pub context: Option<Box<dyn Any + Send>>,
}

impl EditDocument {
    pub fn new(
        tool: impl Into<String>,
        reference: impl Into<String>,
        handle: Box<dyn Any + Send>,
        data: Box<dyn Any + Send>,
        revision: impl Into<String>,
    ) -> Self {
        Self {
            tool: tool.into(),
            reference: reference.into(),
            handle,
            data,
            revision: revision.into(),
            context: None,
        }
    }
}

impl std::fmt::Debug for EditDocument {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EditDocument")
            .field("tool", &self.tool)
            .field("reference", &self.reference)
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}

/// 会被视为「派生子 agent」的工具名；替换回复时一律拒绝。
pub const SPAWN_TOOL_NAMES: &[&str] = &["agent", "spawn_agent", "task"];

/// 默认的编辑前快照原因码。
pub const SNAPSHOT_BEFORE_EDIT: &str = "snapshot.before_edit";

/// `EditBackend.saved_revision` 的默认实现：读回落盘文件算 `hash_bytes`。
///
/// 读不到就是缺陷（Python 抛 `RuntimeError`，经 RPC 兜底成 `internal.unexpected`）。
pub fn default_saved_revision(
    name: &str,
    result: &Map<String, Value>,
    _doc: &EditDocument,
) -> DomainResult<String> {
    let saved_as = result.get("saved_as").and_then(Value::as_str).unwrap_or("");
    let path = Path::new(saved_as);
    if path.is_file() {
        if let Ok(bytes) = fs::read(path) {
            return Ok(hash_bytes(&bytes));
        }
    }
    Err(DomainError::internal(format!(
        "{name} 无法读取已保存会话 revision"
    )))
}

/// `len(json.dumps(value, ensure_ascii=False).encode())`：按 UTF-8 字节数计。
pub fn json_size(value: &Value) -> usize {
    super::writing::python_json_dumps(value).len()
}

/// 就地编辑版的 JSONL 落盘：同目录 `.{name}.{pid}.tmp` → 换行拼接 → `os.replace`。
///
/// 与 [`super::writing::write_jsonl`] 的差别是**故意**的：这里不建父目录、
/// 不 fsync（目标文件已存在，编辑事务外层还有快照兜底）。
pub fn write_jsonl(path: &Path, records: &[Value]) -> std::io::Result<()> {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temporary = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    let mut payload = String::new();
    for (index, record) in records.iter().enumerate() {
        if index > 0 {
            payload.push('\n');
        }
        payload.push_str(&super::writing::python_json_dumps(record));
    }
    payload.push('\n');
    fs::write(&temporary, payload.as_bytes())?;
    fs::rename(&temporary, path)
}

/// 替换回复里出现 spawn/task 工具 → 拒绝：子 Agent 会改变会话树。
pub fn reject_replacement_spawn(reply: &Value) -> DomainResult<()> {
    let items = reply.get("items").and_then(Value::as_array);
    let has_spawn = items.is_some_and(|items| {
        items.iter().any(|item| {
            item.get("kind").and_then(Value::as_str) == Some("tool")
                && is_spawn_name(item.get("name"))
        })
    });
    if has_spawn {
        return Err(DomainError::subagent_not_supported(
            "子 Agent spawn/task 会改变会话树，编辑操作已拒绝",
        ));
    }
    Ok(())
}

/// 目标回复本身是 spawn/task → 拒绝。Python 里是无条件 `raise`，
/// Rust 直接返回构造好的错误，由调用方 `return Err(...)`。
pub fn reject_target_spawn(tool: &str) -> DomainError {
    let mut params = Map::new();
    params.insert("tool".into(), Value::from(tool));
    DomainError::new(
        "edit.subagent_not_supported",
        "SubagentNotSupportedError",
        "目标回复包含子 Agent spawn/task，编辑操作已拒绝",
        params,
    )
}

/// 工具名是否是 spawn/task（大小写不敏感；非字符串一律 false）。
pub fn is_spawn_name(name: Option<&Value>) -> bool {
    name.and_then(Value::as_str)
        .is_some_and(|name| SPAWN_TOOL_NAMES.contains(&name.to_lowercase().as_str()))
}

/// 字符串形态的便捷版本。
pub fn is_spawn_tool(name: &str) -> bool {
    SPAWN_TOOL_NAMES.contains(&name.to_lowercase().as_str())
}

/// 把首个命中 `is_reply` 的位置整体替换成 `compiled`，其余命中项删除。
///
/// 一条都没命中时把 `compiled` 追加到末尾（对齐 Python 的 `if not inserted`）。
pub fn replace_at_first<T: Clone>(
    records: &[T],
    is_reply: impl Fn(&T) -> bool,
    compiled: &[T],
) -> Vec<T> {
    let mut result = Vec::with_capacity(records.len() + compiled.len());
    let mut inserted = false;
    for record in records {
        if is_reply(record) {
            if !inserted {
                result.extend_from_slice(compiled);
                inserted = true;
            }
        } else {
            result.push(record.clone());
        }
    }
    if !inserted {
        result.extend_from_slice(compiled);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn spawn_names_are_case_insensitive_and_type_safe() {
        assert!(is_spawn_tool("Task"));
        assert!(is_spawn_tool("SPAWN_AGENT"));
        assert!(!is_spawn_tool("bash"));
        assert!(is_spawn_name(Some(&json!("agent"))));
        assert!(!is_spawn_name(Some(&json!(1))));
        assert!(!is_spawn_name(None));
    }

    #[test]
    fn replacement_replies_reject_spawn_tools() {
        let clean = json!({"items": [{"kind": "text", "text": "hi"},
                                     {"kind": "tool", "name": "Bash"}]});
        assert!(reject_replacement_spawn(&clean).is_ok());
        let dirty = json!({"items": [{"kind": "tool", "name": "Task"}]});
        let error = reject_replacement_spawn(&dirty).unwrap_err();
        assert_eq!(error.code, "edit.subagent_not_supported");
        assert_eq!(
            error.message(),
            "子 Agent spawn/task 会改变会话树，编辑操作已拒绝"
        );
        // kind 不是 tool 的项即使叫 task 也不算（对齐 isinstance(item, ToolItem)）。
        let text_named_task = json!({"items": [{"kind": "text", "name": "task"}]});
        assert!(reject_replacement_spawn(&text_named_task).is_ok());
    }

    #[test]
    fn replace_at_first_collapses_every_match_into_one_slot() {
        let records = vec![1, 2, 3, 2, 4];
        assert_eq!(
            replace_at_first(&records, |value| *value == 2, &[9, 9]),
            vec![1, 9, 9, 3, 4]
        );
        // 一条都没命中 -> 追加到末尾。
        assert_eq!(
            replace_at_first(&records, |value| *value == 7, &[9]),
            vec![1, 2, 3, 2, 4, 9]
        );
        // 空 compiled = 纯删除。
        assert_eq!(
            replace_at_first(&records, |value| *value == 2, &[]),
            vec![1, 3, 4]
        );
    }

    #[test]
    fn edit_write_jsonl_uses_a_pid_scoped_sibling_temp_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("session.jsonl");
        write_jsonl(&path, &[json!({"a": 1}), json!({"b": 2})]).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "{\"a\": 1}\n{\"b\": 2}\n"
        );
        // 临时文件已 rename 掉，目录里只剩目标文件。
        let names: Vec<String> = fs::read_dir(root.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["session.jsonl"]);
    }

    /// 编辑版对空记录集会写出**单个换行**（`"\n".join([]) + "\n"`），
    /// 迁移版写出空文件——两者不可互换。
    #[test]
    fn the_two_write_jsonl_variants_differ_on_empty_input() {
        let root = tempfile::tempdir().unwrap();
        let edited = root.path().join("edited.jsonl");
        write_jsonl(&edited, &[]).unwrap();
        assert_eq!(fs::read_to_string(&edited).unwrap(), "\n");

        let migrated = root.path().join("migrated.jsonl");
        super::super::writing::write_jsonl(&migrated, &[]).unwrap();
        assert_eq!(fs::read_to_string(&migrated).unwrap(), "");
    }

    #[test]
    fn saved_revision_hashes_the_file_on_disk() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("saved.jsonl");
        fs::write(&path, b"ferry").unwrap();
        let doc = EditDocument::new("claude", "ref", Box::new(()), Box::new(()), "sha256:old");
        let mut result = Map::new();
        result.insert("saved_as".into(), Value::from(path.to_str().unwrap()));
        assert_eq!(
            default_saved_revision("claude", &result, &doc).unwrap(),
            hash_bytes(b"ferry")
        );
        let error = default_saved_revision("claude", &Map::new(), &doc).unwrap_err();
        assert_eq!(error.code, "internal.unexpected");
        assert_eq!(error.message(), "claude 无法读取已保存会话 revision");
    }
}
