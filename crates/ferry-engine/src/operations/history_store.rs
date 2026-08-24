//! Ferry 迁移历史的 SQLite 存储。

use std::sync::Arc;

use rusqlite::params;
use serde_json::Value;

use crate::operations::types::{EngineError, EngineResult};
use crate::storage::database::StateConnector;

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
    fn legacy_probe_fields_round_trip_without_schema_filtering() {
        let (_dir, database) = database();
        let legacy = json!({
            "src": "claude",
            "dst": "codex",
            "probe_model": "legacy-model",
            "probe": {"status": "passed"},
            "validation": {
                "structure": {"ok": true},
                "runtime": {"status": "passed", "model": "legacy-model"}
            }
        });
        database
            .migration_history
            .append("history_legacy", &legacy)
            .unwrap();

        let entries = database.migration_history.list_all().unwrap();
        let mut expected = legacy;
        expected["id"] = json!("history_legacy");
        assert_eq!(entries, vec![expected]);
    }

}
