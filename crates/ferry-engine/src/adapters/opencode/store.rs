//! OpenCode 当前 SQLite 存储与官方 CLI 边界。
//!
//! 三条写路径全部经过本模块的 [`NativeCli`]：导入走 `opencode import <file>`、
//! 删除走 `opencode session delete`、导出走 `opencode export`。单测通过
//! [`install_cli`] 换成假实现，等价 Python 侧对 `import_payload` /
//! `delete_session` 的 monkeypatch。

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, RwLock};
use std::time::Duration;

use rusqlite::Connection;
use serde_json::{Map, Value};

use crate::errors::{DomainError, DomainResult};
use crate::system::executables;
use crate::system::paths::{home_dir, opencode_database_path, process_environ, Platform};
use crate::system::probes;
use crate::system::sqlite;

/// `run_command` 的超时（Python `timeout=120`）。
const COMMAND_TIMEOUT: Duration = Duration::from_secs(120);

/// Ferry 当前支持的 OpenCode SQLite 列集合（`store.py:22-53`）。
///
/// 缺任何一列都视为 OpenCode 改了结构，直接抛 `agent.format_changed`，
/// 不做「尽力而为」的兼容读取。
pub const CURRENT_DB_COLUMNS: &[(&str, &[&str])] = &[
    (
        "session",
        &[
            "id",
            "slug",
            "project_id",
            "directory",
            "path",
            "title",
            "version",
            "summary_additions",
            "summary_deletions",
            "summary_files",
            "cost",
            "tokens_input",
            "tokens_output",
            "tokens_reasoning",
            "tokens_cache_read",
            "tokens_cache_write",
            "time_created",
            "time_updated",
            "parent_id",
            "agent",
            "model",
            "permission",
            "share_url",
            "revert",
            "time_archived",
            "time_compacting",
        ],
    ),
    ("message", &["id", "session_id", "data", "time_created"]),
    (
        "part",
        &["id", "message_id", "session_id", "data", "time_created"],
    ),
];

/// 测试用的库路径覆盖，等价 Python 侧 `monkeypatch.setattr(store, "DB_PATH", ...)`。
static DB_PATH_OVERRIDE: LazyLock<RwLock<Option<PathBuf>>> = LazyLock::new(|| RwLock::new(None));

/// 当前 OpenCode 会话库位置。
///
/// Python 在 import 期把 `DB_PATH` 固化成模块常量；Rust 没有 import 副作用，
/// 每次按环境重新解析（`FERRY_OPENCODE_DB` 优先），测试则用
/// [`set_database_path_override`] 精确指定。
pub fn database_path() -> PathBuf {
    if let Some(path) = DB_PATH_OVERRIDE.read().expect("库路径覆盖锁中毒").clone() {
        return path;
    }
    opencode_database_path(Platform::current(), &process_environ(), &home_dir())
}

/// 覆盖 [`database_path`]；`None` 恢复按环境解析。
pub fn set_database_path_override(path: Option<PathBuf>) {
    *DB_PATH_OVERRIDE.write().expect("库路径覆盖锁中毒") = path;
}

// ---------------------------------------------------------------------------
// 官方 CLI 边界
// ---------------------------------------------------------------------------

/// OpenCode 官方 CLI 的三条写/读路径。
pub trait NativeCli: Send + Sync {
    /// `opencode <args...>`，返回 stdout。
    fn run_command(&self, args: &[&str], cwd: Option<&Path>) -> DomainResult<String>;

    /// `opencode export <session_id>`。
    fn export_session(&self, session_id: &str) -> DomainResult<Value>;

    /// 临时 JSON + `opencode import <file>`。
    fn import_payload(&self, payload: &Value, session_id: &str, cwd: &str) -> DomainResult<()>;

    /// `opencode session delete <session_id>`。
    fn delete_session(&self, session_id: &str, cwd: Option<&str>) -> DomainResult<()>;
}

/// 真实实现：拉起 `opencode` 可执行文件。
pub struct SystemCli;

fn run_argv(argv: &[String], cwd: Option<&Path>) -> DomainResult<probes::CommandOutput> {
    probes::run(argv, cwd, COMMAND_TIMEOUT, None)
        .map_err(|error| DomainError::internal(error.message))
}

impl NativeCli for SystemCli {
    fn run_command(&self, args: &[&str], cwd: Option<&Path>) -> DomainResult<String> {
        let argv = executables::argv("opencode", args);
        let output = run_argv(&argv, cwd)?;
        if output.returncode != Some(0) {
            // Python 取 stderr 的**后** 400 字符。
            let tail: String = {
                let characters: Vec<char> = output.stderr.chars().collect();
                characters[characters.len().saturating_sub(400)..]
                    .iter()
                    .collect()
            };
            return Err(DomainError::internal(format!(
                "opencode {} 失败: {tail}",
                args.join(" ")
            )));
        }
        Ok(output.stdout)
    }

    fn export_session(&self, session_id: &str) -> DomainResult<Value> {
        let argv = executables::argv("opencode", &["export", session_id]);
        let output = run_argv(&argv, None)?;
        if output.returncode != Some(0) {
            let characters: Vec<char> = output.stderr.chars().collect();
            let tail: String = characters[characters.len().saturating_sub(400)..]
                .iter()
                .collect();
            return Err(DomainError::internal(format!(
                "opencode export 失败: {tail}"
            )));
        }
        serde_json::from_str(&output.stdout)
            .map_err(|error| DomainError::internal(format!("opencode export 输出非法: {error}")))
    }

    fn import_payload(&self, payload: &Value, session_id: &str, cwd: &str) -> DomainResult<()> {
        let mut temporary = tempfile::Builder::new()
            .prefix(&format!("rh-import-{session_id}-"))
            .suffix(".json")
            .tempfile()
            .map_err(|error| DomainError::internal(format!("import 临时文件创建失败: {error}")))?;
        let body = serde_json::to_string(payload).map_err(|error| {
            DomainError::internal(format!("import payload 序列化失败: {error}"))
        })?;
        temporary
            .write_all(body.as_bytes())
            .and_then(|()| temporary.flush())
            .map_err(|error| DomainError::internal(format!("import 临时文件写入失败: {error}")))?;
        let path = temporary.path().to_string_lossy().into_owned();
        let output = self.run_command(&["import", &path], Some(Path::new(cwd)))?;
        if !output.contains(session_id) {
            let characters: Vec<char> = output.chars().collect();
            let tail: String = characters[characters.len().saturating_sub(300)..]
                .iter()
                .collect();
            return Err(DomainError::internal(format!("import 结果异常: {tail}")));
        }
        Ok(())
    }

    fn delete_session(&self, session_id: &str, cwd: Option<&str>) -> DomainResult<()> {
        self.run_command(&["session", "delete", session_id], cwd.map(Path::new))?;
        Ok(())
    }
}

static CLI: LazyLock<RwLock<Arc<dyn NativeCli>>> =
    LazyLock::new(|| RwLock::new(Arc::new(SystemCli)));

/// 当前生效的 CLI 实现。
pub fn cli() -> Arc<dyn NativeCli> {
    CLI.read().expect("CLI 注册锁中毒").clone()
}

/// 换掉 CLI 实现（单测用）。
pub fn install_cli(implementation: Arc<dyn NativeCli>) {
    *CLI.write().expect("CLI 注册锁中毒") = implementation;
}

/// 恢复真实 CLI 实现。
pub fn reset_cli() {
    install_cli(Arc::new(SystemCli));
}

/// `opencode export <id>`（模块级便捷函数，等价 Python 的同名函数）。
pub fn export_session(session_id: &str) -> DomainResult<Value> {
    cli().export_session(session_id)
}

/// 临时 JSON + `opencode import`。
pub fn import_payload(payload: &Value, session_id: &str, cwd: &str) -> DomainResult<()> {
    cli().import_payload(payload, session_id, cwd)
}

/// `opencode session delete`。
pub fn delete_session(session_id: &str, cwd: Option<&str>) -> DomainResult<()> {
    cli().delete_session(session_id, cwd)
}

// ---------------------------------------------------------------------------
// 只读 SQLite
// ---------------------------------------------------------------------------

fn strings(values: &[&str]) -> Value {
    Value::Array(values.iter().map(|value| Value::from(*value)).collect())
}

/// 只读打开并严格校验 Ferry 当前支持的 OpenCode SQLite 结构。
pub fn open_database() -> DomainResult<Connection> {
    let path = database_path();
    if !path.exists() {
        return Err(DomainError::session_store_unavailable(
            "opencode",
            &format!("数据库不存在: {}", path.display()),
        ));
    }
    let connection = sqlite::open_readonly(&path)
        .and_then(|connection| {
            // Python 显式 `BEGIN`：整个读取过程锁定一个快照，避免读到半个写事务。
            connection.execute_batch("BEGIN")?;
            Ok(connection)
        })
        .map_err(|error| {
            DomainError::session_store_unavailable(
                "opencode",
                &format!("数据库不可只读访问: {error}"),
            )
        })?;

    for (table, required) in CURRENT_DB_COLUMNS {
        let columns = match table_columns(&connection, table) {
            Ok(columns) => columns,
            Err(error) => {
                return Err(DomainError::agent_format_changed(
                    "opencode",
                    "sqlite.schema",
                    Value::from("readable current schema"),
                    Value::from(error.to_string()),
                ));
            }
        };
        let mut missing: Vec<&str> = required
            .iter()
            .copied()
            .filter(|name| !columns.iter().any(|column| column == name))
            .collect();
        if !missing.is_empty() {
            missing.sort_unstable();
            let mut expected: Vec<&str> = required.to_vec();
            expected.sort_unstable();
            let mut actual: Vec<&str> = columns.iter().map(String::as_str).collect();
            actual.sort_unstable();
            return Err(DomainError::agent_format_changed(
                "opencode",
                &format!("sqlite.{table}"),
                strings(&expected),
                strings(&actual),
            ));
        }
    }
    Ok(connection)
}

/// `PRAGMA table_info("<table>")` 的列名清单（按 cid 顺序）。
pub fn table_columns(connection: &Connection, table: &str) -> rusqlite::Result<Vec<String>> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info(\"{table}\")"))?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    rows.collect()
}

/// SQLite 单元格 → JSON 值。BLOB 走 Python `default=str` 的等价形态。
pub fn cell_to_json(value: rusqlite::types::ValueRef<'_>) -> Value {
    use rusqlite::types::ValueRef;
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(number) => Value::from(number),
        ValueRef::Real(number) => Value::from(number),
        ValueRef::Text(bytes) => Value::from(String::from_utf8_lossy(bytes).into_owned()),
        ValueRef::Blob(bytes) => Value::from(format!("{bytes:?}")),
    }
}

fn format_changed(session_id: &str, kind: &str) -> DomainError {
    DomainError::agent_format_changed(
        "opencode",
        &format!("session.{session_id}"),
        Value::from("current session/message/part JSON"),
        Value::from(kind),
    )
}

fn integer_or_zero(value: &Value) -> Value {
    match value {
        Value::Null | Value::Bool(false) => Value::from(0),
        other => other.clone(),
    }
}

/// 官方 export 的 `info` 段：SQLite 25 列 → 嵌套 JSON。
fn session_info(row: &Map<String, Value>, session_id: &str) -> DomainResult<Value> {
    let get = |key: &str| row.get(key).cloned().unwrap_or(Value::Null);
    // Python: `float.is_integer()` 的浮点 cost 收敛成 int。
    let cost = match get("cost") {
        Value::Number(number) => match number.as_f64() {
            Some(float) if number.as_i64().is_none() && float.fract() == 0.0 => {
                Value::from(float as i64)
            }
            _ => Value::Number(number),
        },
        other => other,
    };
    let parse_json = |key: &str| -> DomainResult<Option<Value>> {
        match row.get(key) {
            Some(Value::String(text)) if !text.is_empty() => {
                Ok(Some(serde_json::from_str(text).map_err(|_| {
                    format_changed(session_id, "JSONDecodeError")
                })?))
            }
            _ => Ok(None),
        }
    };

    let mut summary = Map::new();
    summary.insert(
        "additions".into(),
        integer_or_zero(&get("summary_additions")),
    );
    summary.insert(
        "deletions".into(),
        integer_or_zero(&get("summary_deletions")),
    );
    summary.insert("files".into(), integer_or_zero(&get("summary_files")));

    let mut cache = Map::new();
    cache.insert("read".into(), get("tokens_cache_read"));
    cache.insert("write".into(), get("tokens_cache_write"));
    let mut tokens = Map::new();
    tokens.insert("input".into(), get("tokens_input"));
    tokens.insert("output".into(), get("tokens_output"));
    tokens.insert("reasoning".into(), get("tokens_reasoning"));
    tokens.insert("cache".into(), Value::Object(cache));

    let mut time = Map::new();
    time.insert("created".into(), get("time_created"));
    time.insert("updated".into(), get("time_updated"));

    let mut info = Map::new();
    info.insert("id".into(), get("id"));
    info.insert("slug".into(), get("slug"));
    info.insert("projectID".into(), get("project_id"));
    info.insert("directory".into(), get("directory"));
    info.insert(
        "path".into(),
        match get("path") {
            Value::String(text) if !text.is_empty() => Value::from(text),
            _ => Value::from(""),
        },
    );
    info.insert("title".into(), get("title"));
    info.insert("version".into(), get("version"));
    info.insert("summary".into(), Value::Object(summary));
    info.insert("cost".into(), cost);
    info.insert("tokens".into(), Value::Object(tokens));
    info.insert("time".into(), Value::Object(time));

    if let Value::String(parent) = get("parent_id") {
        if !parent.is_empty() {
            info.insert("parentID".into(), Value::from(parent));
        }
    }
    if let Value::String(agent) = get("agent") {
        if !agent.is_empty() {
            info.insert("agent".into(), Value::from(agent));
        }
    }
    if let Some(model) = parse_json("model")? {
        info.insert("model".into(), model);
    }
    if let Some(permission) = parse_json("permission")? {
        info.insert("permission".into(), permission);
    }
    if let Value::String(url) = get("share_url") {
        if !url.is_empty() {
            let mut share = Map::new();
            share.insert("url".into(), Value::from(url));
            info.insert("share".into(), Value::Object(share));
        }
    }
    if let Some(revert) = parse_json("revert")? {
        info.insert("revert".into(), revert);
    }
    for (column, key) in [
        ("time_archived", "archived"),
        ("time_compacting", "compacting"),
    ] {
        let value = get(column);
        let present = match &value {
            Value::Null | Value::Bool(false) => false,
            Value::Number(number) => number.as_f64().is_some_and(|float| float != 0.0),
            Value::String(text) => !text.is_empty(),
            _ => true,
        };
        if present {
            if let Some(Value::Object(time)) = info.get_mut("time") {
                time.insert(key.into(), value);
            }
        }
    }
    Ok(Value::Object(info))
}

fn row_map(row: &rusqlite::Row<'_>, columns: &[String]) -> Map<String, Value> {
    let mut entries = Map::new();
    for (index, name) in columns.iter().enumerate() {
        entries.insert(
            name.clone(),
            cell_to_json(
                row.get_ref(index)
                    .unwrap_or(rusqlite::types::ValueRef::Null),
            ),
        );
    }
    entries
}

/// 直读 SQLite 构造当前官方 export 形状 `{info, messages:[{info, parts:[]}]}`。
///
/// `None` 表示会话不存在（Python 返回 `None`，调用方翻成 `SessionNotFoundError`）。
pub fn export_from_database(
    connection: &Connection,
    session_id: &str,
) -> DomainResult<Option<Value>> {
    let columns = table_columns(connection, "session")
        .map_err(|_| format_changed(session_id, "DatabaseError"))?;
    let session_row: Option<Map<String, Value>> = {
        let mut statement = connection
            .prepare("SELECT * FROM session WHERE id = ?")
            .map_err(|_| format_changed(session_id, "DatabaseError"))?;
        let mut rows = statement
            .query([session_id])
            .map_err(|_| format_changed(session_id, "DatabaseError"))?;
        rows.next()
            .map_err(|_| format_changed(session_id, "DatabaseError"))?
            .map(|row| row_map(row, &columns))
    };
    let Some(session_row) = session_row else {
        return Ok(None);
    };

    let mut parts_by_message: Map<String, Value> = Map::new();
    {
        let mut statement = connection
            .prepare(
                "SELECT id, message_id, session_id, data FROM part \
                 WHERE session_id = ? ORDER BY time_created, id",
            )
            .map_err(|_| format_changed(session_id, "DatabaseError"))?;
        let mut rows = statement
            .query([session_id])
            .map_err(|_| format_changed(session_id, "DatabaseError"))?;
        while let Some(row) = rows
            .next()
            .map_err(|_| format_changed(session_id, "DatabaseError"))?
        {
            let id: String = row
                .get(0)
                .map_err(|_| format_changed(session_id, "KeyError"))?;
            let message_id: String = row
                .get(1)
                .map_err(|_| format_changed(session_id, "KeyError"))?;
            let owner: String = row
                .get(2)
                .map_err(|_| format_changed(session_id, "KeyError"))?;
            let blob: String = row
                .get(3)
                .map_err(|_| format_changed(session_id, "KeyError"))?;
            let mut data: Value = serde_json::from_str(&blob)
                .map_err(|_| format_changed(session_id, "JSONDecodeError"))?;
            let entries = data
                .as_object_mut()
                .ok_or_else(|| format_changed(session_id, "AttributeError"))?;
            entries.insert("id".into(), Value::from(id));
            entries.insert("sessionID".into(), Value::from(owner));
            entries.insert("messageID".into(), Value::from(message_id.clone()));
            match parts_by_message.get_mut(&message_id) {
                Some(Value::Array(items)) => items.push(data),
                _ => {
                    parts_by_message.insert(message_id, Value::Array(vec![data]));
                }
            }
        }
    }

    let mut messages = Vec::new();
    {
        let mut statement = connection
            .prepare(
                "SELECT id, session_id, data FROM message \
                 WHERE session_id = ? ORDER BY time_created, id",
            )
            .map_err(|_| format_changed(session_id, "DatabaseError"))?;
        let mut rows = statement
            .query([session_id])
            .map_err(|_| format_changed(session_id, "DatabaseError"))?;
        while let Some(row) = rows
            .next()
            .map_err(|_| format_changed(session_id, "DatabaseError"))?
        {
            let id: String = row
                .get(0)
                .map_err(|_| format_changed(session_id, "KeyError"))?;
            let owner: String = row
                .get(1)
                .map_err(|_| format_changed(session_id, "KeyError"))?;
            let blob: String = row
                .get(2)
                .map_err(|_| format_changed(session_id, "KeyError"))?;
            let mut data: Value = serde_json::from_str(&blob)
                .map_err(|_| format_changed(session_id, "JSONDecodeError"))?;
            let entries = data
                .as_object_mut()
                .ok_or_else(|| format_changed(session_id, "AttributeError"))?;
            entries.insert("id".into(), Value::from(id.clone()));
            entries.insert("sessionID".into(), Value::from(owner));
            let mut message = Map::new();
            message.insert("info".into(), data);
            message.insert(
                "parts".into(),
                parts_by_message
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| Value::Array(Vec::new())),
            );
            messages.push(Value::Object(message));
        }
    }

    let mut payload = Map::new();
    payload.insert("info".into(), session_info(&session_row, session_id)?);
    payload.insert("messages".into(), Value::Array(messages));
    Ok(Some(Value::Object(payload)))
}

/// 打开库、导出单个会话；不存在即 `SessionNotFoundError`。
pub fn load_native_payload(session_id: &str) -> DomainResult<Value> {
    let connection = open_database()?;
    export_from_database(&connection, session_id)?
        .ok_or_else(|| DomainError::session_not_found("opencode", session_id))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    /// opencode 的进程级状态（库路径覆盖、CLI 替身、指纹索引）是共享的，
    /// 触碰它们的单测必须串行。
    pub(crate) fn exclusive() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 读取 `tests/fixtures/agent_formats/opencode/<case>/session.json`。
    pub(crate) fn fixture(case: &str) -> Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/agent_formats/opencode")
            .join(case)
            .join("session.json");
        serde_json::from_str(&std::fs::read_to_string(path).expect("fixture 可读")).unwrap()
    }

    /// 按 fixture 的三张表行还原一个只读库（对齐
    /// `tests/golden_regen.rs` 的 opencode 分支）。
    pub(crate) fn materialize(path: &Path, fixture: &Value) {
        let session_columns: Vec<&str> = CURRENT_DB_COLUMNS[0].1.to_vec();
        let columns = session_columns
            .iter()
            .map(|name| format!("\"{name}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let connection = Connection::open(path).unwrap();
        connection
            .execute_batch(&format!(
                "CREATE TABLE session ({columns});\
                 CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT, data TEXT, \
                 time_created INTEGER);\
                 CREATE TABLE part (id TEXT PRIMARY KEY, message_id TEXT, session_id TEXT, \
                 data TEXT, time_created INTEGER);"
            ))
            .unwrap();
        let session = &fixture["session"];
        let placeholders = vec!["?"; session_columns.len()].join(",");
        let values: Vec<rusqlite::types::Value> = session_columns
            .iter()
            .map(|name| match session.get(*name) {
                Some(Value::String(text)) => rusqlite::types::Value::Text(text.clone()),
                Some(Value::Number(number)) if number.is_f64() => {
                    rusqlite::types::Value::Real(number.as_f64().unwrap())
                }
                Some(Value::Number(number)) => {
                    rusqlite::types::Value::Integer(number.as_i64().unwrap_or_default())
                }
                _ => rusqlite::types::Value::Null,
            })
            .collect();
        connection
            .execute(
                &format!("INSERT INTO session ({columns}) VALUES ({placeholders})"),
                rusqlite::params_from_iter(values),
            )
            .unwrap();
        for (index, row) in fixture["messages"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .enumerate()
        {
            connection
                .execute(
                    "INSERT INTO message (id, session_id, data, time_created) VALUES (?,?,?,?)",
                    rusqlite::params![
                        row["id"].as_str().unwrap(),
                        row["session_id"].as_str().unwrap(),
                        row["data"].as_str().unwrap(),
                        index as i64
                    ],
                )
                .unwrap();
        }
        for (index, row) in fixture["parts"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .enumerate()
        {
            connection
                .execute(
                    "INSERT INTO part (id, message_id, session_id, data, time_created) \
                     VALUES (?,?,?,?,?)",
                    rusqlite::params![
                        row["id"].as_str().unwrap(),
                        row["message_id"].as_str().unwrap(),
                        row["session_id"].as_str().unwrap(),
                        row["data"].as_str().unwrap(),
                        index as i64
                    ],
                )
                .unwrap();
        }
    }

    #[test]
    fn missing_columns_raise_agent_format_changed() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("opencode.db");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE session (id TEXT PRIMARY KEY);\
                 CREATE TABLE message (id TEXT, session_id TEXT, data TEXT);\
                 CREATE TABLE part (id TEXT, message_id TEXT, session_id TEXT, data TEXT);",
            )
            .unwrap();
        drop(connection);
        let _guard = exclusive();
        set_database_path_override(Some(path));
        let error = open_database().unwrap_err();
        set_database_path_override(None);
        assert_eq!(error.code, "agent.format_changed");
        assert_eq!(error.params()["location"], json!("sqlite.session"));
    }

    #[test]
    fn a_missing_database_is_store_unavailable() {
        let root = tempfile::tempdir().unwrap();
        let _guard = exclusive();
        set_database_path_override(Some(root.path().join("nope.db")));
        let error = open_database().unwrap_err();
        set_database_path_override(None);
        assert_eq!(error.code, "session.store_unavailable");
    }

    #[test]
    fn session_info_rebuilds_the_official_export_shape() {
        let mut row = Map::new();
        for (key, value) in [
            ("id", json!("ses_1")),
            ("slug", json!("demo")),
            ("project_id", json!("global")),
            ("directory", json!("/work")),
            ("path", json!(null)),
            ("title", json!("Demo")),
            ("version", json!("1.18.3")),
            ("summary_additions", json!(null)),
            ("summary_deletions", json!(2)),
            ("summary_files", json!(null)),
            ("cost", json!(3.0)),
            ("tokens_input", json!(10)),
            ("tokens_output", json!(20)),
            ("tokens_reasoning", json!(null)),
            ("tokens_cache_read", json!(0)),
            ("tokens_cache_write", json!(0)),
            ("time_created", json!(100)),
            ("time_updated", json!(200)),
            ("parent_id", json!("ses_0")),
            ("agent", json!("build")),
            ("model", json!("{\"providerID\":\"openai\"}")),
            ("permission", json!(null)),
            ("share_url", json!("https://share")),
            ("revert", json!(null)),
            ("time_archived", json!(null)),
            ("time_compacting", json!(300)),
        ] {
            row.insert(key.into(), value);
        }
        let info = session_info(&row, "ses_1").unwrap();
        assert_eq!(info["path"], json!(""));
        assert_eq!(
            info["summary"],
            json!({"additions": 0, "deletions": 2, "files": 0})
        );
        // 整数值的浮点 cost 收敛成整数。
        assert_eq!(info["cost"], json!(3));
        assert_eq!(info["parentID"], json!("ses_0"));
        assert_eq!(info["model"], json!({"providerID": "openai"}));
        assert_eq!(info["share"], json!({"url": "https://share"}));
        assert_eq!(info["time"]["compacting"], json!(300));
        assert!(info["time"].get("archived").is_none());
        assert!(info.get("permission").is_none());
    }
}
