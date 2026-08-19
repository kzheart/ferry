//! Codex `state_5.sqlite` 会话注册。
//!
//! 语义事实源：`engine/adapters/codex/registry.py`。

use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::Connection;
use serde_json::{Map, Value};

use crate::errors::{DomainError, DomainResult};
use crate::model::{BlockKind, Session};

/// 一个待注册节点：会话、分配到的 id、落盘路径、父 id、cwd、agent_path、边状态。
pub struct RegistryNode<'a> {
    pub session: &'a Session,
    pub session_id: String,
    pub path: std::path::PathBuf,
    pub parent_id: Option<String>,
    pub cwd: String,
    pub agent_path: String,
    pub status: Option<String>,
}

/// `PRAGMA table_info` 的一行（只取判定要用的四列）。
struct ColumnInfo {
    name: String,
    notnull: bool,
    has_default: bool,
    primary_key: bool,
}

fn columns(db: &Connection, table: &str) -> Vec<ColumnInfo> {
    let query = format!("PRAGMA table_info({table})");
    let Ok(mut statement) = db.prepare(&query) else {
        return Vec::new();
    };
    let rows = statement.query_map([], |row| {
        Ok(ColumnInfo {
            name: row.get::<_, String>(1)?,
            notnull: row.get::<_, i64>(3)? != 0,
            has_default: !matches!(row.get_ref(4), Ok(rusqlite::types::ValueRef::Null) | Err(_)),
            primary_key: row.get::<_, i64>(5)? != 0,
        })
    });
    match rows {
        Ok(rows) => rows.filter_map(Result::ok).collect(),
        Err(_) => Vec::new(),
    }
}

fn first_user_message(session: &Session) -> String {
    for message in &session.messages {
        if message.role != "user" {
            continue;
        }
        let text = message
            .blocks
            .iter()
            .filter(|block| block.kind == BlockKind::Text && !block.text.is_empty())
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        if !text.is_empty() {
            return text;
        }
    }
    String::new()
}

fn bind(value: &Value) -> Box<dyn rusqlite::ToSql> {
    match value {
        Value::Null => Box::new(Option::<String>::None),
        Value::Bool(flag) => Box::new(i64::from(*flag)),
        Value::Number(number) => match number.as_i64() {
            Some(value) => Box::new(value),
            None => Box::new(number.as_f64().unwrap_or_default()),
        },
        Value::String(text) => Box::new(text.clone()),
        other => Box::new(crate::adapters::shared::writing::python_json_dumps(other)),
    }
}

/// 按 `PRAGMA table_info` 动态过滤列；出现未知的「必填且无默认值」列即报错。
fn insert(db: &Connection, table: &str, values: &Map<String, Value>) -> DomainResult<()> {
    let schema = columns(db, table);
    if schema.is_empty() {
        return Err(DomainError::internal(format!(
            "Codex 注册库缺少 {table} 表"
        )));
    }
    let missing: BTreeSet<&str> = schema
        .iter()
        .filter(|column| column.notnull && !column.has_default && !column.primary_key)
        .map(|column| column.name.as_str())
        .filter(|name| !values.contains_key(*name))
        .collect();
    if !missing.is_empty() {
        return Err(DomainError::internal(format!(
            "Codex 注册库包含不支持的必填字段: {}",
            missing.into_iter().collect::<Vec<_>>().join(", ")
        )));
    }
    let available: BTreeSet<&str> = schema.iter().map(|column| column.name.as_str()).collect();
    let names: Vec<&String> = values
        .keys()
        .filter(|name| available.contains(name.as_str()))
        .collect();
    let placeholders = vec!["?"; names.len()].join(",");
    let column_list = names
        .iter()
        .map(|name| name.as_str())
        .collect::<Vec<_>>()
        .join(",");
    let bound: Vec<Box<dyn rusqlite::ToSql>> =
        names.iter().map(|name| bind(&values[*name])).collect();
    let params: Vec<&dyn rusqlite::ToSql> = bound.iter().map(AsRef::as_ref).collect();
    db.execute(
        &format!("INSERT OR REPLACE INTO {table} ({column_list}) VALUES ({placeholders})"),
        params.as_slice(),
    )
    .map_err(|error| DomainError::internal(format!("Codex 注册写入失败: {error}")))?;
    Ok(())
}

fn now_parts() -> (i64, i64) {
    let delta = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    (delta.as_secs() as i64, delta.as_millis() as i64)
}

/// 注册 writer 已发布的节点树。
pub fn register_tree(
    state_db: &Path,
    nodes: &[RegistryNode<'_>],
    cli_version: &str,
) -> DomainResult<()> {
    if !state_db.exists() {
        return Err(DomainError::internal(format!(
            "Codex 注册库不存在: {}",
            state_db.display()
        )));
    }
    let (now, now_ms) = now_parts();
    let connection = Connection::open(state_db)
        .map_err(|error| DomainError::internal(format!("Codex 注册库打开失败: {error}")))?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|error| DomainError::internal(format!("Codex 注册库打开失败: {error}")))?;
    connection
        .execute_batch("PRAGMA foreign_keys=ON; BEGIN IMMEDIATE")
        .map_err(|error| DomainError::internal(format!("Codex 注册事务失败: {error}")))?;

    let outcome = (|| -> DomainResult<()> {
        for node in nodes {
            let first_user = first_user_message(node.session);
            let title = if node.session.title.is_empty() {
                first_user.chars().take(80).collect::<String>()
            } else {
                node.session.title.clone()
            };
            let source = match node.parent_id.as_deref() {
                None => Value::from("cli"),
                Some(parent) => {
                    let mut spawn = Map::new();
                    spawn.insert("parent_thread_id".into(), Value::from(parent));
                    spawn.insert("agent_path".into(), Value::from(node.agent_path.as_str()));
                    spawn.insert(
                        "agent_nickname".into(),
                        node.session
                            .agent_nickname
                            .as_deref()
                            .map_or(Value::Null, Value::from),
                    );
                    spawn.insert(
                        "agent_role".into(),
                        node.session
                            .agent_role
                            .as_deref()
                            .map_or(Value::Null, Value::from),
                    );
                    let mut subagent = Map::new();
                    subagent.insert("thread_spawn".into(), Value::Object(spawn));
                    let mut wrapper = Map::new();
                    wrapper.insert("subagent".into(), Value::Object(subagent));
                    Value::from(compact_json(&Value::Object(wrapper)))
                }
            };
            let mut row = Map::new();
            row.insert("id".into(), Value::from(node.session_id.as_str()));
            row.insert(
                "rollout_path".into(),
                Value::from(
                    std::fs::canonicalize(&node.path)
                        .unwrap_or_else(|_| node.path.clone())
                        .to_string_lossy()
                        .into_owned(),
                ),
            );
            row.insert("created_at".into(), Value::from(now));
            row.insert("updated_at".into(), Value::from(now));
            row.insert("created_at_ms".into(), Value::from(now_ms));
            row.insert("updated_at_ms".into(), Value::from(now_ms));
            row.insert("recency_at".into(), Value::from(now));
            row.insert("recency_at_ms".into(), Value::from(now_ms));
            row.insert("source".into(), source);
            row.insert(
                "model_provider".into(),
                Value::from(
                    node.session
                        .model_provider
                        .clone()
                        .filter(|value| !value.is_empty())
                        .unwrap_or_else(|| "openai".to_string()),
                ),
            );
            row.insert("cwd".into(), Value::from(node.cwd.as_str()));
            row.insert("title".into(), Value::from(title));
            row.insert(
                "sandbox_policy".into(),
                Value::from("{\"type\": \"read-only\"}"),
            );
            row.insert("approval_mode".into(), Value::from("on-request"));
            row.insert("tokens_used".into(), Value::from(0));
            row.insert(
                "has_user_event".into(),
                Value::from(i64::from(!first_user.is_empty())),
            );
            row.insert("archived".into(), Value::from(0));
            row.insert("cli_version".into(), Value::from(cli_version));
            row.insert(
                "first_user_message".into(),
                Value::from(first_user.as_str()),
            );
            row.insert(
                "agent_nickname".into(),
                node.session
                    .agent_nickname
                    .as_deref()
                    .map_or(Value::Null, Value::from),
            );
            row.insert(
                "agent_role".into(),
                node.session
                    .agent_role
                    .as_deref()
                    .map_or(Value::Null, Value::from),
            );
            row.insert("agent_path".into(), Value::from(node.agent_path.as_str()));
            row.insert(
                "thread_source".into(),
                Value::from(if node.parent_id.is_none() {
                    "user"
                } else {
                    "subagent"
                }),
            );
            row.insert("preview".into(), Value::from(first_user.as_str()));
            row.insert("history_mode".into(), Value::from("legacy"));
            insert(&connection, "threads", &row)?;
        }
        if !columns(&connection, "thread_spawn_edges").is_empty() {
            for node in nodes {
                let Some(parent) = node.parent_id.as_deref().filter(|id| !id.is_empty()) else {
                    continue;
                };
                let mut edge = Map::new();
                edge.insert("parent_thread_id".into(), Value::from(parent));
                edge.insert(
                    "child_thread_id".into(),
                    Value::from(node.session_id.as_str()),
                );
                edge.insert(
                    "status".into(),
                    Value::from(
                        node.status
                            .clone()
                            .filter(|status| !status.is_empty())
                            .unwrap_or_else(|| "closed".to_string()),
                    ),
                );
                insert(&connection, "thread_spawn_edges", &edge)?;
            }
        }
        Ok(())
    })();

    match outcome {
        Ok(()) => connection
            .execute_batch("COMMIT")
            .map_err(|error| DomainError::internal(format!("Codex 注册提交失败: {error}"))),
        Err(error) => {
            let _ = connection.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

/// `json.dumps(value, ensure_ascii=False, separators=(",", ":"))`。
fn compact_json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

/// 删除会话树在注册库中的痕迹；缺库/缺表都当成无事发生。
pub fn unregister_tree(state_db: Option<&Path>, session_ids: &BTreeSet<String>) {
    let Some(state_db) = state_db.filter(|path| path.exists()) else {
        return;
    };
    if session_ids.is_empty() {
        return;
    }
    let Ok(connection) = Connection::open(state_db) else {
        return;
    };
    let _ = connection.busy_timeout(Duration::from_secs(5));
    let placeholders = vec!["?"; session_ids.len()].join(",");
    let single: Vec<&dyn rusqlite::ToSql> = session_ids
        .iter()
        .map(|id| id as &dyn rusqlite::ToSql)
        .collect();
    if !columns(&connection, "thread_spawn_edges").is_empty() {
        let doubled: Vec<&dyn rusqlite::ToSql> =
            single.iter().chain(single.iter()).copied().collect();
        let _ = connection.execute(
            &format!(
                "DELETE FROM thread_spawn_edges WHERE parent_thread_id IN ({placeholders}) \
                 OR child_thread_id IN ({placeholders})"
            ),
            doubled.as_slice(),
        );
    }
    if !columns(&connection, "threads").is_empty() {
        let _ = connection.execute(
            &format!("DELETE FROM threads WHERE id IN ({placeholders})"),
            single.as_slice(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Block, Message};

    fn schema() -> &'static str {
        "CREATE TABLE threads (
             id TEXT PRIMARY KEY, rollout_path TEXT NOT NULL,
             created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
             source TEXT NOT NULL, model_provider TEXT NOT NULL, cwd TEXT NOT NULL,
             title TEXT NOT NULL, sandbox_policy TEXT NOT NULL,
             approval_mode TEXT NOT NULL, tokens_used INTEGER NOT NULL DEFAULT 0,
             has_user_event INTEGER NOT NULL DEFAULT 0, archived INTEGER NOT NULL DEFAULT 0,
             cli_version TEXT NOT NULL DEFAULT '', first_user_message TEXT NOT NULL DEFAULT '',
             agent_path TEXT, thread_source TEXT, preview TEXT NOT NULL DEFAULT '',
             recency_at INTEGER NOT NULL DEFAULT 0, history_mode TEXT NOT NULL DEFAULT 'legacy'
         );
         CREATE TABLE thread_spawn_edges (
             parent_thread_id TEXT NOT NULL, child_thread_id TEXT NOT NULL PRIMARY KEY,
             status TEXT NOT NULL
         );"
    }

    fn session_with_user(text: &str) -> Session {
        let mut session = Session::new("codex", "s", "/w");
        let mut message = Message::new("user");
        message.blocks.push(Block::text(text));
        session.messages.push(message);
        session
    }

    #[test]
    fn registration_filters_unknown_columns_and_records_edges() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("state_5.sqlite");
        Connection::open(&db_path)
            .unwrap()
            .execute_batch(schema())
            .unwrap();

        let root = session_with_user("hello");
        let child = session_with_user("child work");
        let nodes = vec![
            RegistryNode {
                session: &root,
                session_id: "r1".into(),
                path: temp.path().join("a.jsonl"),
                parent_id: None,
                cwd: "/w".into(),
                agent_path: "/root".into(),
                status: None,
            },
            RegistryNode {
                session: &child,
                session_id: "c1".into(),
                path: temp.path().join("b.jsonl"),
                parent_id: Some("r1".into()),
                cwd: "/w".into(),
                agent_path: "/root/docs".into(),
                status: Some("open".into()),
            },
        ];
        register_tree(&db_path, &nodes, "0.144.0").unwrap();

        let db = Connection::open(&db_path).unwrap();
        let (source, thread_source, title, has_user): (String, String, String, i64) = db
            .query_row(
                "SELECT source, thread_source, title, has_user_event FROM threads WHERE id='c1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert!(source.contains("\"parent_thread_id\":\"r1\""));
        assert_eq!(thread_source, "subagent");
        assert_eq!(title, "child work");
        assert_eq!(has_user, 1);
        let root_source: String = db
            .query_row("SELECT source FROM threads WHERE id='r1'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(root_source, "cli");
        let status: String = db
            .query_row(
                "SELECT status FROM thread_spawn_edges WHERE child_thread_id='c1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "open");
    }

    #[test]
    fn unknown_mandatory_columns_are_a_hard_failure() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("state_5.sqlite");
        Connection::open(&db_path)
            .unwrap()
            .execute_batch(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT NOT NULL,
                 brand_new_flag INTEGER NOT NULL);",
            )
            .unwrap();
        let root = session_with_user("hello");
        let nodes = vec![RegistryNode {
            session: &root,
            session_id: "r1".into(),
            path: temp.path().join("a.jsonl"),
            parent_id: None,
            cwd: "/w".into(),
            agent_path: "/root".into(),
            status: None,
        }];
        let error = register_tree(&db_path, &nodes, "").unwrap_err();
        assert!(error.message().contains("brand_new_flag"));
    }

    #[test]
    fn missing_registry_files_are_reported() {
        let temp = tempfile::tempdir().unwrap();
        let error = register_tree(&temp.path().join("nope.sqlite"), &[], "").unwrap_err();
        assert!(error.message().contains("Codex 注册库不存在"));
    }

    #[test]
    fn unregister_removes_threads_and_edges() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("state_5.sqlite");
        let db = Connection::open(&db_path).unwrap();
        db.execute_batch(schema()).unwrap();
        db.execute_batch(
            "INSERT INTO threads (id, rollout_path, created_at, updated_at, source,
             model_provider, cwd, title, sandbox_policy, approval_mode)
             VALUES ('r1','/a',0,0,'cli','openai','/w','t','{}','on-request');
             INSERT INTO thread_spawn_edges VALUES ('r1','c1','closed');",
        )
        .unwrap();
        drop(db);
        unregister_tree(
            Some(&db_path),
            &["r1".to_string()].into_iter().collect::<BTreeSet<_>>(),
        );
        let db = Connection::open(&db_path).unwrap();
        let threads: i64 = db
            .query_row("SELECT COUNT(*) FROM threads", [], |row| row.get(0))
            .unwrap();
        let edges: i64 = db
            .query_row("SELECT COUNT(*) FROM thread_spawn_edges", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!((threads, edges), (0, 0));
    }
}
