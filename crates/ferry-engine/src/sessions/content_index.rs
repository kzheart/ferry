//! 跨会话正文的持久化全文索引。
//!
//! SQLite FTS5 + trigram 分词：中英文与代码都按任意子串命中。索引以 revision
//! 对账做增量——每次搜索只重建内容真正变过的会话，其余零 IO；首次全量构建在
//! 后台线程完成，期间搜索返回部分结果并如实上报覆盖度。
//!
//! `~/.ferry/content-index.sqlite3` 的表结构由 `user_version` 标识（当前是 2），
//! 改 schema 必须同时升它，否则旧库会被当成新库读。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use regex::Regex;
use rusqlite::{Connection, OptionalExtension};
use serde_json::{Map, Value};

use crate::errors::{DomainError, DomainResult};
use crate::model::{tool_result_text, BlockKind, Message, Session};
use crate::system::paths::home_dir;

use super::agent_read::{char_find, char_slice, read_indexed_session};
use super::index::{AgentSessionIndex, IndexedSession};

const SCHEMA_VERSION: i64 = 2;
/// trigram 至少要 3 个字符才能走倒排；更短的查询回退为子串扫描。
pub const MIN_TRIGRAM_CHARS: usize = 3;
/// 单条消息两列各自的入索引上限。
pub const RECORD_TEXT_CAP: usize = 16_000;
/// 待更新集小于这个规模就同步补完再查。
const SYNC_SESSION_LIMIT: usize = 32;
const SYNC_BYTE_LIMIT: i64 = 32 * 1024 * 1024;
/// 病态高频词的行数上限；命中即在 DTO 里标注 `rows_capped`。
const MAX_MATCH_ROWS: usize = 50_000;
const SNIPPET_BEFORE: usize = 120;
const SNIPPET_AFTER: usize = 240;
const MATCHES_PER_SESSION: usize = 3;

const SCHEMA_SQL: &str = r#"
            CREATE TABLE IF NOT EXISTS indexed_sessions(
                tool TEXT NOT NULL,
                ref TEXT NOT NULL,
                revision TEXT NOT NULL,
                record_rows INTEGER NOT NULL DEFAULT 0,
                clipped_rows INTEGER NOT NULL DEFAULT 0,
                failed INTEGER NOT NULL DEFAULT 0,
                indexed_at INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY(tool, ref)
            );
            CREATE TABLE IF NOT EXISTS records(
                id INTEGER PRIMARY KEY,
                tool TEXT NOT NULL,
                ref TEXT NOT NULL,
                message INTEGER NOT NULL,
                turn INTEGER NOT NULL,
                role TEXT NOT NULL,
                text TEXT NOT NULL DEFAULT '',
                tool_text TEXT NOT NULL DEFAULT '',
                clipped INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS records_by_session
                ON records(tool, ref);
            CREATE VIRTUAL TABLE IF NOT EXISTS records_fts USING fts5(
                text, tool_text,
                content='records', content_rowid='id',
                tokenize='trigram'
            );
            CREATE TRIGGER IF NOT EXISTS records_fts_insert
            AFTER INSERT ON records BEGIN
                INSERT INTO records_fts(rowid, text, tool_text)
                VALUES (new.id, new.text, new.tool_text);
            END;
            CREATE TRIGGER IF NOT EXISTS records_fts_delete
            AFTER DELETE ON records BEGIN
                INSERT INTO records_fts(records_fts, rowid, text, tool_text)
                VALUES ('delete', old.id, old.text, old.tool_text);
            END;
"#;

/// `(tool, canonical_ref)`。
pub type SessionKey = (String, String);

/// 一个会话的内容命中桶。
#[derive(Clone, Debug, Default)]
pub struct ContentHit {
    pub count: i64,
    /// bm25 越小越相关；`None` 表示子串扫描（无排名）。
    pub best_rank: Option<f64>,
    pub rows: Vec<Map<String, Value>>,
}

/// 一条消息拆成正文列与工具输出列；第三个返回值是「有内容被 16 KB 截断」。
fn extract(message: &Message) -> (String, String, bool) {
    let mut texts: Vec<&str> = Vec::new();
    let mut tools: Vec<String> = Vec::new();
    for block in &message.blocks {
        if block.kind == BlockKind::Text && !block.text.is_empty() {
            texts.push(&block.text);
        } else if block.kind == BlockKind::Tool {
            let Some(call) = block.tool.as_ref() else {
                continue;
            };
            tools.push(format!("[tool {}]", call.name));
            let output = tool_result_text(call.result.as_ref());
            if !output.is_empty() {
                tools.push(output);
            }
        }
    }
    let text = texts.join("\n");
    let tool_text = tools.join("\n");
    let clipped =
        text.chars().count() > RECORD_TEXT_CAP || tool_text.chars().count() > RECORD_TEXT_CAP;
    (
        char_slice(&text, 0, RECORD_TEXT_CAP),
        char_slice(&tool_text, 0, RECORD_TEXT_CAP),
        clipped,
    )
}

struct RecordRow {
    message: i64,
    turn: i64,
    role: String,
    text: String,
    tool_text: String,
    clipped: i64,
}

/// message/turn 编号与 `session_read` 完全同口径，命中可直接跳读。
fn session_rows(session: &Session) -> Vec<RecordRow> {
    let mut rows = Vec::new();
    let mut turn = 0i64;
    for (message_index, message) in session.messages.iter().enumerate() {
        if message.role == "user" {
            turn += 1;
        }
        let (text, tool_text, clipped) = extract(message);
        if text.is_empty() && tool_text.is_empty() {
            continue;
        }
        rows.push(RecordRow {
            message: message_index as i64 + 1,
            turn,
            role: message.role.clone(),
            text,
            tool_text,
            clipped: i64::from(clipped),
        });
    }
    rows
}

fn like_pattern(needle: &str) -> String {
    let escaped = needle
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}

/// bm25 越小越相关；`None` 排最后。
fn rank_is_better(new: Option<f64>, old: Option<f64>) -> bool {
    match (new, old) {
        (None, _) => false,
        (Some(_), None) => true,
        (Some(new), Some(old)) => new < old,
    }
}

static QUERY_TERM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""([^"]+)"|(\S+)"#).expect("查询分词正则必须可编译"));

/// 把查询拆成词级 AND 的检索词；支持 `"..."` 精确短语。
///
/// 布尔操作符不支持，裸的 `OR`/`AND`/`NOT` 按噪声丢弃。
pub fn parse_query_terms(query: &str) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();
    for captures in QUERY_TERM.captures_iter(query) {
        let term = captures
            .get(1)
            .or_else(|| captures.get(2))
            .map(|matched| matched.as_str())
            .unwrap_or("");
        if matches!(term, "OR" | "AND" | "NOT") {
            continue;
        }
        terms.push(term.to_string());
    }
    if terms.is_empty() {
        return vec![query.trim().to_string()];
    }
    terms
}

fn fts_phrase(term: &str) -> String {
    format!("\"{}\"", term.replace('"', "\"\""))
}

fn columns_for(include_tool_outputs: bool) -> &'static str {
    if include_tool_outputs {
        "{text tool_text}"
    } else {
        "{text}"
    }
}

/// 一行原始查询结果。
#[derive(Clone, Debug)]
struct MatchRow {
    id: i64,
    tool: String,
    reference: String,
    message: i64,
    turn: i64,
    role: String,
    rank: Option<f64>,
}

struct Database {
    connection: Option<Connection>,
    unavailable: Option<String>,
    closed: bool,
}

/// 写事务：任一步失败立刻 ROLLBACK。
///
/// Python 侧是 `with connection:`（正常提交、异常回滚，隐式 deferred BEGIN）。
/// 这里的连接是共享的 `&Connection`（拿不到 `&mut`，用不了 rusqlite 的
/// `Transaction`），漏掉 ROLLBACK 会把一个打开的事务留给同一句柄的后续所有
/// 调用——下一次 `BEGIN` 报「transaction within a transaction」，写入全落进那个
/// 泄漏的事务里。
fn write_transaction(
    connection: &Connection,
    body: impl FnOnce(&Connection) -> rusqlite::Result<()>,
) -> rusqlite::Result<()> {
    connection.execute_batch("BEGIN")?;
    match body(connection) {
        Ok(()) => connection.execute_batch("COMMIT"),
        Err(error) => {
            let _ = connection.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

#[derive(Default)]
struct WorkerState {
    /// 按插入序去重；`popitem()` 取**最后**一项（LIFO）。
    queued: Vec<(SessionKey, IndexedSession)>,
    running: bool,
    closed: bool,
}

pub struct ContentIndex {
    path: PathBuf,
    database: Mutex<Database>,
    worker: Mutex<WorkerState>,
}

impl ContentIndex {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self {
            path: path.unwrap_or_else(|| home_dir().join(".ferry").join("content-index.sqlite3")),
            database: Mutex::new(Database {
                connection: None,
                unavailable: None,
                closed: false,
            }),
            worker: Mutex::new(WorkerState::default()),
        }
    }

    /// 在已加锁的数据库句柄上执行闭包。
    ///
    /// 两种失败**必须分开**，与 Python 的 `_db()` + `connection.execute(...)`
    /// 两段式逐条对齐：
    /// - `Ok(None)`：索引不可用/已关闭（Python 的 `_db()` 只在**建连与建表**
    ///   失败时吞掉异常并置 `_unavailable`，调用方按「没有内容索引」降级）；
    /// - `Err(..)`：查询本身失败（Python 里 `sqlite3.Error` 一路抛到
    ///   `search_sessions`，落成 `internal.unexpected` 的 RPC 错误）。把后者也
    ///   降级成 `None` 会让检索静默退回全量扫描却仍上报 `ready:true`。
    fn with_db<T>(
        &self,
        action: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> DomainResult<Option<T>> {
        let mut guard = self
            .database
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if guard.unavailable.is_some() || guard.closed {
            return Ok(None);
        }
        if guard.connection.is_none() {
            match Self::open(&self.path) {
                Ok(connection) => guard.connection = Some(connection),
                Err(error) => {
                    guard.unavailable = Some(format!("content index unavailable: {error}"));
                    return Ok(None);
                }
            }
        }
        let connection = guard.connection.as_ref().expect("上一步已装载");
        action(connection)
            .map(Some)
            .map_err(|error| DomainError::internal(format!("内容索引查询失败: {error}")))
    }

    fn open(path: &PathBuf) -> rusqlite::Result<Connection> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| rusqlite::Error::InvalidPath(PathBuf::from(error.to_string())))?;
        }
        let connection = Connection::open(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        Self::ensure_schema(&connection)?;
        Ok(connection)
    }

    fn ensure_schema(connection: &Connection) -> rusqlite::Result<()> {
        let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version != SCHEMA_VERSION {
            connection.execute_batch(
                "DROP TABLE IF EXISTS records_fts;
                 DROP TABLE IF EXISTS records;
                 DROP TABLE IF EXISTS indexed_sessions;",
            )?;
        }
        connection.execute_batch(SCHEMA_SQL)?;
        connection.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        Ok(())
    }

    /// 索引不可用的原因（`None` = 可用或尚未尝试打开）。
    pub fn unavailable_reason(&self) -> Option<String> {
        self.database
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .unavailable
            .clone()
    }

    pub fn close(&self) {
        {
            let mut worker = self
                .worker
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            worker.queued.clear();
            worker.closed = true;
        }
        // 后台线程见 closed 即返回；最多等 5 秒。
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.building() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut guard = self
            .database
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.closed = true;
        guard.connection = None;
    }

    // ---------- 增量同步 ----------

    /// 按 revision 对账并返回覆盖度状态。
    ///
    /// 小变更同步补完（保证刚写入的内容立即可搜）；大变更/首建转后台，调用方
    /// 拿到 `pending_sessions` 用于告知模型内容结果是部分的。
    pub fn sync(
        self: &Arc<Self>,
        index: &Arc<AgentSessionIndex>,
        records: &[IndexedSession],
        prefer_background: bool,
    ) -> DomainResult<Map<String, Value>> {
        if self.with_db(|_| Ok(()))?.is_none() {
            let mut status = Map::new();
            status.insert("ready".into(), Value::Bool(false));
            status.insert(
                "reason".into(),
                self.unavailable_reason()
                    .map(Value::from)
                    .unwrap_or(Value::Null),
            );
            return Ok(status);
        }
        let stored = self.stored_revisions()?;
        let mut current: Vec<SessionKey> = Vec::with_capacity(records.len());
        let mut pending: Vec<IndexedSession> = Vec::new();
        for record in records {
            let key = (record.tool.clone(), record.canonical_ref.clone());
            current.push(key.clone());
            if stored.get(&key).map(String::as_str) != Some(record.revision.as_str()) {
                pending.push(record.clone());
            }
        }
        let stale: Vec<SessionKey> = stored
            .keys()
            .filter(|key| !current.contains(key))
            .cloned()
            .collect();
        if !stale.is_empty() {
            self.drop_sessions(&stale)?;
        }
        if !pending.is_empty() {
            let total_bytes: i64 = pending
                .iter()
                .map(|record| record.row.get("size").and_then(Value::as_i64).unwrap_or(0))
                .sum();
            if !prefer_background
                && !self.building()
                && pending.len() <= SYNC_SESSION_LIMIT
                && total_bytes <= SYNC_BYTE_LIMIT
            {
                for record in &pending {
                    // 前台补完路径不吞 sqlite 错误（Python 的 `sync` 也不捕获）。
                    self.index_session(index, record)?;
                }
                pending.clear();
            } else {
                self.enqueue(index, &pending);
            }
        }
        let mut status = Map::new();
        status.insert(
            "ready".into(),
            Value::Bool(pending.is_empty() && !self.building()),
        );
        status.insert(
            "indexed_sessions".into(),
            Value::from(records.len() - pending.len()),
        );
        status.insert("pending_sessions".into(), Value::from(pending.len()));
        Ok(status)
    }

    /// 只读覆盖度：与 [`Self::sync`] 同口径统计，但不写库、不入队、不清陈旧。
    ///
    /// `daemon.status` 与 `ferry scan --wait` 靠它判断内容索引是否已就绪；
    /// 状态查询必须无副作用，所以不能借道 `sync`。
    pub fn coverage(&self, records: &[IndexedSession]) -> DomainResult<Map<String, Value>> {
        let mut status = Map::new();
        if self.with_db(|_| Ok(()))?.is_none() {
            status.insert("ready".into(), Value::Bool(false));
            status.insert(
                "reason".into(),
                self.unavailable_reason()
                    .map(Value::from)
                    .unwrap_or(Value::Null),
            );
            return Ok(status);
        }
        let stored = self.stored_revisions()?;
        let pending = records
            .iter()
            .filter(|record| {
                stored
                    .get(&(record.tool.clone(), record.canonical_ref.clone()))
                    .map(String::as_str)
                    != Some(record.revision.as_str())
            })
            .count();
        status.insert(
            "ready".into(),
            Value::Bool(pending == 0 && !self.building()),
        );
        status.insert(
            "indexed_sessions".into(),
            Value::from(records.len() - pending),
        );
        status.insert("pending_sessions".into(), Value::from(pending));
        status.insert("building".into(), Value::Bool(self.building()));
        Ok(status)
    }

    /// 等后台构建收敛；测试与预热用，请求路径不等。
    pub fn wait_until_idle(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while self.building() {
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        true
    }

    fn stored_revisions(&self) -> DomainResult<HashMap<SessionKey, String>> {
        Ok(self
            .with_db(|connection| {
                let mut statement =
                    connection.prepare("SELECT tool, ref, revision FROM indexed_sessions")?;
                let rows = statement.query_map([], |row| {
                    Ok((
                        (row.get::<_, String>(0)?, row.get::<_, String>(1)?),
                        row.get::<_, String>(2)?,
                    ))
                })?;
                let mut stored = HashMap::new();
                for row in rows {
                    let (key, revision) = row?;
                    stored.insert(key, revision);
                }
                Ok(stored)
            })?
            .unwrap_or_default())
    }

    fn drop_sessions(&self, keys: &[SessionKey]) -> DomainResult<()> {
        self.with_db(|connection| {
            write_transaction(connection, |connection| {
                for (tool, reference) in keys {
                    connection.execute(
                        "DELETE FROM records WHERE tool = ? AND ref = ?",
                        rusqlite::params![tool, reference],
                    )?;
                    connection.execute(
                        "DELETE FROM indexed_sessions WHERE tool = ? AND ref = ?",
                        rusqlite::params![tool, reference],
                    )?;
                }
                Ok(())
            })
        })?;
        Ok(())
    }

    fn index_session(
        &self,
        index: &Arc<AgentSessionIndex>,
        record: &IndexedSession,
    ) -> DomainResult<()> {
        // 解析在锁外做，大会话的读取不能阻塞并发查询。索引是批量后台读，
        // 不钉内容：读到比 revision 更新的内容无害，下轮 revision 变化会重新入队。
        let (rows, failed) = match read_indexed_session(index, record, false) {
            Ok(session) => (session_rows(&session), 0i64),
            // 读取失败按当前 revision 记账，不空转重试。
            Err(_) => (Vec::new(), 1i64),
        };
        let clipped_rows: i64 = rows.iter().map(|row| row.clipped).sum();
        let indexed_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs() as i64)
            .unwrap_or_default();
        self.with_db(|connection| {
            write_transaction(connection, |connection| {
                connection.execute(
                    "DELETE FROM records WHERE tool = ? AND ref = ?",
                    rusqlite::params![record.tool, record.canonical_ref],
                )?;
                {
                    let mut statement = connection.prepare(
                        "INSERT INTO records(tool, ref, message, turn, role, text, tool_text, clipped)
                         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                    )?;
                    for row in &rows {
                        statement.execute(rusqlite::params![
                            record.tool,
                            record.canonical_ref,
                            row.message,
                            row.turn,
                            row.role,
                            row.text,
                            row.tool_text,
                            row.clipped,
                        ])?;
                    }
                }
                connection.execute(
                    "INSERT OR REPLACE INTO indexed_sessions(tool, ref, revision, record_rows,
                     clipped_rows, failed, indexed_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
                    rusqlite::params![
                        record.tool,
                        record.canonical_ref,
                        record.revision,
                        rows.len() as i64,
                        clipped_rows,
                        failed,
                        indexed_at,
                    ],
                )?;
                Ok(())
            })
        })?;
        Ok(())
    }

    fn enqueue(self: &Arc<Self>, index: &Arc<AgentSessionIndex>, records: &[IndexedSession]) {
        let mut worker = self
            .worker
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if worker.closed {
            return;
        }
        for record in records {
            let key = (record.tool.clone(), record.canonical_ref.clone());
            match worker.queued.iter_mut().find(|(seen, _)| *seen == key) {
                Some(slot) => slot.1 = record.clone(),
                None => worker.queued.push((key, record.clone())),
            }
        }
        if worker.running {
            return;
        }
        worker.running = true;
        drop(worker);
        let this = self.clone();
        let index = index.clone();
        let spawned = std::thread::Builder::new()
            .name("content-index".into())
            .spawn(move || this.drain(&index));
        if spawned.is_err() {
            self.worker
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .running = false;
        }
    }

    fn drain(&self, index: &Arc<AgentSessionIndex>) {
        loop {
            let next = {
                let mut worker = self
                    .worker
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if worker.closed || worker.queued.is_empty() {
                    worker.running = false;
                    return;
                }
                worker.queued.pop().map(|(_, record)| record)
            };
            let Some(record) = next else {
                continue;
            };
            // 后台构建吞掉 sqlite 错误并记日志（Python `_drain` 的
            // `except sqlite3.Error`）：一条会话建不起来不能拖垮整条队列。
            if let Err(error) = self.index_session(index, &record) {
                crate::server::serve::log_warning(&format!(
                    "内容索引后台构建失败: {} {}",
                    record.canonical_ref,
                    error.message()
                ));
            }
        }
    }

    fn building(&self) -> bool {
        let worker = self
            .worker
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        !worker.queued.is_empty() || worker.running
    }

    // ---------- 查询 ----------

    /// 内容已入索引的会话集（排除读取失败的）；`None` 表示索引不可用。
    ///
    /// 预过滤用它区分「索引说没有」与「索引还不知道」。
    pub fn indexed_session_keys(&self) -> DomainResult<Option<Vec<SessionKey>>> {
        self.with_db(|connection| {
            let mut statement =
                connection.prepare("SELECT tool, ref FROM indexed_sessions WHERE failed = 0")?;
            let rows = statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    /// 每个会话被 16 KB 截断的消息数；只返回确有截断的会话。
    pub fn clipped_rows_by_session(&self) -> DomainResult<HashMap<SessionKey, i64>> {
        Ok(self
            .with_db(|connection| {
                let mut statement = connection.prepare(
                    "SELECT tool, ref, clipped_rows FROM indexed_sessions WHERE clipped_rows > 0",
                )?;
                let rows = statement.query_map([], |row| {
                    Ok((
                        (row.get::<_, String>(0)?, row.get::<_, String>(1)?),
                        row.get::<_, i64>(2)?,
                    ))
                })?;
                let mut hits = HashMap::new();
                for row in rows {
                    let (key, count) = row?;
                    hits.insert(key, count);
                }
                Ok(hits)
            })?
            .unwrap_or_default())
    }

    /// 正则预过滤：必然字面量**全部**命中（同一会话内，可跨消息）的会话集。
    ///
    /// 返回 `None` 表示索引不可用，调用方应退化为全量扫描。
    pub fn sessions_matching_literals(
        &self,
        literals: &[String],
        include_tool_outputs: bool,
    ) -> DomainResult<Option<Vec<SessionKey>>> {
        let columns = columns_for(include_tool_outputs);
        self.with_db(|connection| {
            let mut candidates: Option<Vec<SessionKey>> = None;
            for literal in literals {
                let mut statement = connection.prepare(
                    "SELECT DISTINCT r.tool, r.ref FROM records_fts
                     JOIN records r ON r.id = records_fts.rowid
                     WHERE records_fts MATCH ?",
                )?;
                let query = format!("{columns}: {}", fts_phrase(literal));
                let rows = statement.query_map([query], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?;
                let matched: Vec<SessionKey> = rows.collect::<rusqlite::Result<_>>()?;
                candidates = Some(match candidates {
                    None => matched,
                    Some(previous) => previous
                        .into_iter()
                        .filter(|key| matched.contains(key))
                        .collect(),
                });
                if candidates.as_ref().is_some_and(Vec::is_empty) {
                    break;
                }
            }
            Ok(candidates.unwrap_or_default())
        })
    }

    fn short_term_filter(
        short_terms: &[&str],
        include_tool_outputs: bool,
        alias: &str,
    ) -> (String, Vec<String>) {
        let mut clauses = String::new();
        let mut params = Vec::new();
        for term in short_terms {
            let pattern = like_pattern(term);
            if include_tool_outputs {
                clauses.push_str(&format!(
                    " AND ({alias}text LIKE ? ESCAPE '\\' OR {alias}tool_text LIKE ? ESCAPE '\\')"
                ));
                params.push(pattern.clone());
                params.push(pattern);
            } else {
                clauses.push_str(&format!(" AND {alias}text LIKE ? ESCAPE '\\'"));
                params.push(pattern);
            }
        }
        (clauses, params)
    }

    fn match_one(
        &self,
        needle: &str,
        include_tool_outputs: bool,
    ) -> DomainResult<(Vec<MatchRow>, &'static str)> {
        let terms = parse_query_terms(needle);
        let long_terms: Vec<&str> = terms
            .iter()
            .filter(|term| term.chars().count() >= MIN_TRIGRAM_CHARS)
            .map(String::as_str)
            .collect();
        let short_terms: Vec<&str> = terms
            .iter()
            .filter(|term| term.chars().count() < MIN_TRIGRAM_CHARS)
            .map(String::as_str)
            .collect();
        if long_terms.is_empty() {
            return Ok((
                self.match_substring(&short_terms, include_tool_outputs)?,
                "substring_scan",
            ));
        }
        Ok((
            self.match_trigram(&long_terms, &short_terms, include_tool_outputs)?,
            "trigram",
        ))
    }

    fn match_trigram(
        &self,
        long_terms: &[&str],
        short_terms: &[&str],
        include_tool_outputs: bool,
    ) -> DomainResult<Vec<MatchRow>> {
        let columns = columns_for(include_tool_outputs);
        // 多个短语在 FTS5 里默认 AND：同一条消息内全部命中才算数。
        let phrases: Vec<String> = long_terms.iter().map(|term| fts_phrase(term)).collect();
        let query = format!("{columns}: {}", phrases.join(" "));
        let (extra, extra_params) =
            Self::short_term_filter(short_terms, include_tool_outputs, "r.");
        self.with_db(|connection| {
            let sql = format!(
                "SELECT r.id, r.tool, r.ref, r.message, r.turn, r.role, bm25(records_fts) AS rank
                 FROM records_fts JOIN records r ON r.id = records_fts.rowid
                 WHERE records_fts MATCH ?{extra} ORDER BY rank LIMIT ?"
            );
            let mut statement = connection.prepare(&sql)?;
            let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(query.clone())];
            for value in &extra_params {
                params.push(Box::new(value.clone()));
            }
            params.push(Box::new(MAX_MATCH_ROWS as i64 + 1));
            let rows = statement.query_map(rusqlite::params_from_iter(params.iter()), read_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
        .map(Option::unwrap_or_default)
    }

    fn match_substring(
        &self,
        terms: &[&str],
        include_tool_outputs: bool,
    ) -> DomainResult<Vec<MatchRow>> {
        let (extra, params) = Self::short_term_filter(terms, include_tool_outputs, "");
        self.with_db(|connection| {
            let sql = format!(
                "SELECT id, tool, ref, message, turn, role, NULL FROM records
                 WHERE 1=1{extra} ORDER BY id LIMIT ?"
            );
            let mut statement = connection.prepare(&sql)?;
            let mut bound: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
            for value in &params {
                bound.push(Box::new(value.clone()));
            }
            bound.push(Box::new(MAX_MATCH_ROWS as i64 + 1));
            let rows = statement.query_map(rusqlite::params_from_iter(bound.iter()), read_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
        .map(Option::unwrap_or_default)
    }

    /// 返回 (按 `(tool, canonical_ref)` 聚合的命中, 查询元信息)。
    ///
    /// `needles` 是 OR 关系的多个 pattern：任一 pattern 命中即算命中（单个
    /// pattern 内部仍是词级 AND）。跨 pattern 去重按行 id，取最优 bm25 排名。
    pub fn search(
        &self,
        needles: &[String],
        include_tool_outputs: bool,
    ) -> DomainResult<(HashMap<SessionKey, ContentHit>, Map<String, Value>)> {
        if self.with_db(|_| Ok(()))?.is_none() {
            let mut meta = Map::new();
            meta.insert("match_mode".into(), Value::Null);
            meta.insert("rows_capped".into(), Value::Bool(false));
            return Ok((HashMap::new(), meta));
        }
        let mut merged: Vec<MatchRow> = Vec::new();
        let mut by_id: HashMap<i64, usize> = HashMap::new();
        let mut modes: Vec<&'static str> = Vec::new();
        for needle in needles {
            if needle.trim().is_empty() {
                continue;
            }
            let (rows, mode) = self.match_one(needle, include_tool_outputs)?;
            if !modes.contains(&mode) {
                modes.push(mode);
            }
            for row in rows {
                match by_id.get(&row.id) {
                    Some(position) => {
                        if rank_is_better(row.rank, merged[*position].rank) {
                            merged[*position] = row;
                        }
                    }
                    None => {
                        by_id.insert(row.id, merged.len());
                        merged.push(row);
                    }
                }
            }
        }
        // 稳定排序：rank 相同的行保持首次插入顺序（对齐 Python dict + sorted）。
        merged.sort_by(|left, right| {
            let key = |row: &MatchRow| row.rank.unwrap_or(f64::INFINITY);
            key(left)
                .partial_cmp(&key(right))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let capped = merged.len() > MAX_MATCH_ROWS;
        let mode = if modes.contains(&"trigram") {
            Some("trigram")
        } else {
            modes.first().copied()
        };
        let mut hits: HashMap<SessionKey, ContentHit> = HashMap::new();
        for row in merged.into_iter().take(MAX_MATCH_ROWS) {
            let key = (row.tool.clone(), row.reference.clone());
            let bucket = hits.entry(key).or_insert_with(|| ContentHit {
                count: 0,
                best_rank: row.rank,
                rows: Vec::new(),
            });
            bucket.count += 1;
            if rank_is_better(row.rank, bucket.best_rank) {
                bucket.best_rank = row.rank;
            }
            if bucket.rows.len() < MATCHES_PER_SESSION {
                let mut entry = Map::new();
                entry.insert("id".into(), Value::from(row.id));
                entry.insert("message".into(), Value::from(row.message));
                entry.insert("turn".into(), Value::from(row.turn));
                entry.insert("role".into(), Value::from(row.role.as_str()));
                bucket.rows.push(entry);
            }
        }
        let mut meta = Map::new();
        meta.insert(
            "match_mode".into(),
            mode.map(Value::from).unwrap_or(Value::Null),
        );
        meta.insert("rows_capped".into(), Value::Bool(capped));
        Ok((hits, meta))
    }

    /// 定位命中上下文；返回原文窗口，脱敏由 DTO 边界负责。
    pub fn snippet(
        &self,
        row_id: i64,
        needles: &[String],
        include_tool_outputs: bool,
    ) -> DomainResult<String> {
        // Python 的 fetchone() 对缺行返回 None → 空串；query_row 需 optional()
        // 才不会把 QueryReturnedNoRows 当查询失败上抛。
        let row = self
            .with_db(|connection| {
                connection
                    .query_row(
                        "SELECT text, tool_text FROM records WHERE id = ?",
                        [row_id],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                    )
                    .optional()
            })?
            .flatten();
        let Some((text, tool_text)) = row else {
            return Ok(String::new());
        };
        let sources: Vec<&str> = if include_tool_outputs {
            vec![&text, &tool_text]
        } else {
            vec![&text]
        };
        let folded_terms: Vec<String> = needles
            .iter()
            .flat_map(|needle| parse_query_terms(needle))
            .map(|term| super::usage::casefold(&term))
            .collect();
        for source in &sources {
            let folded = super::usage::casefold(source);
            for term in &folded_terms {
                let Some(position) = char_find(&folded, term) else {
                    continue;
                };
                let total = source.chars().count();
                let start = position.saturating_sub(SNIPPET_BEFORE);
                let end = total.min(position + term.chars().count() + SNIPPET_AFTER);
                return Ok(format!(
                    "{}{}{}",
                    if start > 0 { "…" } else { "" },
                    char_slice(source, start, end),
                    if end < total { "…" } else { "" }
                ));
            }
        }
        for source in &sources {
            if source.is_empty() {
                continue;
            }
            let clipped = source.chars().count() > SNIPPET_AFTER;
            return Ok(format!(
                "{}{}",
                char_slice(source, 0, SNIPPET_AFTER),
                if clipped { "…" } else { "" }
            ));
        }
        Ok(String::new())
    }
}

fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MatchRow> {
    Ok(MatchRow {
        id: row.get(0)?,
        tool: row.get(1)?,
        reference: row.get(2)?,
        message: row.get(3)?,
        turn: row.get(4)?,
        role: row.get(5)?,
        rank: row.get(6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Block, Message};

    fn index_at(temp: &tempfile::TempDir) -> Arc<ContentIndex> {
        Arc::new(ContentIndex::new(Some(
            temp.path().join("content-index.sqlite3"),
        )))
    }

    /// 事务出错必须 ROLLBACK：连接是共享的，残留的未闭合事务会毒死后续每一次
    /// 写入（下一个 `BEGIN` 直接报错，写入落进泄漏的事务里）。
    #[test]
    fn a_failed_write_transaction_rolls_back_instead_of_leaking() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY)")
            .unwrap();

        let failed = write_transaction(&connection, |connection| {
            connection.execute("INSERT INTO t(id) VALUES (1)", [])?;
            // 中途失败：整笔都不该落库。
            connection.execute("INSERT INTO nonexistent(id) VALUES (2)", [])?;
            Ok(())
        });
        assert!(failed.is_err());

        // 事务已回滚：既没有残留数据，也没有残留的打开事务。
        let count: i64 = connection
            .query_row("SELECT count(*) FROM t", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
        write_transaction(&connection, |connection| {
            connection.execute("INSERT INTO t(id) VALUES (3)", [])?;
            Ok(())
        })
        .expect("回滚后仍能开新事务");
        let ids: Vec<i64> = connection
            .prepare("SELECT id FROM t")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(ids, vec![3]);
    }

    #[test]
    fn fts5_trigram_is_available_in_the_bundled_sqlite() {
        let temp = tempfile::tempdir().unwrap();
        let content = index_at(&temp);
        // 建库即建 FTS5 虚表；不可用会走 unavailable 分支。
        assert!(content.with_db(|_| Ok(())).unwrap().is_some());
        assert_eq!(content.unavailable_reason(), None);
        let version = content
            .with_db(|connection| {
                connection.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            })
            .unwrap()
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn schema_reset_drops_tables_on_version_mismatch() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("content-index.sqlite3");
        {
            let connection = Connection::open(&path).unwrap();
            connection.execute_batch("PRAGMA user_version = 1").unwrap();
            connection
                .execute_batch("CREATE TABLE records(id INTEGER PRIMARY KEY, junk TEXT)")
                .unwrap();
            connection
                .execute("INSERT INTO records(junk) VALUES ('x')", [])
                .unwrap();
        }
        let content = Arc::new(ContentIndex::new(Some(path)));
        let count = content
            .with_db(|connection| {
                connection.query_row("SELECT COUNT(*) FROM records", [], |row| {
                    row.get::<_, i64>(0)
                })
            })
            .unwrap()
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn parse_query_terms_splits_words_and_honours_quotes() {
        assert_eq!(parse_query_terms("a b"), vec!["a", "b"]);
        assert_eq!(
            parse_query_terms("\"exact phrase\" tail"),
            vec!["exact phrase", "tail"]
        );
        // 裸布尔操作符按噪声丢弃。
        assert_eq!(parse_query_terms("a OR b"), vec!["a", "b"]);
        assert_eq!(parse_query_terms("OR"), vec!["OR".to_string()]);
        assert_eq!(parse_query_terms("  "), vec![String::new()]);
    }

    #[test]
    fn extract_clips_each_column_at_16k() {
        let mut message = Message::new("user");
        message
            .blocks
            .push(Block::text("x".repeat(RECORD_TEXT_CAP + 10)));
        let (text, tool_text, clipped) = extract(&message);
        assert_eq!(text.chars().count(), RECORD_TEXT_CAP);
        assert!(tool_text.is_empty());
        assert!(clipped);

        let mut short = Message::new("user");
        short.blocks.push(Block::text("hi"));
        assert_eq!(extract(&short), ("hi".into(), String::new(), false));
    }

    #[test]
    fn session_rows_skip_empty_messages_and_track_turns() {
        let mut session = Session::new("claude", "s", "/tmp");
        let mut empty = Message::new("assistant");
        empty.blocks.push(Block::text(""));
        let mut first = Message::new("user");
        first.blocks.push(Block::text("q1"));
        let mut reply = Message::new("assistant");
        reply.blocks.push(Block::text("a1"));
        session.messages = vec![first, empty, reply];
        let rows = session_rows(&session);
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].message, rows[0].turn), (1, 1));
        // 空消息不入索引，但编号仍按原始下标。
        assert_eq!((rows[1].message, rows[1].turn), (3, 1));
    }

    #[test]
    fn like_patterns_escape_wildcards() {
        assert_eq!(like_pattern("a_b%c"), "%a\\_b\\%c%");
        assert_eq!(like_pattern("a\\b"), "%a\\\\b%");
    }

    #[test]
    fn rank_comparison_puts_none_last() {
        assert!(rank_is_better(Some(-2.0), Some(-1.0)));
        assert!(!rank_is_better(Some(-1.0), Some(-2.0)));
        assert!(rank_is_better(Some(-1.0), None));
        assert!(!rank_is_better(None, Some(-1.0)));
        assert!(!rank_is_better(None, None));
    }

    /// 端到端：写入两个会话的记录，trigram 查询按 AND 命中，短词走 LIKE。
    #[test]
    fn trigram_and_substring_paths_return_expected_rows() {
        let temp = tempfile::tempdir().unwrap();
        let content = index_at(&temp);
        content
            .with_db(|connection| {
                connection.execute_batch(
                    "INSERT INTO records(tool, ref, message, turn, role, text, tool_text, clipped)
                 VALUES ('claude','/a',1,1,'user','ferry content index','',0),
                        ('claude','/a',2,1,'assistant','unrelated text','tool dump',0),
                        ('codex','/b',1,1,'user','ferry only','',0);",
                )?;
                Ok(())
            })
            .unwrap();

        let (hits, meta) = content
            .search(&["content index".to_string()], false)
            .unwrap();
        assert_eq!(meta["match_mode"], Value::from("trigram"));
        assert_eq!(meta["rows_capped"], Value::Bool(false));
        assert_eq!(hits.len(), 1);
        let hit = hits.get(&("claude".into(), "/a".into())).unwrap();
        assert_eq!(hit.count, 1);
        assert_eq!(hit.rows[0]["message"], Value::from(1));

        // OR：两个 pattern 命中并集。
        let (hits, _) = content
            .search(
                &["content index".to_string(), "ferry only".to_string()],
                false,
            )
            .unwrap();
        assert_eq!(hits.len(), 2);

        // 短词（<3 字符）退化为 LIKE 子串扫描。
        let (hits, meta) = content.search(&["on".to_string()], false).unwrap();
        assert_eq!(meta["match_mode"], Value::from("substring_scan"));
        assert!(!hits.is_empty());

        // include_tool_outputs 才能命中 tool_text 列。
        let (hits, _) = content.search(&["tool dump".to_string()], false).unwrap();
        assert!(hits.is_empty());
        let (hits, _) = content.search(&["tool dump".to_string()], true).unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn literal_prefilter_intersects_across_messages() {
        let temp = tempfile::tempdir().unwrap();
        let content = index_at(&temp);
        content
            .with_db(|connection| {
                connection.execute_batch(
                    "INSERT INTO records(tool, ref, message, turn, role, text, tool_text, clipped)
                 VALUES ('claude','/a',1,1,'user','alpha marker','',0),
                        ('claude','/a',2,1,'user','beta marker','',0),
                        ('codex','/b',1,1,'user','alpha only','',0);",
                )?;
                Ok(())
            })
            .unwrap();
        // 两个字面量都要命中（可跨消息）→ 只剩 /a。
        let matched = content
            .sessions_matching_literals(&["alpha".into(), "beta".into()], false)
            .unwrap()
            .unwrap();
        assert_eq!(matched, vec![("claude".to_string(), "/a".to_string())]);
        // 任一字面量无命中 → 空集。
        let empty = content
            .sessions_matching_literals(&["alpha".into(), "zzzz".into()], false)
            .unwrap()
            .unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn snippet_windows_around_the_first_matching_term() {
        let temp = tempfile::tempdir().unwrap();
        let content = index_at(&temp);
        let body = format!("{}NEEDLE{}", "a".repeat(300), "b".repeat(300));
        content
            .with_db(|connection| {
                connection.execute(
                "INSERT INTO records(id, tool, ref, message, turn, role, text, tool_text, clipped)
                 VALUES (1,'claude','/a',1,1,'user',?,'',0)",
                [&body],
            )?;
                Ok(())
            })
            .unwrap();
        let text = content.snippet(1, &["needle".to_string()], false).unwrap();
        assert!(text.starts_with('…') && text.ends_with('…'));
        assert!(text.contains("NEEDLE"));
        // 无命中时回落到开头窗口。
        let fallback = content.snippet(1, &["zzz".to_string()], false).unwrap();
        assert!(fallback.starts_with('a'));
        assert_eq!(content.snippet(999, &["x".to_string()], false).unwrap(), "");
    }

    #[test]
    fn clipped_rows_and_indexed_keys_are_reported() {
        let temp = tempfile::tempdir().unwrap();
        let content = index_at(&temp);
        content
            .with_db(|connection| {
                connection.execute_batch(
                    "INSERT INTO indexed_sessions(tool, ref, revision, record_rows, clipped_rows,
                 failed, indexed_at) VALUES ('claude','/a','r1',3,2,0,0),
                 ('codex','/b','r2',1,0,1,0);",
                )?;
                Ok(())
            })
            .unwrap();
        let keys = content.indexed_session_keys().unwrap().unwrap();
        // failed=1 的会话不算「已入索引」。
        assert_eq!(keys, vec![("claude".to_string(), "/a".to_string())]);
        let clipped = content.clipped_rows_by_session().unwrap();
        assert_eq!(clipped.len(), 1);
        assert_eq!(clipped[&("claude".into(), "/a".into())], 2);
    }
}
