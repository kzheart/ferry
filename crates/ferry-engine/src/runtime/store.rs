//! Ferry Runtime 会话、消息与事件的 SQLite 存储。
//!
//! 语义事实源：`engine/runtime/store.py`。
//!
//! 这里只搬运 Runtime 已经做过体积约束的不透明 JSON，不解释 Provider / Role /
//! AgentMessage。写入按键不可变：同一 `(session_id, ordinal|seq)` 重复提交但
//! 载荷不同即冲突（先 rollback 再报错）。

use std::sync::Arc;

use rusqlite::{params, Connection};
use serde_json::{Map, Value};

use crate::operations::types::{EngineError, EngineResult};
use crate::storage::database::StateConnector;

#[derive(Debug)]
pub struct RuntimeSessionStore {
    connector: Arc<StateConnector>,
}

/// `record[key]` 取出的主键：Runtime 提交的是整数，但不做类型收窄以外的解释。
fn record_key(record: &Value, key: &str) -> EngineResult<i64> {
    record
        .get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| EngineError::key_error(key.to_string()))
}

impl RuntimeSessionStore {
    pub fn new(connector: Arc<StateConnector>) -> Self {
        Self { connector }
    }

    pub fn load_all(&self) -> EngineResult<Vec<Value>> {
        self.connector.with_connection(|connection| {
            let mut sessions =
                connection.prepare("SELECT session_id, metadata_json FROM runtime_sessions")?;
            let rows: Vec<(String, String)> = sessions
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<_, _>>()?;

            let mut result = Vec::with_capacity(rows.len());
            for (session_id, metadata_json) in rows {
                let messages = load_column(
                    connection,
                    "SELECT message_json FROM runtime_messages
                     WHERE session_id = ? ORDER BY ordinal",
                    &session_id,
                )?;
                let events = load_column(
                    connection,
                    "SELECT event_json FROM runtime_events
                     WHERE session_id = ? ORDER BY seq",
                    &session_id,
                )?;
                let mut state = parse_object(&metadata_json)?;
                state.insert("messages".into(), Value::Array(messages));
                let mut entry = Map::new();
                entry.insert("state".into(), Value::Object(state));
                entry.insert("events".into(), Value::Array(events));
                result.push(Value::Object(entry));
            }
            Ok(result)
        })
    }

    pub fn commit(&self, update: &Value) -> EngineResult<()> {
        let metadata = update
            .get("metadata")
            .ok_or_else(|| EngineError::key_error("metadata"))?;
        let session_id = metadata
            .get("session_id")
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::key_error("session_id"))?
            .to_string();
        let timestamp = update
            .get("timestamp")
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::key_error("timestamp"))?
            .to_string();
        let metadata_json = crate::storage::database::canonical_json(metadata)?;
        let empty = Vec::new();
        let messages = update
            .get("messages")
            .and_then(Value::as_array)
            .unwrap_or(&empty);
        let events = update
            .get("events")
            .and_then(Value::as_array)
            .unwrap_or(&empty);

        self.connector.with_connection(|connection| {
            connection.execute_batch("BEGIN IMMEDIATE")?;
            let existing_created_at: Option<String> = {
                let mut statement = connection.prepare(
                    "SELECT metadata_json, created_at
                     FROM runtime_sessions
                     WHERE session_id = ?",
                )?;
                let mut rows = statement.query(params![session_id])?;
                match rows.next()? {
                    Some(row) => Some(row.get("created_at")?),
                    None => None,
                }
            };
            connection.execute(
                "INSERT INTO runtime_sessions(
                     session_id, metadata_json, created_at, updated_at
                 ) VALUES (?, ?, ?, ?)
                 ON CONFLICT(session_id) DO UPDATE SET
                     metadata_json = excluded.metadata_json,
                     updated_at = excluded.updated_at",
                params![
                    session_id,
                    metadata_json,
                    existing_created_at.as_deref().unwrap_or(&timestamp),
                    timestamp,
                ],
            )?;
            insert_records(
                connection,
                "runtime_messages",
                &session_id,
                "ordinal",
                messages,
                |record| {
                    record
                        .get("message")
                        .cloned()
                        .ok_or_else(|| EngineError::key_error("message"))
                },
                "message_json",
            )?;
            // 事件按信封原样落盘：Runtime 提交的就是信封本身，seq 在信封里。
            insert_records(
                connection,
                "runtime_events",
                &session_id,
                "seq",
                events,
                |record| Ok(record.clone()),
                "event_json",
            )?;
            connection.execute_batch("COMMIT")?;
            Ok(())
        })
    }

    /// 删除 ordinal ≥ from_ordinal 的消息与 seq ≥ from_seq 的事件。
    ///
    /// 存储按键不可变，编辑重发要重用被截断的 ordinal/seq，必须先物理删除。
    pub fn truncate(
        &self,
        session_id: &str,
        from_ordinal: i64,
        from_seq: i64,
    ) -> EngineResult<(usize, usize)> {
        self.connector.with_connection(|connection| {
            let messages = connection.execute(
                "DELETE FROM runtime_messages
                 WHERE session_id = ? AND ordinal >= ?",
                params![session_id, from_ordinal],
            )?;
            let events = connection.execute(
                "DELETE FROM runtime_events WHERE session_id = ? AND seq >= ?",
                params![session_id, from_seq],
            )?;
            Ok((messages, events))
        })
    }

    pub fn delete(&self, session_id: &str) -> EngineResult<bool> {
        self.connector.with_connection(|connection| {
            Ok(connection.execute(
                "DELETE FROM runtime_sessions WHERE session_id = ?",
                params![session_id],
            )? == 1)
        })
    }
}

fn parse_object(text: &str) -> EngineResult<Map<String, Value>> {
    match serde_json::from_str(text).map_err(|error| EngineError::value_error(error.to_string()))? {
        Value::Object(object) => Ok(object),
        other => Err(EngineError::value_error(format!(
            "runtime_sessions.metadata_json 不是 object: {other}"
        ))),
    }
}

fn load_column(connection: &Connection, sql: &str, session_id: &str) -> EngineResult<Vec<Value>> {
    let mut statement = connection.prepare(sql)?;
    let rows = statement.query_map(params![session_id], |row| row.get::<_, String>(0))?;
    let mut values = Vec::new();
    for row in rows {
        values.push(
            serde_json::from_str(&row?)
                .map_err(|error| EngineError::value_error(error.to_string()))?,
        );
    }
    Ok(values)
}

fn insert_records(
    connection: &Connection,
    table: &str,
    session_id: &str,
    key: &str,
    records: &[Value],
    payload_of: impl Fn(&Value) -> EngineResult<Value>,
    column: &str,
) -> EngineResult<()> {
    for record in records {
        let identifier = record_key(record, key)?;
        let payload = crate::storage::database::canonical_json(&payload_of(record)?)?;
        let select = format!("SELECT {column} FROM {table} WHERE session_id = ? AND {key} = ?");
        let existing: Option<String> = {
            let mut statement = connection.prepare(&select)?;
            let mut rows = statement.query(params![session_id, identifier])?;
            match rows.next()? {
                Some(row) => Some(row.get(0)?),
                None => None,
            }
        };
        if let Some(existing) = existing {
            if existing != payload {
                connection.execute_batch("ROLLBACK")?;
                return Err(EngineError::runtime("Runtime 持久化记录冲突"));
            }
            continue;
        }
        let insert = format!("INSERT INTO {table}(session_id, {key}, {column}) VALUES (?, ?, ?)");
        connection.execute(&insert, params![session_id, identifier, payload])?;
    }
    Ok(())
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

    fn update(session_id: &str, timestamp: &str) -> Value {
        json!({
            "metadata": {"session_id": session_id, "title": "标题"},
            "timestamp": timestamp,
            "messages": [{"ordinal": 0, "message": {"role": "user"}}],
            "events": [{"seq": 0, "type": "run.started"}],
        })
    }

    #[test]
    fn commit_then_load_all_round_trips_the_opaque_payload() {
        let (_dir, database) = database();
        database
            .runtime_sessions
            .commit(&update("s1", "2026-01-01T00:00:00Z"))
            .unwrap();

        let loaded = database.runtime_sessions.load_all().unwrap();
        assert_eq!(
            loaded,
            vec![json!({
                "state": {
                    "session_id": "s1",
                    "title": "标题",
                    "messages": [{"role": "user"}],
                },
                "events": [{"seq": 0, "type": "run.started"}],
            })]
        );
    }

    #[test]
    fn created_at_is_preserved_across_commits() {
        let (_dir, database) = database();
        database
            .runtime_sessions
            .commit(&update("s1", "t1"))
            .unwrap();
        database
            .runtime_sessions
            .commit(&update("s1", "t2"))
            .unwrap();

        let (created_at, updated_at): (String, String) = {
            let connection = rusqlite::Connection::open(&database.path).unwrap();
            connection
                .query_row(
                    "SELECT created_at, updated_at FROM runtime_sessions",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap()
        };
        assert_eq!(created_at, "t1");
        assert_eq!(updated_at, "t2");
    }

    #[test]
    fn conflicting_payload_for_an_existing_key_rolls_back() {
        let (_dir, database) = database();
        database
            .runtime_sessions
            .commit(&update("s1", "t1"))
            .unwrap();

        let mut conflicting = update("s1", "t2");
        conflicting["messages"][0]["message"] = json!({"role": "assistant"});
        let error = database.runtime_sessions.commit(&conflicting).unwrap_err();
        assert_eq!(error.message(), "Runtime 持久化记录冲突");

        // rollback 生效：后续写事务仍能拿到锁，且旧内容未被覆盖。
        database
            .runtime_sessions
            .commit(&update("s1", "t3"))
            .unwrap();
        let loaded = database.runtime_sessions.load_all().unwrap();
        assert_eq!(loaded[0]["state"]["messages"][0]["role"], json!("user"));
    }

    #[test]
    fn truncate_removes_from_the_given_ordinal_and_seq() {
        let (_dir, database) = database();
        let mut payload = update("s1", "t1");
        payload["messages"] = json!([
            {"ordinal": 0, "message": {"n": 0}},
            {"ordinal": 1, "message": {"n": 1}},
            {"ordinal": 2, "message": {"n": 2}},
        ]);
        payload["events"] = json!([{"seq": 0}, {"seq": 1}]);
        database.runtime_sessions.commit(&payload).unwrap();

        let (messages, events) = database.runtime_sessions.truncate("s1", 1, 1).unwrap();
        assert_eq!((messages, events), (2, 1));
        let loaded = database.runtime_sessions.load_all().unwrap();
        assert_eq!(loaded[0]["state"]["messages"], json!([{"n": 0}]));
        assert_eq!(loaded[0]["events"], json!([{"seq": 0}]));
    }

    #[test]
    fn delete_cascades_to_messages_and_events() {
        let (_dir, database) = database();
        database
            .runtime_sessions
            .commit(&update("s1", "t1"))
            .unwrap();

        assert!(database.runtime_sessions.delete("s1").unwrap());
        assert!(!database.runtime_sessions.delete("s1").unwrap());
        assert!(database.runtime_sessions.load_all().unwrap().is_empty());
        let remaining: i64 = {
            let connection = rusqlite::Connection::open(&database.path).unwrap();
            connection
                .query_row("SELECT COUNT(*) FROM runtime_messages", [], |row| {
                    row.get(0)
                })
                .unwrap()
        };
        assert_eq!(remaining, 0);
    }
}
