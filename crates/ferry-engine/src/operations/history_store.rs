//! Ferry 迁移历史的 SQLite 存储。
//!
//! `delete` 的 DELETE 与 COUNT 必须在同一个 `BEGIN IMMEDIATE` 事务里，
//! 否则 `remaining` 会读到别的写入者提交后的数字（§2.3 第 17 条）。

use std::sync::Arc;

use rusqlite::params;
use serde_json::{Map, Value};

use crate::operations::types::{EngineError, EngineResult};
use crate::storage::database::StateConnector;

/// `delete()` 的返回三元组。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryDeletion {
    pub deleted: bool,
    pub id: String,
    pub remaining: i64,
}

impl HistoryDeletion {
    pub fn to_value(&self) -> Value {
        let mut payload = Map::new();
        payload.insert("deleted".into(), Value::from(self.deleted));
        payload.insert("id".into(), Value::from(self.id.as_str()));
        payload.insert("remaining".into(), Value::from(self.remaining));
        Value::Object(payload)
    }
}

#[derive(Debug)]
pub struct MigrationHistoryStore {
    connector: Arc<StateConnector>,
}

impl MigrationHistoryStore {
    pub fn new(connector: Arc<StateConnector>) -> Self {
        Self { connector }
    }

    /// 单语句 autocommit 写（对齐 Python：这里没有显式事务）。
    pub fn append(&self, history_id: &str, entry: &Value) -> EngineResult<()> {
        let entry_json = crate::storage::database::canonical_json(entry)?;
        self.connector.with_connection(|connection| {
            connection.execute(
                "INSERT INTO migration_history(history_id, entry_json)
                 VALUES (?, ?)",
                params![history_id, entry_json],
            )?;
            Ok(())
        })
    }

    /// 倒序返回全部历史；`id` 覆盖在条目字段之上。
    pub fn list_all(&self) -> EngineResult<Vec<Value>> {
        self.connector.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT history_id, entry_json
                 FROM migration_history
                 ORDER BY sequence DESC",
            )?;
            let rows = statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            let mut entries = Vec::new();
            for row in rows {
                let (history_id, entry_json) = row?;
                let value: Value = serde_json::from_str(&entry_json)
                    .map_err(|error| EngineError::value_error(error.to_string()))?;
                let mut object = match value {
                    Value::Object(object) => object,
                    other => {
                        return Err(EngineError::value_error(format!(
                            "migration_history.entry_json 不是 object: {other}"
                        )))
                    }
                };
                object.insert("id".into(), Value::from(history_id));
                entries.push(Value::Object(object));
            }
            Ok(entries)
        })
    }

    pub fn delete(&self, history_id: &str) -> EngineResult<HistoryDeletion> {
        self.connector.with_connection(|connection| {
            connection.execute_batch("BEGIN IMMEDIATE")?;
            let deleted = connection.execute(
                "DELETE FROM migration_history WHERE history_id = ?",
                params![history_id],
            )? == 1;
            let remaining: i64 =
                connection.query_row("SELECT COUNT(*) FROM migration_history", [], |row| {
                    row.get(0)
                })?;
            connection.execute_batch("COMMIT")?;
            Ok(HistoryDeletion {
                deleted,
                id: history_id.to_string(),
                remaining,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::database::StateDatabase;
    use serde_json::json;

    fn database() -> (tempfile::TempDir, StateDatabase) {
        let dir = tempfile::tempdir().unwrap();
        let database = StateDatabase::open(dir.path().join("ferry-state.sqlite3"), false).unwrap();
        (dir, database)
    }

    #[test]
    fn entries_are_listed_newest_first_with_the_history_id_merged_in() {
        let (_dir, database) = database();
        database
            .migration_history
            .append("history_a", &json!({"src": "claude"}))
            .unwrap();
        database
            .migration_history
            .append("history_b", &json!({"src": "codex"}))
            .unwrap();

        let entries = database.migration_history.list_all().unwrap();
        assert_eq!(
            entries,
            vec![
                json!({"src": "codex", "id": "history_b"}),
                json!({"src": "claude", "id": "history_a"}),
            ]
        );
    }

    #[test]
    fn delete_reports_the_remaining_count_from_the_same_transaction() {
        let (_dir, database) = database();
        database
            .migration_history
            .append("history_a", &json!({}))
            .unwrap();
        database
            .migration_history
            .append("history_b", &json!({}))
            .unwrap();

        let removed = database.migration_history.delete("history_a").unwrap();
        assert_eq!(
            removed,
            HistoryDeletion {
                deleted: true,
                id: "history_a".into(),
                remaining: 1,
            }
        );

        let missing = database.migration_history.delete("history_a").unwrap();
        assert!(!missing.deleted);
        assert_eq!(missing.remaining, 1);
    }
}
