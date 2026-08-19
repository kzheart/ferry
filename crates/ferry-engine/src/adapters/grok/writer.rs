//! 生成当前形态的 Grok bundle，并维护它的 schema v4 搜索索引。
//!
//! 两件事被严格分开：
//! - **写 bundle**：每个节点先写进 `.{sid}.{pid}.tmp`，自读一遍 + 真实 grok CLI
//!   验收通过后才逆序 `rename` 发布并 fsync 父目录；任何一步失败都把临时目录与
//!   已发布目录一起删干净。
//! - **维护索引**：`session_search.sqlite` 是 Grok 自己的资产，动它之前逐项校验
//!   schema（版本、列元组、FTS5 建表片段、三个触发器片段），并先备份再写。

use std::fs::{self, File};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use rand::Rng as _;
use rusqlite::Connection;
use serde_json::{Map, Value};

use crate::adapters::shared::migration::RenderDecision;
use crate::adapters::shared::scanner::{parse_iso8601_ms, split_jsonl_lines};
use crate::adapters::shared::writing::{python_json_dumps, write_jsonl};
use crate::errors::{DomainError, DomainResult};
use crate::model::{tool_result_text, BlockKind, Message, Session, ToolCall, ToolResultStatus};
use crate::system::paths::{grok_home, home_dir, process_environ};

use super::blake3::blake3_hex;
use super::store::read_text;

/// 索引 schema 版本；不匹配即拒绝写入。
pub const SEARCH_SCHEMA_VERSION: &str = "4";

/// `session_docs` 的列元组，顺序即 `PRAGMA table_info` 的顺序。
pub const SEARCH_COLUMNS: [&str; 7] = [
    "session_id",
    "cwd",
    "updated_at",
    "title",
    "content",
    "content_hash",
    "last_indexed_offset",
];

/// 迁移判定回调：plan / preview / writer 三路共用同一个 `evaluate_tool`。
pub type ToolDecider<'a> =
    dyn Fn(&ToolCall, &Session, Option<&Message>) -> DomainResult<RenderDecision> + 'a;

// ---------------------------------------------------------------------------
// session_search.sqlite（schema v4）
// ---------------------------------------------------------------------------

const CREATE_STATEMENTS: [&str; 6] = [
    "CREATE TABLE meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        )",
    "CREATE TABLE session_docs (
            session_id TEXT PRIMARY KEY,
            cwd TEXT NOT NULL,
            updated_at INTEGER NOT NULL,
            title TEXT NOT NULL,
            content TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            last_indexed_offset INTEGER NOT NULL DEFAULT 0
        )",
    "CREATE VIRTUAL TABLE session_docs_fts USING fts5(
            title, content, content='session_docs', content_rowid='rowid'
        )",
    "CREATE TRIGGER session_docs_ai AFTER INSERT ON session_docs BEGIN
            INSERT INTO session_docs_fts(rowid, title, content)
            VALUES (new.rowid, new.title, new.content);
        END",
    "CREATE TRIGGER session_docs_ad AFTER DELETE ON session_docs BEGIN
            INSERT INTO session_docs_fts(session_docs_fts, rowid, title, content)
            VALUES ('delete', old.rowid, old.title, old.content);
        END",
    "CREATE TRIGGER session_docs_au AFTER UPDATE ON session_docs BEGIN
            INSERT INTO session_docs_fts(session_docs_fts, rowid, title, content)
            VALUES ('delete', old.rowid, old.title, old.content);
            INSERT INTO session_docs_fts(rowid, title, content)
            VALUES (new.rowid, new.title, new.content);
        END",
];

fn sqlite_error(error: &rusqlite::Error) -> DomainError {
    DomainError::internal(format!("Grok 搜索索引操作失败: {error}"))
}

fn unsupported_schema() -> DomainError {
    DomainError::internal("Grok session_search.sqlite 结构或版本不受支持")
}

fn create_search_schema(database: &Connection) -> DomainResult<()> {
    for statement in CREATE_STATEMENTS {
        database
            .execute_batch(statement)
            .map_err(|error| sqlite_error(&error))?;
    }
    database
        .execute(
            "INSERT INTO meta(key, value) VALUES (?, ?)",
            ("session_search_schema_version", SEARCH_SCHEMA_VERSION),
        )
        .map_err(|error| sqlite_error(&error))?;
    Ok(())
}

/// `re.sub(r"\s+", " ", str(value or "")).lower()`。
fn normalized_sql(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut in_space = false;
    for character in value.chars() {
        if character.is_whitespace() {
            in_space = true;
            continue;
        }
        // 边界空白也折成一个空格：`re.sub` 不吃掉首尾空白。
        if in_space {
            out.push(' ');
        }
        in_space = false;
        out.extend(character.to_lowercase());
    }
    if in_space {
        out.push(' ');
    }
    out
}

/// `(schema 版本, session_docs 列名, sqlite_schema 的 (type, name, sql) 三元组)`。
type SchemaSnapshot = (Option<String>, Vec<String>, Vec<(String, String, String)>);

/// 写前逐项校验：meta 版本、列元组、FTS 建表 SQL 片段、三个触发器片段。
fn validate_search_schema(database: &Connection) -> DomainResult<()> {
    let read = || -> rusqlite::Result<SchemaSnapshot> {
        let version = database
            .query_row(
                "SELECT value FROM meta WHERE key=?",
                ("session_search_schema_version",),
                |row| row.get::<_, String>(0),
            )
            .ok();
        let mut columns_statement = database.prepare("PRAGMA table_info(session_docs)")?;
        let columns: Vec<String> = columns_statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        let mut schema_statement = database.prepare(
            "SELECT type, name, sql FROM sqlite_schema
             WHERE name IN (
                 'session_docs_fts', 'session_docs_ai',
                 'session_docs_ad', 'session_docs_au'
             )",
        )?;
        let rows: Vec<(String, String, String)> = schema_statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok((version, columns, rows))
    };
    let (version, columns, rows) = read().map_err(|_| unsupported_schema())?;
    let sql_of = |kind: &str, name: &str| -> String {
        rows.iter()
            .find(|(row_kind, row_name, _)| row_kind == kind && row_name == name)
            .map(|(_, _, sql)| normalized_sql(sql))
            .unwrap_or_default()
    };
    let fts = sql_of("table", "session_docs_fts");
    let trigger_fragments: [(&str, &[&str]); 3] = [
        (
            "session_docs_ai",
            &[
                "after insert on session_docs",
                "insert into session_docs_fts",
            ],
        ),
        (
            "session_docs_ad",
            &[
                "after delete on session_docs",
                "values ('delete', old.rowid",
            ],
        ),
        (
            "session_docs_au",
            &[
                "after update on session_docs",
                "values ('delete', old.rowid",
                "values (new.rowid",
            ],
        ),
    ];
    let valid_triggers = trigger_fragments.iter().all(|(name, fragments)| {
        let sql = sql_of("trigger", name);
        fragments.iter().all(|fragment| sql.contains(fragment))
    });
    if version.as_deref() != Some(SEARCH_SCHEMA_VERSION)
        || columns != SEARCH_COLUMNS
        || !fts.contains("using fts5")
        || !fts.contains("content='session_docs'")
        || !fts.contains("content_rowid='rowid'")
        || !valid_triggers
    {
        return Err(unsupported_schema());
    }
    Ok(())
}

/// 主机名判别符：小写、非 `[a-z0-9]` 换成 `-`、截断 24 字符、去掉首尾 `-`。
fn host_discriminator() -> Option<String> {
    let raw = hostname().to_lowercase();
    let mapped: String = raw
        .chars()
        .map(|character| {
            if character.is_ascii_lowercase() || character.is_ascii_digit() {
                character
            } else {
                '-'
            }
        })
        .take(24)
        .collect();
    let trimmed = mapped.trim_matches('-');
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(unix)]
fn hostname() -> String {
    let mut buffer = [0 as libc::c_char; 256];
    // SAFETY: 缓冲区长度如实传入，gethostname 只写 buffer 内的字节。
    let code = unsafe { libc::gethostname(buffer.as_mut_ptr(), buffer.len()) };
    if code != 0 {
        return String::new();
    }
    let bytes: Vec<u8> = buffer
        .iter()
        .take_while(|byte| **byte != 0)
        .map(|byte| *byte as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(not(unix))]
fn hostname() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_default()
}

/// 网络文件系统上 SQLite 的 WAL 不可靠，Grok 会按主机名分片索引；探测方式与
/// Python 一致（`stat -f`）。
fn is_network_filesystem(path: &Path) -> bool {
    let display = path.to_string_lossy().into_owned();
    let command: Vec<String> = if cfg!(target_os = "macos") {
        vec!["/usr/bin/stat".into(), "-f".into(), "%T".into(), display]
    } else {
        vec![
            "stat".into(),
            "-f".into(),
            "-c".into(),
            "%T".into(),
            display,
        ]
    };
    let Ok(result) = crate::system::probes::run(&command, None, Duration::from_secs(3), None)
    else {
        return false;
    };
    let filesystem = result.stdout.trim().to_lowercase();
    ["nfs", "smb", "cifs", "afp", "webdav", "sshfs"]
        .iter()
        .any(|marker| filesystem.contains(marker))
}

fn search_database_path(sessions_root: &Path) -> PathBuf {
    let base = sessions_root.join("session_search.sqlite");
    let per_host = host_discriminator()
        .map(|host| sessions_root.join(format!("session_search.h-{host}.sqlite")));
    let override_mode = process_environ()
        .get("GROK_SQLITE_JOURNAL_MODE")
        .map(|value| value.to_lowercase())
        .unwrap_or_default();
    if override_mode == "truncate"
        || (override_mode.is_empty() && is_network_filesystem(sessions_root))
    {
        return per_host.unwrap_or(base);
    }
    if let Some(per_host) = per_host {
        if per_host.exists() && !base.exists() {
            return per_host;
        }
    }
    base
}

/// 索引路径不得是符号链接：跟着链接写会把别人的文件当成 Grok 的资产。
fn reject_database_symlink(path: &Path) -> DomainResult<()> {
    let is_symlink = fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink());
    if is_symlink {
        return Err(DomainError::internal("拒绝通过符号链接维护 Grok 搜索索引"));
    }
    Ok(())
}

/// 写前备份成 `*.ferry-backup`（0600、tmp + replace）。
///
/// Python 用 `sqlite3.Connection.backup`；rusqlite 的同名 API 在未启用的
/// `backup` feature 后面，这里用等价的 `VACUUM INTO`（同样产出一份完整、一致的
/// 数据库副本，且不需要目标已存在）。
fn backup_database(database: &Connection, path: &Path) -> DomainResult<PathBuf> {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let backup = path.with_file_name(format!("{name}.ferry-backup"));
    reject_database_symlink(&backup)?;
    let temporary = path.with_file_name(format!(
        ".{name}.ferry-backup.{}.{}.tmp",
        std::process::id(),
        uuid4_hex()
    ));
    database
        .execute("VACUUM INTO ?", (temporary.to_string_lossy(),))
        .map_err(|error| sqlite_error(&error))?;
    set_owner_only(&temporary)?;
    fs::rename(&temporary, &backup).map_err(|error| {
        DomainError::internal(format!(
            "Grok 搜索索引备份失败: {}: {error}",
            backup.display()
        ))
    })?;
    Ok(backup)
}

fn set_owner_only(path: &Path) -> DomainResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|error| {
            DomainError::internal(format!("设置备份权限失败: {}: {error}", path.display()))
        })?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|delta| delta.as_secs() as i64)
        .unwrap_or(0)
}

fn updated_at(summary: &Value) -> i64 {
    summary
        .get("updated_at")
        .and_then(Value::as_str)
        .and_then(parse_iso8601_ms)
        .map(|millis| millis / 1000)
        .unwrap_or_else(now_seconds)
}

/// 索引正文只取 `chat_history.jsonl`：它是 bundle 里唯一「一行一条完整消息」的
/// 视图，updates 的 chunk 流拼起来会重复计入同一段文本。
fn index_content(bundle: &Path) -> DomainResult<String> {
    let path = bundle.join("chat_history.jsonl");
    let text = read_text(&path).map_err(|error| {
        DomainError::internal(format!(
            "读取 Grok 会话文件失败: {}: {error}",
            path.display()
        ))
    })?;
    let mut parts: Vec<String> = Vec::new();
    for line in split_jsonl_lines(&text) {
        if line.trim().is_empty() {
            continue;
        }
        let row: Value = serde_json::from_str(line)
            .map_err(|error| DomainError::internal(format!("Grok chat 记录损坏: {error}")))?;
        match row.get("content") {
            Some(Value::String(text)) => parts.push(text.clone()),
            Some(Value::Array(items)) => parts.extend(
                items
                    .iter()
                    .filter(|item| item.get("type").and_then(Value::as_str) == Some("text"))
                    .map(|item| truthy_str(item.get("text"))),
            ),
            _ => {}
        }
        if let Some(tools) = row.get("tool_calls").and_then(Value::as_array) {
            for tool in tools.iter().filter(|tool| tool.is_object()) {
                parts.push(truthy_str(tool.get("name")));
                // `tool.get("arguments") or {}`：任何假值都退化成空对象。
                let arguments = tool
                    .get("arguments")
                    .filter(|value| truthy(value))
                    .cloned()
                    .unwrap_or_else(|| Value::Object(Map::new()));
                parts.push(python_json_dumps(&sorted_keys(&arguments)));
            }
        }
    }
    Ok(parts
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n"))
}

/// 等价 `str(value or "")`：假值（缺席 / null / 空串 / 0 / 空容器）都是空串。
fn truthy_str(value: Option<&Value>) -> String {
    value
        .filter(|value| truthy(value))
        .map(crate::adapters::shared::dialect::python_str)
        .unwrap_or_default()
}

/// 递归按 key 排序（等价 `json.dumps(..., sort_keys=True)`）。
fn sorted_keys(value: &Value) -> Value {
    match value {
        Value::Object(entries) => {
            let mut keys: Vec<&String> = entries.keys().collect();
            keys.sort_unstable();
            let mut sorted = Map::new();
            for key in keys {
                sorted.insert(key.clone(), sorted_keys(&entries[key]));
            }
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.iter().map(sorted_keys).collect()),
        other => other.clone(),
    }
}

/// 一条 `session_docs` 记录。
struct IndexDoc {
    session_id: String,
    cwd: String,
    updated_at: i64,
    title: String,
    content: String,
    content_hash: String,
    last_indexed_offset: i64,
}

fn index_doc(bundle: &Path) -> DomainResult<IndexDoc> {
    let summary_path = bundle.join("summary.json");
    let summary: Value = read_text(&summary_path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .ok_or_else(|| {
            DomainError::internal(format!("Grok summary 不可读: {}", summary_path.display()))
        })?;
    let info = summary.get("info").cloned().unwrap_or(Value::Null);
    let session_id = info
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let cwd = info
        .get("cwd")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    // `str(generated_title or session_summary or "")`。
    let title = summary
        .get("generated_title")
        .filter(|value| truthy(value))
        .or_else(|| summary.get("session_summary").filter(|value| truthy(value)))
        .map(crate::adapters::shared::dialect::python_str)
        .unwrap_or_default();
    let content = index_content(bundle)?;
    let mut payload = title.clone().into_bytes();
    payload.push(0);
    payload.extend_from_slice(content.as_bytes());
    let updates = bundle.join("updates.jsonl");
    let last_indexed_offset = fs::metadata(&updates)
        .map(|meta| meta.len() as i64)
        .map_err(|error| {
            DomainError::internal(format!(
                "读取 updates.jsonl 失败: {}: {error}",
                updates.display()
            ))
        })?;
    Ok(IndexDoc {
        session_id,
        cwd,
        updated_at: updated_at(&summary),
        title,
        content_hash: blake3_hex(&payload),
        content,
        last_indexed_offset,
    })
}

fn open_database(path: &Path) -> DomainResult<Connection> {
    let database = Connection::open(path).map_err(|error| sqlite_error(&error))?;
    database
        .busy_timeout(Duration::from_secs(5))
        .map_err(|error| sqlite_error(&error))?;
    database
        .execute_batch("PRAGMA busy_timeout=5000")
        .map_err(|error| sqlite_error(&error))?;
    Ok(database)
}

/// upsert 一批 bundle 的检索文档。
pub fn index_bundles(bundles: &[PathBuf], sessions_root: &Path) -> DomainResult<PathBuf> {
    fs::create_dir_all(sessions_root).map_err(|error| {
        DomainError::internal(format!(
            "创建 Grok 会话根目录失败: {}: {error}",
            sessions_root.display()
        ))
    })?;
    let database_path = search_database_path(sessions_root);
    reject_database_symlink(&database_path)?;
    let existed = database_path.exists();
    let database = open_database(&database_path)?;
    let outcome = (|| -> DomainResult<()> {
        if existed {
            validate_search_schema(&database)?;
            backup_database(&database, &database_path)?;
        }
        database
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|error| sqlite_error(&error))?;
        if !existed {
            create_search_schema(&database)?;
            validate_search_schema(&database)?;
        }
        for bundle in bundles {
            let doc = index_doc(bundle)?;
            database
                .execute(
                    "INSERT INTO session_docs(
                         session_id,cwd,updated_at,title,content,content_hash,
                         last_indexed_offset
                     ) VALUES(?,?,?,?,?,?,?)
                     ON CONFLICT(session_id) DO UPDATE SET
                         cwd=excluded.cwd, updated_at=excluded.updated_at,
                         title=excluded.title, content=excluded.content,
                         content_hash=excluded.content_hash,
                         last_indexed_offset=excluded.last_indexed_offset",
                    (
                        &doc.session_id,
                        &doc.cwd,
                        doc.updated_at,
                        &doc.title,
                        &doc.content,
                        &doc.content_hash,
                        doc.last_indexed_offset,
                    ),
                )
                .map_err(|error| sqlite_error(&error))?;
        }
        database
            .execute_batch("COMMIT")
            .map_err(|error| sqlite_error(&error))?;
        Ok(())
    })();
    if outcome.is_err() {
        let _ = database.execute_batch("ROLLBACK");
    }
    drop(database);
    // 建库失败时不留半截文件，否则下一轮会把它当成已存在的旧索引去校验。
    if outcome.is_err() && !existed {
        let _ = fs::remove_file(&database_path);
    }
    outcome.map(|()| database_path)
}

pub fn index_bundle(bundle: &Path, sessions_root: &Path) -> DomainResult<PathBuf> {
    index_bundles(std::slice::from_ref(&bundle.to_path_buf()), sessions_root)
}

/// 删除若干会话的索引行；索引不存在时是空操作。
pub fn delete_index_rows(session_ids: &[String], sessions_root: &Path) -> DomainResult<()> {
    let database_path = search_database_path(sessions_root);
    if !database_path.exists() {
        return Ok(());
    }
    reject_database_symlink(&database_path)?;
    let database = open_database(&database_path)?;
    let outcome = (|| -> DomainResult<()> {
        validate_search_schema(&database)?;
        backup_database(&database, &database_path)?;
        database
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|error| sqlite_error(&error))?;
        for session_id in session_ids {
            database
                .execute("DELETE FROM session_docs WHERE session_id=?", (session_id,))
                .map_err(|error| sqlite_error(&error))?;
        }
        database
            .execute_batch("COMMIT")
            .map_err(|error| sqlite_error(&error))?;
        Ok(())
    })();
    if outcome.is_err() {
        let _ = database.execute_batch("ROLLBACK");
    }
    outcome
}

// ---------------------------------------------------------------------------
// bundle 生成
// ---------------------------------------------------------------------------

fn uuid4_bytes() -> [u8; 16] {
    let mut bytes = [0u8; 16];
    rand::rng().fill(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    bytes
}

/// `uuid.uuid4().hex`。
fn uuid4_hex() -> String {
    uuid4_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// 隔离目录名里的一次性 token（`uuid.uuid4().hex`）。
pub(crate) fn cleanup_token() -> String {
    uuid4_hex()
}

/// `str(uuid.uuid4())`。
fn uuid4_string() -> String {
    let hex = uuid4_hex();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn write_json(path: &Path, value: &Value) -> DomainResult<()> {
    let mut stream = File::create(path).map_err(|error| {
        DomainError::internal(format!(
            "写入 Grok 会话文件失败: {}: {error}",
            path.display()
        ))
    })?;
    stream
        .write_all(python_json_dumps(value).as_bytes())
        .and_then(|()| stream.flush())
        .and_then(|()| stream.sync_all())
        .map_err(|error| {
            DomainError::internal(format!(
                "写入 Grok 会话文件失败: {}: {error}",
                path.display()
            ))
        })
}

/// 一个工具块在目标端的落地形态。
enum Rendered {
    /// 降级成历史叙述文本。
    Narration(String),
    /// 原生工具记录。
    Native {
        name: String,
        input: Value,
        output: String,
        status: String,
        has_result: bool,
    },
}

fn rendered_tool(
    tool: &ToolCall,
    session: &Session,
    message: &Message,
    decider: Option<&ToolDecider<'_>>,
) -> DomainResult<Rendered> {
    let fallback_output = tool_result_text(tool.result.as_ref());
    let rendered: Map<String, Value> = match decider {
        None => {
            let mut block = Map::new();
            block.insert("name".into(), Value::from(tool.name.as_str()));
            block.insert("input".into(), tool.input.clone());
            block.insert("output".into(), Value::from(fallback_output.as_str()));
            block
        }
        Some(decider) => {
            let decision = decider(tool, session, Some(message))?;
            match decision.rendered {
                None => {
                    let history = python_json_dumps(&tool.input);
                    let mut narration = format!("[Tool {}] {history}", tool.name);
                    if !fallback_output.is_empty() {
                        narration.push('\n');
                        narration.push_str(&fallback_output);
                    }
                    return Ok(Rendered::Narration(narration));
                }
                Some(rendered) => rendered,
            }
        }
    };
    let name = rendered
        .get("name")
        .filter(|value| truthy(value))
        .map(crate::adapters::shared::dialect::python_str)
        .unwrap_or_else(|| tool.name.clone());
    let input = rendered
        .get("input")
        .cloned()
        .unwrap_or_else(|| tool.input.clone());
    let output = rendered
        .get("output")
        .filter(|value| truthy(value))
        .map(crate::adapters::shared::dialect::python_str)
        .unwrap_or_else(|| fallback_output.clone());
    let status = match tool.result.as_ref().map(|result| result.status) {
        Some(ToolResultStatus::Success) => "Completed",
        Some(ToolResultStatus::Error) => "Failed",
        Some(ToolResultStatus::Pending) => "Pending",
        _ => "Completed",
    };
    Ok(Rendered::Native {
        name,
        input,
        output,
        status: status.to_string(),
        has_result: tool.result.is_some(),
    })
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|float| float != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(entries) => !entries.is_empty(),
    }
}

/// 降级工具块在用户/助手消息里的叙述形态。
fn narration_of(rendered: &Rendered) -> String {
    match rendered {
        Rendered::Narration(text) => text.clone(),
        Rendered::Native {
            name,
            input,
            output,
            ..
        } => format!("[Tool {name}] {}\n{output}", python_json_dumps(input))
            .trim_end()
            .to_string(),
    }
}

/// 生成一个节点的 `chat_history.jsonl` 与 `updates.jsonl` 记录。
fn native_rows(
    session: &Session,
    sid: &str,
    decider: Option<&ToolDecider<'_>>,
) -> DomainResult<(Vec<Value>, Vec<Value>)> {
    let mut chat: Vec<Value> = Vec::new();
    let mut updates: Vec<Value> = Vec::new();
    let mut prompt_index = 0i64;
    let model = session
        .model
        .clone()
        .filter(|model| !model.is_empty())
        .unwrap_or_else(|| "grok-code-fast-1".to_string());
    for message in &session.messages {
        if message.role == "user" {
            let mut content: Vec<Value> = Vec::new();
            for block in &message.blocks {
                match (block.kind, block.tool.as_ref()) {
                    (BlockKind::Text, _) => {
                        let mut part = Map::new();
                        part.insert("type".into(), Value::from("text"));
                        part.insert("text".into(), Value::from(block.text.as_str()));
                        content.push(Value::Object(part));
                    }
                    (BlockKind::Tool, Some(tool)) => {
                        let rendered = rendered_tool(tool, session, message, decider)?;
                        let mut part = Map::new();
                        part.insert("type".into(), Value::from("text"));
                        part.insert("text".into(), Value::from(narration_of(&rendered)));
                        content.push(Value::Object(part));
                    }
                    _ => {}
                }
            }
            let joined: String = content
                .iter()
                .filter_map(|item| item.get("text").and_then(Value::as_str))
                .collect();
            let mut row = Map::new();
            row.insert("type".into(), Value::from("user"));
            row.insert("id".into(), Value::from(uuid4_hex()));
            row.insert("content".into(), Value::Array(content));
            chat.push(Value::Object(row));
            updates.push(envelope(
                "session/update",
                sid,
                serde_json::json!({
                    "sessionUpdate": "user_message_chunk",
                    "content": {"type": "text", "text": joined},
                    "_meta": {"promptIndex": prompt_index, "modelId": model},
                }),
                serde_json::json!({"eventId": uuid4_hex()}),
            ));
            prompt_index += 1;
            continue;
        }
        let prompt_id = uuid4_hex();
        let mut assistant = Map::new();
        assistant.insert("type".into(), Value::from("assistant"));
        assistant.insert("id".into(), Value::from(uuid4_hex()));
        assistant.insert("content".into(), Value::from(""));
        assistant.insert("model_id".into(), Value::from(model.as_str()));
        let mut tool_calls: Vec<Value> = Vec::new();
        let mut tool_results: Vec<Value> = Vec::new();
        /// 助手行的 content 是**一整段**拼起来的文本（chunk 只在 updates 里）。
        fn push_text(assistant: &mut Map<String, Value>, text: &str) {
            let current = assistant
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            assistant.insert("content".into(), Value::from(current + text));
        }
        for block in &message.blocks {
            match (block.kind, block.tool.as_ref()) {
                (BlockKind::Text, _) if !block.text.is_empty() => {
                    push_text(&mut assistant, &block.text);
                    updates.push(envelope(
                        "session/update",
                        sid,
                        serde_json::json!({
                            "sessionUpdate": "agent_message_chunk",
                            "content": {"type": "text", "text": block.text},
                        }),
                        serde_json::json!({"eventId": uuid4_hex(), "promptId": prompt_id}),
                    ));
                }
                (BlockKind::Tool, Some(tool)) => {
                    let rendered = rendered_tool(tool, session, message, decider)?;
                    let Rendered::Native {
                        name,
                        input,
                        output,
                        status,
                        has_result,
                    } = rendered
                    else {
                        let narration = narration_of(&rendered);
                        push_text(&mut assistant, &narration);
                        updates.push(envelope(
                            "session/update",
                            sid,
                            serde_json::json!({
                                "sessionUpdate": "agent_message_chunk",
                                "content": {"type": "text", "text": narration},
                            }),
                            serde_json::json!({"eventId": uuid4_hex(), "promptId": prompt_id}),
                        ));
                        continue;
                    };
                    let call_id = tool
                        .source_call_id
                        .clone()
                        .filter(|value| !value.is_empty())
                        .unwrap_or_else(uuid4_hex);
                    tool_calls.push(serde_json::json!({
                        "id": call_id, "name": name,
                        "arguments": serde_json::to_string(&input)
                            .unwrap_or_else(|_| "{}".to_string()),
                    }));
                    updates.push(envelope(
                        "session/update",
                        sid,
                        serde_json::json!({
                            "sessionUpdate": "tool_call", "toolCallId": call_id,
                            "title": name, "kind": name, "status": "pending",
                            "rawInput": input,
                        }),
                        serde_json::json!({
                            "eventId": uuid4_hex(), "promptId": prompt_id,
                            "updateParams": {"toolCallId": call_id, "kind": name,
                                             "status": "Pending"},
                        }),
                    ));
                    if has_result {
                        updates.push(envelope(
                            "session/update",
                            sid,
                            serde_json::json!({
                                "sessionUpdate": "tool_call_update",
                                "toolCallId": call_id,
                                "content": [{"type": "text", "text": output}],
                                "rawOutput": output, "kind": name,
                                "status": status.to_lowercase(),
                            }),
                            serde_json::json!({
                                "eventId": uuid4_hex(), "promptId": prompt_id,
                                "updateParams": {"toolCallId": call_id, "kind": name,
                                                 "status": status},
                            }),
                        ));
                        tool_results.push(serde_json::json!({
                            "type": "tool_result", "id": uuid4_hex(),
                            "tool_call_id": call_id, "content": output,
                        }));
                    }
                }
                _ => {}
            }
        }
        if !tool_calls.is_empty() {
            assistant.insert("tool_calls".into(), Value::Array(tool_calls));
        }
        chat.push(Value::Object(assistant));
        chat.extend(tool_results);
    }
    Ok((chat, updates))
}

fn envelope(method: &str, sid: &str, update: Value, meta: Value) -> Value {
    serde_json::json!({
        "method": method,
        "params": {"sessionId": sid, "update": update, "_meta": meta},
    })
}

/// 父会话里代表子会话的 `_x.ai` 生成/结束事件对。
fn subagent_rows(parent_id: &str, children: &[(String, &Session)]) -> Vec<Value> {
    let mut rows = Vec::new();
    for (child_id, child) in children {
        let agent_id = child.agent_id.clone().unwrap_or_else(|| child_id.clone());
        let description = child
            .agent_role
            .clone()
            .filter(|role| !role.is_empty())
            .or_else(|| Some(child.title.clone()).filter(|title| !title.is_empty()))
            .unwrap_or_else(|| "Migrated child".to_string());
        rows.push(envelope(
            "_x.ai/session/update",
            parent_id,
            serde_json::json!({
                "sessionUpdate": "subagent_spawned",
                "subagent_id": agent_id,
                "parent_session_id": parent_id,
                "child_session_id": child_id,
                "subagent_type": child.agent_type.clone()
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| "general-purpose".to_string()),
                "description": description,
            }),
            serde_json::json!({"eventId": uuid4_hex()}),
        ));
        rows.push(envelope(
            "_x.ai/session/update",
            parent_id,
            serde_json::json!({
                "sessionUpdate": "subagent_finished",
                "subagent_id": agent_id, "child_session_id": child_id,
                "status": "completed", "tool_calls": 0,
                "turns": 0, "duration_ms": 0,
            }),
            serde_json::json!({"eventId": uuid4_hex()}),
        ));
    }
    rows
}

/// 前序遍历，同时记录父节点在结果里的下标（与 `Session::walk` 顺序一致）。
fn preorder(session: &Session) -> Vec<(&Session, Option<usize>)> {
    let mut visited: Vec<(&Session, Option<usize>)> = Vec::new();
    let mut stack: Vec<(&Session, Option<usize>)> = vec![(session, None)];
    while let Some((node, parent)) = stack.pop() {
        visited.push((node, parent));
        let index = visited.len() - 1;
        for child in node.children.iter().rev() {
            stack.push((child, Some(index)));
        }
    }
    visited
}

fn utc_now_iso() -> String {
    // `time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())`。
    let seconds = now_seconds();
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

/// Howard Hinnant 的 `civil_from_days`。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (year + i64::from(month <= 2), month, day)
}

fn remove_tree(path: &Path) {
    let _ = fs::remove_dir_all(path);
}

/// fsync 目录项，保证 rename 已经落盘。
fn fsync_directory(path: &Path) {
    if let Ok(handle) = File::open(path) {
        let _ = handle.sync_all();
    }
}

/// 把 canonical session 树写成 Grok bundle。
///
/// 返回 `(root_session_id, 根 bundle 目录)`。
pub fn write_bundle(
    session: &Session,
    cwd: &str,
    root: Option<&Path>,
    decider: Option<&ToolDecider<'_>>,
) -> DomainResult<(String, PathBuf)> {
    let sessions_root = match root {
        Some(root) => root.to_path_buf(),
        None => grok_home(&process_environ(), &home_dir()).join("sessions"),
    };
    let target_cwd = fs::canonicalize(cwd)
        .ok()
        .filter(|path| path.is_dir())
        .ok_or_else(|| DomainError::internal(format!("Grok 迁移目标目录不存在: {cwd}")))?;
    let nodes = preorder(session);
    let identifiers: Vec<String> = nodes.iter().map(|_| uuid4_string()).collect();
    let root_id = identifiers[0].clone();
    let node_cwd = target_cwd.to_string_lossy().into_owned();
    let project = sessions_root.join(utf8_percent_encode(&node_cwd, NON_ALPHANUMERIC).to_string());
    let now = utc_now_iso();

    let mut temporary_paths: Vec<PathBuf> = Vec::new();
    let mut destinations: Vec<PathBuf> = Vec::new();
    let mut published: Vec<PathBuf> = Vec::new();
    let outcome = (|| -> DomainResult<()> {
        for (index, (node, parent)) in nodes.iter().enumerate() {
            let sid = &identifiers[index];
            let destination = project.join(sid);
            let temporary = project.join(format!(".{sid}.{}.tmp", std::process::id()));
            fs::create_dir_all(&temporary).map_err(|error| {
                DomainError::internal(format!(
                    "创建 Grok 临时目录失败: {}: {error}",
                    temporary.display()
                ))
            })?;
            temporary_paths.push(temporary.clone());
            destinations.push(destination);
            let children: Vec<(String, &Session)> = nodes
                .iter()
                .enumerate()
                .filter(|(_, (_, child_parent))| *child_parent == Some(index))
                .map(|(child_index, (child, _))| (identifiers[child_index].clone(), *child))
                .collect();
            let (chat, mut updates) = native_rows(node, sid, decider)?;
            updates.extend(subagent_rows(sid, &children));
            let mut summary = Map::new();
            summary.insert(
                "info".into(),
                serde_json::json!({"id": sid, "cwd": node_cwd}),
            );
            let title = if node.title.is_empty() {
                "Migrated session"
            } else {
                node.title.as_str()
            };
            summary.insert("session_summary".into(), Value::from(title));
            summary.insert("generated_title".into(), Value::from(title));
            summary.insert("created_at".into(), Value::from(now.as_str()));
            summary.insert("updated_at".into(), Value::from(now.as_str()));
            summary.insert("num_messages".into(), Value::from(chat.len() as i64));
            summary.insert("num_chat_messages".into(), Value::from(chat.len() as i64));
            summary.insert(
                "current_model_id".into(),
                Value::from(
                    node.model
                        .clone()
                        .filter(|model| !model.is_empty())
                        .unwrap_or_else(|| "grok-code-fast-1".to_string()),
                ),
            );
            summary.insert("chat_format_version".into(), Value::from(1));
            summary.insert("root_session_id".into(), Value::from(root_id.as_str()));
            if let Some(parent) = parent {
                summary.insert(
                    "parent_session_id".into(),
                    Value::from(identifiers[*parent].as_str()),
                );
            }
            write_json(&temporary.join("summary.json"), &Value::Object(summary))?;
            let write = |name: &str, rows: &[Value]| -> DomainResult<()> {
                write_jsonl(&temporary.join(name), rows).map_err(|error| {
                    DomainError::internal(format!("写入 Grok {name} 失败: {error}"))
                })
            };
            write("updates.jsonl", &updates)?;
            write("chat_history.jsonl", &chat)?;
            // 自读一遍：生成的 bundle 必须能被自己的 reader 还原。
            super::reader::read(&temporary)?;
            let report = super::probe::probe_bundle(&temporary)?;
            if report.status != "passed" {
                return Err(DomainError::internal(format!(
                    "Grok CLI 无法验收生成会话: {}",
                    serde_json::to_string(&report.diagnostic).unwrap_or_default()
                )));
            }
        }
        // 逆序发布：子会话先就位，父会话最后可见，避免扫描到半棵树。
        for (temporary, destination) in temporary_paths.iter().zip(&destinations).rev() {
            fs::rename(temporary, destination).map_err(|error| {
                DomainError::internal(format!(
                    "发布 Grok 会话失败: {}: {error}",
                    destination.display()
                ))
            })?;
            if let Some(parent) = destination.parent() {
                fsync_directory(parent);
            }
            published.push(destination.clone());
        }
        index_bundles(&published, &sessions_root)?;
        Ok(())
    })();

    match outcome {
        Ok(()) => Ok((root_id, destinations[0].clone())),
        Err(error) => {
            for path in &temporary_paths {
                remove_tree(path);
            }
            for path in &published {
                remove_tree(path);
            }
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{text_tool_result, Block, ToolResult, ToolResultBlock, ToolResultBlockKind};
    use crate::tool_ops::CanonicalOp;
    use serde_json::json;

    fn source_session(cwd: &Path, title: &str) -> Session {
        let mut tool = ToolCall::new(
            "read",
            Some(CanonicalOp::TOOL_INVOKE.to_string()),
            json!({"namespace": "grok", "name": "read",
                   "input": {"path": "/raw/input.txt"}}),
        );
        tool.result = Some(text_tool_result("raw output", ToolResultStatus::Success));
        tool.source_call_id = Some("call-fixed".into());
        let mut session = Session::new("fixture", "source", cwd.to_string_lossy());
        session.title = title.to_string();
        let mut user = Message::new("user");
        user.blocks.push(Block::text("read input"));
        let mut assistant = Message::new("assistant");
        assistant.blocks.push(Block::text("before"));
        let mut tool_block = Block::new(BlockKind::Tool);
        tool_block.tool = Some(tool);
        assistant.blocks.push(tool_block);
        assistant.blocks.push(Block::text("after"));
        session.messages.push(user);
        session.messages.push(assistant);
        session
    }

    fn passing_probe() -> super::super::probe::ProbeGuard {
        super::super::probe::install_test_probe(|_| {
            Ok(crate::system::probes::report("passed", None, None, "", ""))
        })
    }

    fn failing_probe() -> super::super::probe::ProbeGuard {
        super::super::probe::install_test_probe(|_| {
            Ok(crate::system::probes::report("failed", None, None, "", ""))
        })
    }

    #[test]
    fn normalized_sql_collapses_whitespace_and_lowercases() {
        assert_eq!(normalized_sql("CREATE  TABLE\n  x"), "create table x");
        assert_eq!(normalized_sql(" a "), " a ");
        assert_eq!(normalized_sql(""), "");
    }

    #[test]
    fn a_fresh_index_passes_its_own_validation() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("session_search.sqlite");
        let database = Connection::open(&path).unwrap();
        create_search_schema(&database).unwrap();
        validate_search_schema(&database).unwrap();
        let version: String = database
            .query_row(
                "SELECT value FROM meta WHERE key='session_search_schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, "4");
    }

    #[test]
    fn schema_drift_is_rejected_item_by_item() {
        let cases: [&str; 4] = [
            // 版本不对。
            "UPDATE meta SET value='3' WHERE key='session_search_schema_version'",
            // 少一列。
            "DROP TABLE session_docs; CREATE TABLE session_docs(session_id TEXT PRIMARY KEY)",
            // FTS 不是 external content。
            "DROP TABLE session_docs_fts; CREATE VIRTUAL TABLE session_docs_fts \
             USING fts5(title, content)",
            // 触发器被换掉。
            "DROP TRIGGER session_docs_au; CREATE TRIGGER session_docs_au \
             AFTER UPDATE ON session_docs BEGIN SELECT 1; END",
        ];
        for statement in cases {
            let root = tempfile::tempdir().unwrap();
            let database = Connection::open(root.path().join("s.sqlite")).unwrap();
            create_search_schema(&database).unwrap();
            database.execute_batch(statement).unwrap();
            let error = validate_search_schema(&database).unwrap_err();
            assert_eq!(
                error.message(),
                "Grok session_search.sqlite 结构或版本不受支持"
            );
        }
    }

    #[test]
    fn writing_round_trips_and_indexes_the_search_document() {
        let _probe = passing_probe();
        let workspace = tempfile::tempdir().unwrap();
        let sessions = workspace.path().join("sessions");
        let source = source_session(workspace.path(), "sentinel-grok-writer");
        let target = super::super::migration::GrokMigrationTarget;
        let decider = |tool: &ToolCall, session: &Session, message: Option<&Message>| {
            use crate::adapters::shared::migration::MigrationTargetBase;
            target.evaluate_tool(tool, session, message)
        };
        let (sid, path) = write_bundle(
            &source,
            &workspace.path().to_string_lossy(),
            Some(&sessions),
            Some(&decider),
        )
        .unwrap();

        let migrated = super::super::reader::read(&path).unwrap();
        assert_eq!(migrated.source_id, sid);
        let kinds: Vec<BlockKind> = migrated.messages[1]
            .blocks
            .iter()
            .map(|block| block.kind)
            .collect();
        assert_eq!(kinds, [BlockKind::Text, BlockKind::Tool, BlockKind::Text]);
        let tool = migrated.messages[1].blocks[1].tool.as_ref().unwrap();
        assert_eq!(tool.name, "read");
        assert_eq!(
            tool.input,
            json!({"namespace": "grok", "name": "read",
                   "input": {"path": "/raw/input.txt"}})
        );
        assert_eq!(tool.result.as_ref().unwrap().blocks[0].text, "raw output");

        // 原生 chat 行里的 arguments 是无空格紧凑 JSON。
        let chat = read_text(&path.join("chat_history.jsonl")).unwrap();
        assert!(chat.contains(r#""arguments": "{\"path\":\"/raw/input.txt\"}""#));

        let database = Connection::open(sessions.join("session_search.sqlite")).unwrap();
        let (title, content, hash, offset): (String, String, String, i64) = database
            .query_row(
                "SELECT title, content, content_hash, last_indexed_offset
                 FROM session_docs WHERE session_id=?",
                (&sid,),
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(title, "sentinel-grok-writer");
        let mut payload = title.clone().into_bytes();
        payload.push(0);
        payload.extend_from_slice(content.as_bytes());
        assert_eq!(hash, blake3_hex(&payload));
        assert_eq!(
            offset,
            fs::metadata(path.join("updates.jsonl")).unwrap().len() as i64
        );
        // FTS 影子表跟着触发器同步。
        let hits: Vec<String> = database
            .prepare(
                "SELECT d.session_id FROM session_docs_fts
                 JOIN session_docs d ON d.rowid=session_docs_fts.rowid
                 WHERE session_docs_fts MATCH ?",
            )
            .unwrap()
            .query_map(("sentinel",), |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(hits, [sid]);
    }

    #[test]
    fn unicode_line_separators_survive_the_round_trip_and_the_index() {
        let _probe = passing_probe();
        let workspace = tempfile::tempdir().unwrap();
        let sessions = workspace.path().join("sessions");
        let mut source = source_session(workspace.path(), "sep");
        let content = "alpha\u{85}beta\u{2028}gamma\u{2029}omega";
        source.messages[0].blocks[0].text = content.to_string();
        let (sid, path) = write_bundle(
            &source,
            &workspace.path().to_string_lossy(),
            Some(&sessions),
            None,
        )
        .unwrap();
        let migrated = super::super::reader::read(&path).unwrap();
        assert_eq!(migrated.messages[0].blocks[0].text, content);
        let database = Connection::open(sessions.join("session_search.sqlite")).unwrap();
        let indexed: String = database
            .query_row(
                "SELECT content FROM session_docs WHERE session_id=?",
                (&sid,),
                |row| row.get(0),
            )
            .unwrap();
        assert!(indexed.contains(content));
    }

    #[test]
    fn tree_links_are_preserved_and_neighbours_are_untouched() {
        let _probe = passing_probe();
        let workspace = tempfile::tempdir().unwrap();
        let sessions = workspace.path().join("sessions");
        let existing = sessions.join("existing-project/existing");
        fs::create_dir_all(&existing).unwrap();
        let sentinel = existing.join("summary.json");
        fs::write(&sentinel, b"{\"existing\":true}\n").unwrap();

        let mut source = source_session(workspace.path(), "root-sentinel");
        let mut child = Session::new("fixture", "child", workspace.path().to_string_lossy());
        child.title = "child-sentinel".into();
        let mut message = Message::new("user");
        message.blocks.push(Block::text("child"));
        child.messages.push(message);
        source.children.push(child);

        let (root_id, root_path) = write_bundle(
            &source,
            &workspace.path().to_string_lossy(),
            Some(&sessions),
            None,
        )
        .unwrap();

        let summaries: Vec<Value> = walkdir::WalkDir::new(&sessions)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name() == "summary.json" && entry.path() != sentinel)
            .map(|entry| serde_json::from_str(&fs::read_to_string(entry.path()).unwrap()).unwrap())
            .collect();
        let child_summary = summaries
            .iter()
            .find(|summary| summary["info"]["id"] != json!(root_id))
            .unwrap();
        assert_eq!(child_summary["parent_session_id"], json!(root_id));
        assert_eq!(child_summary["root_session_id"], json!(root_id));
        let root_updates = read_text(&root_path.join("updates.jsonl")).unwrap();
        assert!(root_updates.contains(child_summary["info"]["id"].as_str().unwrap()));
        assert!(root_updates.contains("subagent_spawned"));
        assert!(root_updates.contains("subagent_finished"));
        assert_eq!(fs::read(&sentinel).unwrap(), b"{\"existing\":true}\n");
    }

    #[test]
    fn a_failing_probe_removes_every_generated_artifact() {
        let _probe = failing_probe();
        let workspace = tempfile::tempdir().unwrap();
        let sessions = workspace.path().join("sessions");
        let error = write_bundle(
            &source_session(workspace.path(), "x"),
            &workspace.path().to_string_lossy(),
            Some(&sessions),
            None,
        )
        .unwrap_err();
        assert!(error.message().contains("无法验收"));
        let leftovers: Vec<_> = walkdir::WalkDir::new(&sessions)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name() == "summary.json")
            .collect();
        assert!(leftovers.is_empty());
        assert!(!sessions.join("session_search.sqlite").exists());
    }

    #[test]
    fn a_broken_index_aborts_the_write_and_unpublishes_bundles() {
        let _probe = passing_probe();
        let workspace = tempfile::tempdir().unwrap();
        let sessions = workspace.path().join("sessions");
        fs::create_dir_all(&sessions).unwrap();
        // 一份 schema 不受支持的旧索引：写入必须整体失败。
        let database = Connection::open(sessions.join("session_search.sqlite")).unwrap();
        database
            .execute_batch("CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT)")
            .unwrap();
        drop(database);

        let error = write_bundle(
            &source_session(workspace.path(), "x"),
            &workspace.path().to_string_lossy(),
            Some(&sessions),
            None,
        )
        .unwrap_err();
        assert_eq!(
            error.message(),
            "Grok session_search.sqlite 结构或版本不受支持"
        );
        let leftovers: Vec<_> = walkdir::WalkDir::new(&sessions)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name() == "summary.json")
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn an_existing_index_is_backed_up_before_the_next_transaction() {
        let _probe = passing_probe();
        let workspace = tempfile::tempdir().unwrap();
        let sessions = workspace.path().join("sessions");
        let cwd = workspace.path().to_string_lossy().into_owned();
        let (first_id, _) = write_bundle(
            &source_session(workspace.path(), "first-sentinel"),
            &cwd,
            Some(&sessions),
            None,
        )
        .unwrap();
        let (second_id, _) = write_bundle(
            &source_session(workspace.path(), "second-sentinel"),
            &cwd,
            Some(&sessions),
            None,
        )
        .unwrap();
        let backup = sessions.join("session_search.sqlite.ferry-backup");
        assert!(backup.is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&backup).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let database = Connection::open(&backup).unwrap();
        let ids: Vec<String> = database
            .prepare("SELECT session_id FROM session_docs")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        // 备份是「上一轮事务之前」的状态。
        assert_eq!(ids, [first_id]);
        assert!(!ids.contains(&second_id));
    }

    #[test]
    fn symlinked_indexes_are_refused() {
        let root = tempfile::tempdir().unwrap();
        let sessions = root.path().join("sessions");
        fs::create_dir_all(&sessions).unwrap();
        let real = root.path().join("elsewhere.sqlite");
        fs::write(&real, b"").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, sessions.join("session_search.sqlite")).unwrap();
        #[cfg(unix)]
        {
            let error = delete_index_rows(&["s".to_string()], &sessions).unwrap_err();
            assert_eq!(error.message(), "拒绝通过符号链接维护 Grok 搜索索引");
        }
    }

    #[test]
    fn deleting_rows_keeps_the_fts_shadow_in_sync() {
        let _probe = passing_probe();
        let workspace = tempfile::tempdir().unwrap();
        let sessions = workspace.path().join("sessions");
        let (sid, _) = write_bundle(
            &source_session(workspace.path(), "doomed-sentinel"),
            &workspace.path().to_string_lossy(),
            Some(&sessions),
            None,
        )
        .unwrap();
        delete_index_rows(std::slice::from_ref(&sid), &sessions).unwrap();
        let database = Connection::open(sessions.join("session_search.sqlite")).unwrap();
        let remaining: i64 = database
            .query_row("SELECT count(*) FROM session_docs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, 0);
        let hits: i64 = database
            .query_row(
                "SELECT count(*) FROM session_docs_fts WHERE session_docs_fts MATCH ?",
                ("doomed",),
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(hits, 0);
    }

    #[test]
    fn the_tool_decider_can_degrade_a_call_to_narration() {
        let _probe = passing_probe();
        let workspace = tempfile::tempdir().unwrap();
        let sessions = workspace.path().join("sessions");
        let source = source_session(workspace.path(), "narrated");
        let decider = |_: &ToolCall, _: &Session, _: Option<&Message>| {
            Ok(RenderDecision::new(
                crate::adapters::shared::migration::Fidelity::Narrated,
            ))
        };
        let (_, path) = write_bundle(
            &source,
            &workspace.path().to_string_lossy(),
            Some(&sessions),
            Some(&decider),
        )
        .unwrap();
        let chat = read_text(&path.join("chat_history.jsonl")).unwrap();
        assert!(chat.contains("[Tool read]"));
        assert!(!chat.contains("tool_calls"));
    }

    #[test]
    fn index_content_sorts_tool_arguments() {
        let root = tempfile::tempdir().unwrap();
        let bundle = root.path().join("b");
        fs::create_dir_all(&bundle).unwrap();
        fs::write(
            bundle.join("chat_history.jsonl"),
            "{\"type\":\"assistant\",\"content\":\"hi\",\"tool_calls\":\
             [{\"name\":\"read\",\"arguments\":{\"b\":2,\"a\":1}}]}\n",
        )
        .unwrap();
        assert_eq!(
            index_content(&bundle).unwrap(),
            "hi\nread\n{\"a\": 1, \"b\": 2}"
        );
    }

    #[test]
    fn json_result_blocks_project_to_text_for_the_native_record() {
        let _probe = passing_probe();
        let workspace = tempfile::tempdir().unwrap();
        let mut source = source_session(workspace.path(), "json-result");
        let mut block = ToolResultBlock::new(ToolResultBlockKind::Json);
        block.data = json!({"a": 1});
        source.messages[1].blocks[1].tool.as_mut().unwrap().result = Some(ToolResult {
            status: ToolResultStatus::Success,
            blocks: vec![block],
            ..ToolResult::default()
        });
        let (_, path) = write_bundle(
            &source,
            &workspace.path().to_string_lossy(),
            Some(&workspace.path().join("sessions")),
            None,
        )
        .unwrap();
        let chat = read_text(&path.join("chat_history.jsonl")).unwrap();
        assert!(chat.contains(r#"{\"a\":1}"#));
    }
}
