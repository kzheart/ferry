//! Cursor 的 SQLite 边界。
//!
//! Cursor 全部会话住在一个 `state.vscdb` 里（本机 1.9 GB），且 IDE 在运行时持续
//! 以 WAL 写入。浏览路径**只读**：连接固定 `mode=ro` + `query_only`，从不 VACUUM、
//! 不加写锁、不落任何文件到 Cursor 的目录。
//!
//! 不用 `immutable=1`：那会让 SQLite 忽略 WAL，读到过期快照。
//!
//! 迁移写入路径（[`open_writable`]）是唯一的例外：它只**新增**本次迁移自己生成的
//! 键，从不改写 Cursor 已有的行，且必须先过 [`ensure_offline`] 这道门——Cursor
//! 在运行时会用内存态覆盖磁盘，写进去的会话会被它悄悄抹掉。

use std::path::{Path, PathBuf};
use std::sync::{LazyLock, RwLock};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use crate::errors::{DomainError, DomainResult};
use crate::system::paths::{cursor_database_path, home_dir, process_environ, Environ, OsFamily};
use crate::system::probes;

/// Ferry 当前支持的 Cursor 表结构。
///
/// 缺任何一列都视为 Cursor 换了存储结构，直接抛 `agent.format_changed`，
/// 不做「尽力而为」的兼容读取。`ItemTable` 只存 UI 状态，不参与解析。
pub const CURRENT_DB_COLUMNS: &[(&str, &[&str])] = &[
    (
        "composerHeaders",
        &[
            "composerId",
            "workspaceId",
            "createdAt",
            "lastUpdatedAt",
            "isArchived",
            "isSubagent",
            "recency",
            "checkpointAt",
            "value",
        ],
    ),
    ("cursorDiskKV", &["key", "value"]),
];

/// 测试用的库路径覆盖，避免单测去读开发机上真实的 Cursor 库。
static DB_PATH_OVERRIDE: LazyLock<RwLock<Option<PathBuf>>> = LazyLock::new(|| RwLock::new(None));

/// 当前 Cursor 会话库位置（`FERRY_CURSOR_DB` 优先）。
pub fn database_path() -> PathBuf {
    if let Some(path) = DB_PATH_OVERRIDE.read().expect("库路径覆盖锁中毒").clone() {
        return path;
    }
    cursor_database_path(OsFamily::current(), &process_environ(), &home_dir())
}

/// 覆盖 [`database_path`]；`None` 恢复按环境解析。
pub fn set_database_path_override(path: Option<PathBuf>) {
    *DB_PATH_OVERRIDE.write().expect("库路径覆盖锁中毒") = path;
}

/// 不含任何覆盖的平台默认库位置。
///
/// `FERRY_CURSOR_DB` 与测试覆盖都会把库指到别处；那种库不是「运行中的 Cursor
/// 正在写的那一个」，因此 [`ensure_offline`] 只对本函数返回的位置设防。
fn default_database_path() -> PathBuf {
    let mut environ: Environ = process_environ();
    environ.remove("FERRY_CURSOR_DB");
    cursor_database_path(OsFamily::current(), &environ, &home_dir())
}

/// 列出当前进程名的命令；Windows 用 `tasklist`，其余平台用 `ps`。
fn process_listing_argv() -> Vec<String> {
    let argv: &[&str] = if cfg!(windows) {
        &["tasklist", "/fo", "csv", "/nh"]
    } else {
        &["ps", "-A", "-o", "comm="]
    };
    argv.iter().map(|part| (*part).to_string()).collect()
}

/// 一行进程清单是否指向 Cursor 桌面端本体。
///
/// 只认可执行文件本体，不认 `cursor` CLI 转发器与 `cursor-agent`：前者是一闪而过
/// 的启动器，后者是另一个产品，都不会写这个库。
fn is_cursor_process(line: &str) -> bool {
    let line = line.trim().trim_matches('"');
    if line.is_empty() {
        return false;
    }
    // macOS：/Applications/Cursor.app/Contents/MacOS/Cursor（含 Helper 进程）。
    if line.contains("Cursor.app/") {
        return true;
    }
    let name = line.rsplit(['/', '\\']).next().unwrap_or(line);
    // Linux：/usr/share/cursor/cursor；Windows：Cursor.exe。
    name.eq_ignore_ascii_case("cursor.exe") || (name == "cursor" && line.contains('/'))
}

/// 迁移写入前的门禁：Cursor 必须已完全退出。
///
/// 探测失败（`ps` 不可用、超时）按「没在跑」处理：宁可让用户自己确认，也不要因为
/// 一条取不到的进程清单把整条迁移路径堵死。
pub fn ensure_offline() -> DomainResult<()> {
    if database_path() != default_database_path() {
        return Ok(());
    }
    let Ok(output) = probes::run(
        &process_listing_argv(),
        None,
        Duration::from_secs(5),
        Some(&[]),
    ) else {
        return Ok(());
    };
    if output.returncode != Some(0) || !output.stdout.lines().any(is_cursor_process) {
        return Ok(());
    }
    Err(DomainError::session_store_unavailable(
        "cursor",
        "Cursor 正在运行：请先完全退出 Cursor 再迁入，否则它会用内存态覆盖写入的会话",
    ))
}

/// 读写打开并校验结构；写入路径专用。
///
/// 不做整库备份：Cursor 的 `state.vscdb` 在本机是 1.9 GB，复制一份既慢又可能撑爆
/// 磁盘。写入只 INSERT 本次迁移新生成的键，回滚由 lifecycle 精确删除同一批键完成。
pub fn open_writable() -> DomainResult<Connection> {
    let path = database_path();
    if !path.exists() {
        return Err(DomainError::session_store_unavailable(
            "cursor",
            &format!("数据库不存在: {}", path.display()),
        ));
    }
    reject_symlink(&path)?;
    let connection = Connection::open(&path).map_err(|error| {
        DomainError::session_store_unavailable("cursor", &format!("数据库不可写入: {error}"))
    })?;
    connection
        .busy_timeout(Duration::from_secs(10))
        .map_err(|error| {
            DomainError::session_store_unavailable("cursor", &format!("设置超时失败: {error}"))
        })?;
    validate_schema(&connection)?;
    Ok(connection)
}

/// 库路径是符号链接时拒写：Cursor 自己不会这么放，出现即可疑。
fn reject_symlink(path: &Path) -> DomainResult<()> {
    let is_link = std::fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false);
    if is_link {
        return Err(DomainError::session_store_unavailable(
            "cursor",
            &format!("数据库是符号链接，拒绝写入: {}", path.display()),
        ));
    }
    Ok(())
}

/// 只读打开，不校验结构（扫描路径用：库缺失/损坏只让 cursor 一栏空着）。
pub fn open_readonly(path: &std::path::Path) -> rusqlite::Result<Connection> {
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let uri = format!("file:{}?mode=ro", resolved.display());
    let connection = Connection::open_with_flags(
        &uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    // IDE 正在写时不要立刻失败；只读连接不会阻塞它。
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.pragma_update(None, "query_only", 1)?;
    Ok(connection)
}

fn strings(values: &[&str]) -> Value {
    Value::Array(values.iter().map(|value| Value::from(*value)).collect())
}

/// 只读打开并严格校验 Ferry 当前支持的 Cursor 表结构。
pub fn open_database() -> DomainResult<Connection> {
    let path = database_path();
    if !path.exists() {
        return Err(DomainError::session_store_unavailable(
            "cursor",
            &format!("数据库不存在: {}", path.display()),
        ));
    }
    let connection = open_readonly(&path).map_err(|error| {
        DomainError::session_store_unavailable("cursor", &format!("数据库不可只读访问: {error}"))
    })?;
    validate_schema(&connection)?;
    Ok(connection)
}

/// 缺表或缺列即 `agent.format_changed`。
pub fn validate_schema(connection: &Connection) -> DomainResult<()> {
    for (table, required) in CURRENT_DB_COLUMNS {
        let columns = match table_columns(connection, table) {
            Ok(columns) => columns,
            Err(error) => {
                return Err(DomainError::agent_format_changed(
                    "cursor",
                    "sqlite.schema",
                    Value::from("readable current schema"),
                    Value::from(error.to_string()),
                ));
            }
        };
        if required
            .iter()
            .all(|name| columns.iter().any(|column| column == name))
        {
            continue;
        }
        let mut expected: Vec<&str> = required.to_vec();
        expected.sort_unstable();
        let mut actual: Vec<&str> = columns.iter().map(String::as_str).collect();
        actual.sort_unstable();
        return Err(DomainError::agent_format_changed(
            "cursor",
            &format!("sqlite.{table}"),
            strings(&expected),
            strings(&actual),
        ));
    }
    Ok(())
}

/// `PRAGMA table_info("<table>")` 的列名清单（按 cid 顺序）。
pub fn table_columns(connection: &Connection, table: &str) -> rusqlite::Result<Vec<String>> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info(\"{table}\")"))?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    rows.collect()
}

/// 取 `cursorDiskKV` 的一个键；缺失返回 `None`。
///
/// value 列虽是 BLOB 但实际是 UTF-8 文本，非法字节按 lossy 处理而不是丢整行。
pub fn disk_kv(connection: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    let mut statement =
        connection.prepare_cached("SELECT value FROM cursorDiskKV WHERE key = ?")?;
    let mut rows = statement.query([key])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    Ok(Some(text_cell(row.get_ref(0)?)))
}

/// SQLite 单元格 → 文本（TEXT / BLOB 两种落位都要认）。
pub fn text_cell(value: rusqlite::types::ValueRef<'_>) -> String {
    use rusqlite::types::ValueRef;
    match value {
        ValueRef::Text(bytes) | ValueRef::Blob(bytes) => {
            String::from_utf8_lossy(bytes).into_owned()
        }
        ValueRef::Integer(number) => number.to_string(),
        ValueRef::Real(number) => number.to_string(),
        ValueRef::Null => String::new(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    /// 库路径覆盖与指纹索引是进程级共享状态，触碰它们的单测必须串行。
    pub(crate) fn exclusive() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 程序化构造一个最小 `state.vscdb`。
    ///
    /// `fixture` 形如
    /// `{"sessions": [{"id", "header", "composerData", "bubbles": {id: bubble}}],
    ///   "kv": {key: 文本}}`；header 的列值从 header JSON 与 id 推导，缺省即 NULL，
    /// 这样单测可以只写关心的字段。
    pub(crate) fn materialize(path: &std::path::Path, fixture: &Value) {
        let connection = Connection::open(path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);\
                 CREATE TABLE cursorDiskKV (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);\
                 CREATE TABLE composerHeaders (composerId TEXT PRIMARY KEY, workspaceId TEXT, \
                 createdAt INTEGER, lastUpdatedAt INTEGER, isArchived INTEGER, \
                 isSubagent INTEGER, recency INTEGER, checkpointAt INTEGER, value TEXT);",
            )
            .unwrap();
        let optional_int = |head: &Value, key: &str| match head.get(key).and_then(Value::as_i64) {
            Some(number) => rusqlite::types::Value::Integer(number),
            None => rusqlite::types::Value::Null,
        };
        for session in fixture["sessions"].as_array().cloned().unwrap_or_default() {
            let id = session["id"].as_str().unwrap();
            let head = session.get("header").cloned().unwrap_or(json!({}));
            let created = head.get("createdAt").and_then(Value::as_i64).unwrap_or(0);
            let updated = head.get("lastUpdatedAt").and_then(Value::as_i64);
            connection
                .execute(
                    "INSERT INTO composerHeaders (composerId, workspaceId, createdAt, \
                     lastUpdatedAt, isArchived, isSubagent, recency, checkpointAt, value) \
                     VALUES (?,?,?,?,?,?,?,?,?)",
                    rusqlite::params![
                        id,
                        head.get("workspaceIdentifier")
                            .and_then(|value| value.get("id"))
                            .and_then(Value::as_str),
                        created,
                        optional_int(&head, "lastUpdatedAt"),
                        i64::from(head.get("isArchived").and_then(Value::as_bool) == Some(true)),
                        i64::from(session.get("subagent").and_then(Value::as_bool) == Some(true)),
                        updated.unwrap_or(created),
                        optional_int(&head, "conversationCheckpointLastUpdatedAt"),
                        head.to_string(),
                    ],
                )
                .unwrap();
            if let Some(data) = session.get("composerData") {
                connection
                    .execute(
                        "INSERT INTO cursorDiskKV (key, value) VALUES (?,?)",
                        rusqlite::params![format!("composerData:{id}"), data.to_string()],
                    )
                    .unwrap();
            }
            for (bubble_id, bubble) in session
                .get("bubbles")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default()
            {
                connection
                    .execute(
                        "INSERT INTO cursorDiskKV (key, value) VALUES (?,?)",
                        rusqlite::params![format!("bubbleId:{id}:{bubble_id}"), bubble.to_string()],
                    )
                    .unwrap();
            }
        }
        for (key, value) in fixture
            .get("kv")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default()
        {
            connection
                .execute(
                    "INSERT INTO cursorDiskKV (key, value) VALUES (?,?)",
                    rusqlite::params![key, crate::adapters::shared::dialect::python_str(&value)],
                )
                .unwrap();
        }
    }

    #[test]
    fn missing_columns_raise_agent_format_changed() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("state.vscdb");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE composerHeaders (composerId TEXT PRIMARY KEY, value TEXT);\
                 CREATE TABLE cursorDiskKV (key TEXT, value BLOB);",
            )
            .unwrap();
        drop(connection);
        let _guard = exclusive();
        set_database_path_override(Some(path));
        let error = open_database().unwrap_err();
        set_database_path_override(None);
        assert_eq!(error.code, "agent.format_changed");
        assert_eq!(error.params()["location"], json!("sqlite.composerHeaders"));
    }

    #[test]
    fn a_missing_database_is_store_unavailable() {
        let root = tempfile::tempdir().unwrap();
        let _guard = exclusive();
        set_database_path_override(Some(root.path().join("nope.vscdb")));
        let error = open_database().unwrap_err();
        set_database_path_override(None);
        assert_eq!(error.code, "session.store_unavailable");
    }

    #[test]
    fn only_the_desktop_binary_counts_as_a_running_cursor() {
        for line in [
            "/Applications/Cursor.app/Contents/MacOS/Cursor",
            "/Applications/Cursor.app/Contents/Frameworks/Cursor Helper (Renderer).app/Contents/MacOS/Cursor Helper (Renderer)",
            "/usr/share/cursor/cursor",
            "Cursor.exe",
            "\"Cursor.exe\"",
        ] {
            assert!(is_cursor_process(line), "{line}");
        }
        // 转发器 CLI、另一个产品与随便一个名字里带 cursor 的进程都不算。
        for line in [
            "cursor",
            "cursor-agent",
            "/usr/bin/cursor-agent",
            "",
            "code",
        ] {
            assert!(!is_cursor_process(line), "{line}");
        }
    }

    #[test]
    fn the_offline_gate_only_guards_the_real_cursor_location() {
        let _guard = exclusive();
        let root = tempfile::tempdir().unwrap();
        // 库被指到别处（测试 / FERRY_CURSOR_DB）时不做进程探测：那不是 Cursor 在写的库。
        set_database_path_override(Some(root.path().join("state.vscdb")));
        assert!(ensure_offline().is_ok());
        set_database_path_override(None);
    }

    #[test]
    fn writable_open_rejects_a_missing_database() {
        let _guard = exclusive();
        let root = tempfile::tempdir().unwrap();
        set_database_path_override(Some(root.path().join("nope.vscdb")));
        let error = open_writable().unwrap_err();
        set_database_path_override(None);
        assert_eq!(error.code, "session.store_unavailable");
    }

    #[test]
    fn writable_open_validates_the_schema_and_allows_inserts() {
        let _guard = exclusive();
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("state.vscdb");
        materialize(&path, &json!({"sessions": []}));
        set_database_path_override(Some(path.clone()));
        let connection = open_writable().unwrap();
        connection
            .execute(
                "INSERT INTO cursorDiskKV (key, value) VALUES ('probe', 'x')",
                [],
            )
            .unwrap();
        assert_eq!(disk_kv(&connection, "probe").unwrap().as_deref(), Some("x"));
        drop(connection);

        // 符号链接一律拒写。
        #[cfg(unix)]
        {
            let link = root.path().join("link.vscdb");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            set_database_path_override(Some(link));
            let error = open_writable().unwrap_err();
            assert!(error.message().contains("符号链接"));
        }
        set_database_path_override(None);
    }

    #[test]
    fn materialized_fixtures_pass_the_schema_gate() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("state.vscdb");
        materialize(
            &path,
            &json!({"sessions": [{"id": "s1", "header": {"name": "T"},
                                  "composerData": {"_v": 17}}]}),
        );
        let connection = open_readonly(&path).unwrap();
        validate_schema(&connection).unwrap();
        assert!(disk_kv(&connection, "composerData:s1").unwrap().is_some());
        assert!(disk_kv(&connection, "nope").unwrap().is_none());
    }
}
