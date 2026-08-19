//! Ferry 会话元数据的 SQLite 存储 + 键编码 / 行解码 / 补丁合并三个纯函数。
//!
//! 语义事实源：`engine/contracts/metadata.py` 与
//! `engine/operations/metadata_store.py`。
//!
//! 硬约束（§2.2 第 13 条 / §2.4 第 26 条）：
//! - `metadata_key` = `tool + "\0" + session_id`，NUL 分隔不可换；
//! - `merge_metadata` 剔除假值（`None/False/""/[]`，Python 里 `0 == False`
//!   所以数值 0 同样被剔除），`pinned: false` 等价删键；
//! - 批量 CAS 两阶段、all-or-nothing：任一 expected 不匹配就整体 rollback 返回 None。

use std::sync::Arc;

use rusqlite::params;
use serde_json::{Map, Value};

use crate::operations::types::{EngineError, EngineResult};
use crate::storage::database::StateConnector;

/// `tool \0 session_id`。
pub fn metadata_key(tool: &str, session_id: &str) -> String {
    format!("{tool}\0{session_id}")
}

/// 行 → 元数据；缺行即空表。
pub fn metadata_entry(value_json: Option<&str>) -> EngineResult<Map<String, Value>> {
    let Some(value_json) = value_json else {
        return Ok(Map::new());
    };
    let value: Value = serde_json::from_str(value_json)
        .map_err(|error| EngineError::value_error(error.to_string()))?;
    match value {
        Value::Object(object) => Ok(object),
        other => Err(EngineError::value_error(format!(
            "session_metadata.value_json 不是 object: {other}"
        ))),
    }
}

/// Python 的 `value not in (None, False, "", [])`。
///
/// 注意 `0 == False` / `0.0 == False`：Python 会把数值 0 一并判成假值，
/// 这里逐字复刻，不做「更合理」的修正。
fn is_falsy(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Bool(flag) => !flag,
        Value::String(text) => text.is_empty(),
        Value::Array(items) => items.is_empty(),
        Value::Number(number) => number.as_f64() == Some(0.0),
        Value::Object(_) => false,
    }
}

/// `{**current, **patch}` 后剔除假值；键序 = current 原序 + patch 新键追加。
pub fn merge_metadata(
    current: &Map<String, Value>,
    patch: &Map<String, Value>,
) -> Map<String, Value> {
    let mut merged = current.clone();
    for (key, value) in patch {
        merged.insert(key.clone(), value.clone());
    }
    merged.retain(|_, value| !is_falsy(value));
    merged
}

/// 一次 CAS 请求：`(tool, session_id, expected, patch)`；`expected=None` 表示不校验。
#[derive(Clone, Debug, PartialEq)]
pub struct MetadataChange {
    pub tool: String,
    pub session_id: String,
    pub expected: Option<Map<String, Value>>,
    pub patch: Map<String, Value>,
}

#[derive(Debug)]
pub struct SessionMetadataStore {
    connector: Arc<StateConnector>,
}

impl SessionMetadataStore {
    pub fn new(connector: Arc<StateConnector>) -> Self {
        Self { connector }
    }

    pub fn list_all(&self) -> EngineResult<Map<String, Value>> {
        self.connector.with_connection(|connection| {
            let mut statement =
                connection.prepare("SELECT tool, session_id, value_json FROM session_metadata")?;
            let rows = statement.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            let mut result = Map::new();
            for row in rows {
                let (tool, session_id, value_json) = row?;
                result.insert(
                    metadata_key(&tool, &session_id),
                    Value::Object(metadata_entry(Some(&value_json))?),
                );
            }
            Ok(result)
        })
    }

    /// 无 CAS 的单条写；等价 Python 的 `set()`。
    pub fn set(
        &self,
        tool: &str,
        session_id: &str,
        patch: &Map<String, Value>,
        now: i64,
    ) -> EngineResult<Map<String, Value>> {
        let change = MetadataChange {
            tool: tool.to_string(),
            session_id: session_id.to_string(),
            expected: None,
            patch: patch.clone(),
        };
        let key = metadata_key(tool, session_id);
        let applied = self
            .compare_and_set(std::slice::from_ref(&change), now)?
            .ok_or_else(|| EngineError::key_error(key.clone()))?;
        match applied.get(&key) {
            Some(Value::Object(entry)) => Ok(entry.clone()),
            _ => Err(EngineError::key_error(key)),
        }
    }

    /// 两阶段批量 CAS：先全量比对 expected，再统一 upsert/delete。
    ///
    /// 返回 `None` 表示 CAS 失败（已 rollback，零写入）。
    pub fn compare_and_set(
        &self,
        changes: &[MetadataChange],
        now: i64,
    ) -> EngineResult<Option<Map<String, Value>>> {
        self.connector.with_connection(|connection| {
            connection.execute_batch("BEGIN IMMEDIATE")?;
            let mut current: Vec<Map<String, Value>> = Vec::with_capacity(changes.len());
            for change in changes {
                let mut statement = connection.prepare(
                    "SELECT value_json FROM session_metadata
                     WHERE tool = ? AND session_id = ?",
                )?;
                let mut rows = statement.query(params![change.tool, change.session_id])?;
                let stored: Option<String> = match rows.next()? {
                    Some(row) => Some(row.get(0)?),
                    None => None,
                };
                let value = metadata_entry(stored.as_deref())?;
                if let Some(expected) = &change.expected {
                    if &value != expected {
                        connection.execute_batch("ROLLBACK")?;
                        return Ok(None);
                    }
                }
                current.push(value);
            }

            let mut result = Map::new();
            for (change, current) in changes.iter().zip(current.iter()) {
                let key = metadata_key(&change.tool, &change.session_id);
                let entry = merge_metadata(current, &change.patch);
                if entry.is_empty() {
                    connection.execute(
                        "DELETE FROM session_metadata
                         WHERE tool = ? AND session_id = ?",
                        params![change.tool, change.session_id],
                    )?;
                } else {
                    let value_json =
                        crate::storage::database::canonical_json(&Value::Object(entry.clone()))?;
                    connection.execute(
                        "INSERT INTO session_metadata(
                             tool, session_id, value_json, updated_at
                         ) VALUES (?, ?, ?, ?)
                         ON CONFLICT(tool, session_id) DO UPDATE SET
                             value_json = excluded.value_json,
                             updated_at = excluded.updated_at",
                        params![change.tool, change.session_id, value_json, now],
                    )?;
                }
                result.insert(key, Value::Object(entry));
            }
            connection.execute_batch("COMMIT")?;
            Ok(Some(result))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::database::StateDatabase;
    use serde_json::json;

    fn object(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap()
    }

    fn database() -> (tempfile::TempDir, StateDatabase) {
        let dir = tempfile::tempdir().unwrap();
        let database = StateDatabase::open(dir.path().join("ferry-state.sqlite3"), false).unwrap();
        (dir, database)
    }

    #[test]
    fn key_uses_a_nul_separator() {
        assert_eq!(metadata_key("claude", "one"), "claude\0one");
    }

    #[test]
    fn merge_drops_python_falsy_values() {
        let merged = merge_metadata(
            &object(json!({"name": "keep", "pinned": true, "tags": ["a"]})),
            &object(json!({"pinned": false, "tags": [], "archived": true, "zero": 0})),
        );
        assert_eq!(merged, object(json!({"name": "keep", "archived": true})));
    }

    #[test]
    fn merge_keeps_current_key_order_then_appends_patch_keys() {
        let merged = merge_metadata(
            &object(json!({"name": "a", "pinned": true})),
            &object(json!({"archived": true, "name": "b"})),
        );
        let keys: Vec<&String> = merged.keys().collect();
        assert_eq!(keys, ["name", "pinned", "archived"]);
    }

    #[test]
    fn batch_cas_is_all_or_nothing() {
        let (_dir, database) = database();
        database
            .metadata
            .set("claude", "one", &object(json!({"name": "before"})), 1)
            .unwrap();
        database
            .metadata
            .set("codex", "two", &object(json!({"pinned": true})), 1)
            .unwrap();

        let changed = database
            .metadata
            .compare_and_set(
                &[
                    MetadataChange {
                        tool: "claude".into(),
                        session_id: "one".into(),
                        expected: Some(object(json!({"name": "before"}))),
                        patch: object(json!({"name": "after"})),
                    },
                    MetadataChange {
                        tool: "codex".into(),
                        session_id: "two".into(),
                        expected: Some(Map::new()),
                        patch: object(json!({"archived": true})),
                    },
                ],
                2,
            )
            .unwrap();

        assert!(changed.is_none());
        assert_eq!(
            database.metadata.list_all().unwrap(),
            object(json!({
                "claude\u{0}one": {"name": "before"},
                "codex\u{0}two": {"pinned": true},
            }))
        );
    }

    #[test]
    fn metadata_is_isolated_by_tool_and_native_session_id() {
        let (_dir, database) = database();
        database
            .metadata
            .set("claude", "shared-id", &object(json!({"name": "Claude"})), 1)
            .unwrap();
        database
            .metadata
            .set("codex", "shared-id", &object(json!({"name": "Codex"})), 2)
            .unwrap();

        assert_eq!(
            database.metadata.list_all().unwrap(),
            object(json!({
                "claude\u{0}shared-id": {"name": "Claude"},
                "codex\u{0}shared-id": {"name": "Codex"},
            }))
        );
    }

    #[test]
    fn empty_entry_deletes_the_row() {
        let (_dir, database) = database();
        database
            .metadata
            .set("claude", "one", &object(json!({"pinned": true})), 1)
            .unwrap();
        let entry = database
            .metadata
            .set("claude", "one", &object(json!({"pinned": false})), 2)
            .unwrap();
        assert!(entry.is_empty());
        assert!(database.metadata.list_all().unwrap().is_empty());
    }
}
