//! 跨会话正文的持久化全文索引。
//!
//! SQLite FTS5 + trigram 分词：中英文与代码都按任意子串命中。索引以 revision
//! 对账做增量——每次搜索只重建内容真正变过的会话，其余零 IO；首次全量构建在
//! 后台线程完成，期间搜索返回部分结果并如实上报覆盖度。
//!
//! `~/.ferry/content-index.sqlite3` 的表结构由 `user_version` 标识（当前是 2），
//! 改 schema 必须同时升它，否则旧库会被当成新库读。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use regex::Regex;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
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

/// 后台构建的读+解析并行度；写仍是单线程（sqlite 连接只有一条）。
fn build_workers() -> usize {
    6.min(
        std::thread::available_parallelism()
            .map(|value| value.get())
            .unwrap_or(4),
    )
}

static BUILD_POOL: LazyLock<rayon::ThreadPool> = LazyLock::new(|| {
    rayon::ThreadPoolBuilder::new()
        .num_threads(build_workers())
        .thread_name(|index| format!("content-index-read-{index}"))
        .build()
        .expect("内容索引读取线程池必须可创建")
});

/// 一轮处理多少个会话：先并行解析这么多，再把它们写进一个事务。
///
/// 读已经走独立只读连接（[`ContentIndex::with_read_db`]），写事务不再挡住检索，
/// 所以这个值现在只剩一个作用：解析期间 [`BUILD_POOL`] 满负荷跑多久——跑得越
/// 久，并发检索越抢不到 CPU。
///
/// 本机实测（100 s 窗口，边建边查；检索 p50/p95/最坏，窗口内建完的会话数）：
/// 48 → 1047/3516/6230 ms，1147 条；12 → 924/1690/2073 ms，1344 条；
/// 4 → 795/1114/1738 ms，1608 条。逐条提交的基线是 767/1099/1421 ms，1221 条。
/// 4 在尾延迟和吞吐上同时最好：延迟已与基线齐平，同窗口还多建 32% 的会话。
/// 往大调是两头都亏（解析抢核），别动。
const BUILD_BATCH_SESSIONS: usize = 4;
/// 一批的源文件字节上限：防止一批全是巨型会话把解析结果堆在内存里。
const BUILD_BATCH_BYTES: i64 = 64 * 1024 * 1024;

/// 后台构建耗时分解（纳秒）；只给 `examples/index_bench` 这类基准用。
static PARSE_NANOS: AtomicU64 = AtomicU64::new(0);
static WRITE_NANOS: AtomicU64 = AtomicU64::new(0);

/// 返回 `(读+解析累计纳秒, sqlite 写入累计纳秒)`。
pub fn build_timing() -> (u64, u64) {
    (
        PARSE_NANOS.load(Ordering::Relaxed),
        WRITE_NANOS.load(Ordering::Relaxed),
    )
}

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

/// 以 `\n` 拼接、到 [`RECORD_TEXT_CAP`] 个字符即停手的累加器。
///
/// 语义与「先全量 join、再 `char_slice(0, CAP)`、再比字符总数」完全一致，但不会
/// 为了取前 16 K 字符先把整段（可能是几 MB 的工具输出）拼出来再数一遍。
#[derive(Default)]
struct CappedText {
    text: String,
    chars: usize,
    clipped: bool,
    started: bool,
}

impl CappedText {
    /// `chars` 必须在**每一条**返回路径上都与 `text` 的字符数保持一致，否则下
    /// 一段又能重新写满一整个上限，累计长度突破 CAP。
    fn push(&mut self, piece: &str) {
        if self.started {
            if self.chars >= RECORD_TEXT_CAP {
                // 已满：后面还有内容，就一定发生了截断。
                self.clipped = true;
                return;
            }
            self.text.push('\n');
            self.chars += 1;
        } else {
            // 首段不加分隔符。
            self.started = true;
        }
        for character in piece.chars() {
            if self.chars == RECORD_TEXT_CAP {
                self.clipped = true;
                return;
            }
            self.text.push(character);
            self.chars += 1;
        }
    }
}

/// 一条消息拆成正文列与工具输出列；第三个返回值是「有内容被 16 KB 截断」。
fn extract(message: &Message) -> (String, String, bool) {
    let mut texts = CappedText::default();
    let mut tools = CappedText::default();
    for block in &message.blocks {
        if block.kind == BlockKind::Text && !block.text.is_empty() {
            texts.push(&block.text);
        } else if block.kind == BlockKind::Tool {
            let Some(call) = block.tool.as_ref() else {
                continue;
            };
            tools.push(&format!("[tool {}]", call.name));
            let output = tool_result_text(call.result.as_ref());
            if !output.is_empty() {
                tools.push(&output);
            }
        }
    }
    let clipped = texts.clipped || tools.clipped;
    (texts.text, tools.text, clipped)
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

/// 一个已解析、等待落库的会话。解析全程不碰数据库锁。
struct ParsedSession {
    record: IndexedSession,
    rows: Vec<RecordRow>,
    /// 读取失败：按当前 revision 记账，不空转重试。
    failed: i64,
}

/// 读会话正文并切成待写行；不持有任何数据库锁，可并行调用。
///
/// 索引是批量后台读，不钉内容：读到比 revision 更新的内容无害，下轮 revision
/// 变化会重新入队。
fn parse_session(index: &Arc<AgentSessionIndex>, record: &IndexedSession) -> ParsedSession {
    let started = Instant::now();
    let (rows, failed) = match read_indexed_session(index, record, false) {
        Ok(session) => (session_rows(&session), 0i64),
        Err(_) => (Vec::new(), 1i64),
    };
    PARSE_NANOS.fetch_add(started.elapsed().as_nanos() as u64, Ordering::Relaxed);
    ParsedSession {
        record: record.clone(),
        rows,
        failed,
    }
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

/// 空闲只读连接池。
///
/// WAL 本来就支持「一写多读」，之前所有检索和后台写共用同一条连接、同一把锁，
/// 于是建索引期间的检索要排在整个写事务后面。读走独立连接后两者互不阻塞。
#[derive(Default)]
struct ReaderPool {
    idle: Vec<Connection>,
    /// 已开出去的连接数（含借出未还的），用于封顶。
    opened: usize,
    closed: bool,
}

/// 只读连接上限。检索本身是 CPU 密集的，开太多只会互相抢核。
const MAX_READERS: usize = 4;

pub struct ContentIndex {
    path: PathBuf,
    database: Mutex<Database>,
    readers: Mutex<ReaderPool>,
    worker: Mutex<WorkerState>,
    /// `Database::unavailable` / `Database::closed` 的无锁影子。只读入口每次
    /// 都要看这两个标志；若走 `database` 互斥量，构建期该锁被批事务近乎连续
    /// 占用，一次检索的百余次只读调用会各排一次批级队。原子读免锁；写侧在
    /// 持锁置位的同一处同步 store，读到陈旧值也无害——回落路径仍会在锁下
    /// 复核（见 [`Self::with_db`]）。
    unavailable_flag: AtomicBool,
    closed_flag: AtomicBool,
    /// 写连接已成功打开并确认过 schema（建库/迁移在 [`Self::open`] 里）。
    /// 只读路径不跑迁移，必须先由写连接把 schema 摆正，否则升级后的第一批
    /// 只读查询会撞上旧 `user_version` 的表结构而报错。
    schema_ready: AtomicBool,
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
            readers: Mutex::new(ReaderPool::default()),
            worker: Mutex::new(WorkerState::default()),
            unavailable_flag: AtomicBool::new(false),
            closed_flag: AtomicBool::new(false),
            schema_ready: AtomicBool::new(false),
        }
    }

    /// 只读连接：`mode=ro` 不会建库，也永远拿不到写锁。
    fn open_reader(path: &PathBuf) -> rusqlite::Result<Connection> {
        // 路径可能含 `?` `#` 之类的 URI 元字符，交给 rusqlite 的路径式 URI 打开。
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.busy_timeout(Duration::from_secs(5))?;
        // 读侧自己的页缓存；与写连接的缓存互不共享。
        connection.pragma_update(None, "cache_size", -32_768i64)?;
        Ok(connection)
    }

    /// 借一条只读连接；池空且未到上限就现开一条。
    ///
    /// 返回 `None` 表示「这条路不通」（库还没建出来 / 已关闭 / 开不出只读连接），
    /// 调用方回落到写连接，语义与改造前一致。
    fn checkout_reader(&self) -> Option<Connection> {
        {
            let mut pool = self
                .readers
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if pool.closed {
                return None;
            }
            if let Some(connection) = pool.idle.pop() {
                return Some(connection);
            }
            if pool.opened >= MAX_READERS {
                return None;
            }
            // 先占名额再开连接，避免并发下开超。
            pool.opened += 1;
        }
        match Self::open_reader(&self.path) {
            Ok(connection) => Some(connection),
            Err(_) => {
                let mut pool = self
                    .readers
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                pool.opened -= 1;
                None
            }
        }
    }

    fn return_reader(&self, connection: Connection) {
        let mut pool = self
            .readers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if pool.closed {
            pool.opened -= 1;
            return;
        }
        pool.idle.push(connection);
    }

    /// 纯读查询走独立只读连接，不与后台写抢同一把锁。
    ///
    /// 失败语义与 [`Self::with_db`] 完全一致：`Ok(None)` = 索引不可用，
    /// `Err` = 查询本身失败。拿不到只读连接时回落到写连接（首次建库、
    /// 单测里库还没落盘等情况都走这条）。
    fn with_read_db<T>(
        &self,
        action: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> DomainResult<Option<T>> {
        // 写侧已判定不可用/已关闭时，读侧也必须如实说「没有索引」，
        // 否则会拿着一条陈旧只读连接假装 ready。无锁读影子标志，
        // 不与构建期的批事务抢 database 互斥量。
        if self.unavailable_flag.load(Ordering::Acquire) || self.closed_flag.load(Ordering::Acquire)
        {
            return Ok(None);
        }
        // schema 只有写连接会建/迁移。写连接还没成功打开过时（例如 daemon
        // 刚启动、schema 升级后第一批只读查询先到），先借道 with_db 把库摆
        // 正，否则只读连接会在旧 `user_version` 的表结构上直接报错。
        if !self.schema_ready.load(Ordering::Acquire) {
            return self.with_db(action);
        }
        let Some(connection) = self.checkout_reader() else {
            return self.with_db(action);
        };
        // RAII 归还：action panic 时也不能泄漏 opened 名额，否则 MAX_READERS
        // 次 panic 后 checkout 永远失败，所有读永久静默回落写连接。
        struct Lease<'a> {
            index: &'a ContentIndex,
            connection: Option<Connection>,
        }
        impl Drop for Lease<'_> {
            fn drop(&mut self) {
                let Some(connection) = self.connection.take() else {
                    return;
                };
                if std::thread::panicking() {
                    // 展开途中连接状态未知，保守丢弃，只归还名额。
                    let mut pool = self
                        .index
                        .readers
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    pool.opened -= 1;
                } else {
                    self.index.return_reader(connection);
                }
            }
        }
        let lease = Lease {
            index: self,
            connection: Some(connection),
        };
        let outcome = action(lease.connection.as_ref().expect("上一行刚装载"));
        drop(lease);
        outcome
            .map(Some)
            .map_err(|error| DomainError::internal(format!("内容索引查询失败: {error}")))
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
                    self.unavailable_flag.store(true, Ordering::Release);
                    return Ok(None);
                }
            }
        }
        let connection = guard.connection.as_ref().expect("上一步已装载");
        // 连接在手即 schema 已确认（open() 里跑过 ensure_schema）。
        self.schema_ready.store(true, Ordering::Release);
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
        // 这个库是**派生**索引：断电丢掉最后几个事务只是让那几个会话下轮重建，
        // 不会有正确性损失，所以不值得为它每次提交都 fsync。
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        // trigram FTS 的插入是随机写 b-tree，默认 2 MB 页缓存会把整批插入打成
        // 随机 IO；给 64 MB 让一批的热页留在内存里。
        connection.pragma_update(None, "cache_size", -65_536i64)?;
        // 默认 1000 页（≈4 MB）就 checkpoint 一次，全量构建期间会反复把 WAL
        // 灌回主库；放宽到 ≈64 MB 显著减少这类回写。
        connection.pragma_update(None, "wal_autocheckpoint", 16_384i64)?;
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
        {
            // 先断读侧：借出去的连接归还时会被直接丢弃。
            let mut pool = self
                .readers
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            pool.closed = true;
            pool.opened -= pool.idle.len();
            pool.idle.clear();
        }
        let mut guard = self
            .database
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.closed = true;
        self.closed_flag.store(true, Ordering::Release);
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
        // 用集合而不是 Vec 求差集：会话数上千时 `Vec::contains` 是 O(n²)。
        let mut current: std::collections::HashSet<SessionKey> =
            std::collections::HashSet::with_capacity(records.len());
        let mut pending: Vec<IndexedSession> = Vec::new();
        for record in records {
            let key = (record.tool.clone(), record.canonical_ref.clone());
            if stored.get(&key).map(String::as_str) != Some(record.revision.as_str()) {
                pending.push(record.clone());
            }
            current.insert(key);
        }
        let stale: Vec<SessionKey> = stored
            .keys()
            .filter(|key| !current.contains(*key))
            .cloned()
            .collect();
        if !stale.is_empty() {
            self.drop_sessions(&stale)?;
        }
        let mut unresolved = pending.len();
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
                // 前台补完：写失败不再让整个搜索 RPC 挂掉——降级逐条写入，
                // 成功的部分立即可搜，失败的留在 pending 计数里下轮重试。
                let parsed: Vec<ParsedSession> = if pending.len() == 1 {
                    vec![parse_session(index, &pending[0])]
                } else {
                    BUILD_POOL.install(|| {
                        use rayon::prelude::*;
                        pending
                            .par_iter()
                            .map(|record| parse_session(index, record))
                            .collect()
                    })
                };
                unresolved = self.write_batch_degrading(&parsed).len();
            } else {
                self.enqueue(index, &pending);
            }
        }
        let mut status = Map::new();
        status.insert(
            "ready".into(),
            Value::Bool(unresolved == 0 && !self.building()),
        );
        status.insert(
            "indexed_sessions".into(),
            Value::from(records.len() - unresolved),
        );
        status.insert("pending_sessions".into(), Value::from(unresolved));
        Ok(status)
    }

    /// 只读覆盖度：与 [`Self::sync`] 同口径统计，但不写库、不入队、不清陈旧。
    ///
    /// `daemon.status` 与 `ferry scan --wait` 靠它判断内容索引是否已就绪；
    /// 状态查询必须无副作用，所以不能借道 `sync`。
    pub fn coverage(&self, records: &[IndexedSession]) -> DomainResult<Map<String, Value>> {
        let mut status = Map::new();
        if self.with_read_db(|_| Ok(()))?.is_none() {
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
            .with_read_db(|connection| {
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

    /// 单个会话的「删旧 + 插新 + 记账」；必须在事务里调用。
    fn persist_session(
        connection: &Connection,
        parsed: &ParsedSession,
        indexed_at: i64,
    ) -> rusqlite::Result<()> {
        let record = &parsed.record;
        connection.execute(
            "DELETE FROM records WHERE tool = ? AND ref = ?",
            rusqlite::params![record.tool, record.canonical_ref],
        )?;
        {
            // prepare_cached：整批（乃至整次构建）复用同一份编译好的
            // 语句，避免每会话重新编译插入语句。
            let mut statement = connection.prepare_cached(
                "INSERT INTO records(tool, ref, message, turn, role, text, tool_text, clipped)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )?;
            for row in &parsed.rows {
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
        let clipped_rows: i64 = parsed.rows.iter().map(|row| row.clipped).sum();
        connection.execute(
            "INSERT OR REPLACE INTO indexed_sessions(tool, ref, revision, record_rows,
             clipped_rows, failed, indexed_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                record.tool,
                record.canonical_ref,
                record.revision,
                parsed.rows.len() as i64,
                clipped_rows,
                parsed.failed,
                indexed_at,
            ],
        )?;
        Ok(())
    }

    /// 把一批已解析的会话写进一个事务。
    ///
    /// 批内每个会话的「删旧 + 插新 + 记账」在同一事务里，批中途失败整批回滚，
    /// 落回未索引状态由下一轮 revision 对账重来，不会留下半个会话。
    fn write_batch(&self, batch: &[ParsedSession]) -> DomainResult<()> {
        if batch.is_empty() {
            return Ok(());
        }
        let started = Instant::now();
        let indexed_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs() as i64)
            .unwrap_or_default();
        let outcome = self.with_db(|connection| {
            write_transaction(connection, |connection| {
                for parsed in batch {
                    Self::persist_session(connection, parsed, indexed_at)?;
                }
                Ok(())
            })
        });
        WRITE_NANOS.fetch_add(started.elapsed().as_nanos() as u64, Ordering::Relaxed);
        outcome?;
        Ok(())
    }

    /// 批量写入，失败自动降级：整批事务失败后逐条独立事务重试，只丢真正写
    /// 失败的那条。返回仍然失败的会话键（已按 ref 记日志）。
    ///
    /// 单事务整批写是快路径；但批事务把 4 条会话「连坐」了——毒会话（比如某条
    /// 记录触发 sqlite 错误）会让同批健康会话一起回滚，且 LIFO 分组稳定，下轮
    /// 还是同一批人，健康会话永远建不进去。降级逐条后：
    /// - 只有毒会话本身丢弃（revision 不落库，下轮对账重试）；
    /// - 读取失败（`failed=1`）的记账独立落库，保证「按 revision 记账不空转」。
    fn write_batch_degrading(&self, batch: &[ParsedSession]) -> Vec<SessionKey> {
        if batch.is_empty() {
            return Vec::new();
        }
        let batch_error = match self.write_batch(batch) {
            Ok(()) => return Vec::new(),
            Err(error) => error,
        };
        if batch.len() > 1 {
            crate::server::serve::log_warning(&format!(
                "内容索引批量写入失败, 降级为逐条重试: {} 条会话 {}",
                batch.len(),
                batch_error.message()
            ));
        }
        let mut failed = Vec::new();
        for parsed in batch {
            let outcome = if batch.len() == 1 {
                // 单条批不用重放：刚才那次失败就是它自己。
                Err(batch_error.clone())
            } else {
                self.write_batch(std::slice::from_ref(parsed))
            };
            if let Err(error) = outcome {
                crate::server::serve::log_warning(&format!(
                    "内容索引写入失败(跳过, 下轮 revision 对账重试): {}/{}: {}",
                    parsed.record.tool,
                    parsed.record.canonical_ref,
                    error.message()
                ));
                failed.push((
                    parsed.record.tool.clone(),
                    parsed.record.canonical_ref.clone(),
                ));
            }
        }
        failed
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

    /// 取下一批待建会话（LIFO，与单条出队时同序）。
    fn take_batch(&self) -> Option<Vec<IndexedSession>> {
        let mut worker = self
            .worker
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if worker.closed || worker.queued.is_empty() {
            worker.running = false;
            return None;
        }
        let mut batch = Vec::new();
        let mut bytes = 0i64;
        while batch.len() < BUILD_BATCH_SESSIONS && bytes < BUILD_BATCH_BYTES {
            let Some((_, record)) = worker.queued.pop() else {
                break;
            };
            bytes += record.row.get("size").and_then(Value::as_i64).unwrap_or(0);
            batch.push(record);
        }
        Some(batch)
    }

    fn drain(&self, index: &Arc<AgentSessionIndex>) {
        loop {
            let Some(batch) = self.take_batch() else {
                return;
            };
            if batch.is_empty() {
                continue;
            }
            // 读+解析是纯 CPU/IO 混合负载且互不依赖，先并行做完；写仍串行，
            // 一批一个事务。
            let parsed: Vec<ParsedSession> = if batch.len() == 1 {
                vec![parse_session(index, &batch[0])]
            } else {
                BUILD_POOL.install(|| {
                    use rayon::prelude::*;
                    batch
                        .par_iter()
                        .map(|record| parse_session(index, record))
                        .collect()
                })
            };
            // 后台构建吞掉 sqlite 错误并记日志（Python `_drain` 的
            // `except sqlite3.Error`）：写失败降级逐条重试，只丢真正失败的
            // 那条（已按 ref 记日志），不拖垮同批健康会话与整条队列。
            let _ = self.write_batch_degrading(&parsed);
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
    /// 预过滤用它区分「索引说没有」与「索引还不知道」。返回集合而不是 Vec：
    /// 调用方按候选逐条查成员，上万会话时线性 `contains` 是 O(n²)。
    pub fn indexed_session_keys(&self) -> DomainResult<Option<HashSet<SessionKey>>> {
        self.with_read_db(|connection| {
            let mut statement =
                connection.prepare("SELECT tool, ref FROM indexed_sessions WHERE failed = 0")?;
            let rows = statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            rows.collect::<rusqlite::Result<HashSet<_>>>()
        })
    }

    /// 每个会话被 16 KB 截断的消息数；只返回确有截断的会话。
    pub fn clipped_rows_by_session(&self) -> DomainResult<HashMap<SessionKey, i64>> {
        Ok(self
            .with_read_db(|connection| {
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
    ) -> DomainResult<Option<HashSet<SessionKey>>> {
        let columns = columns_for(include_tool_outputs);
        self.with_read_db(|connection| {
            let mut candidates: Option<HashSet<SessionKey>> = None;
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
                let matched: HashSet<SessionKey> = rows.collect::<rusqlite::Result<_>>()?;
                candidates = Some(match candidates {
                    None => matched,
                    Some(mut previous) => {
                        previous.retain(|key| matched.contains(key));
                        previous
                    }
                });
                if candidates.as_ref().is_some_and(HashSet::is_empty) {
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
        self.with_read_db(|connection| {
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
        self.with_read_db(|connection| {
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
        if self.with_read_db(|_| Ok(()))?.is_none() {
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
            .with_read_db(|connection| {
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

    fn parsed(reference: &str, text: &str, failed: i64) -> ParsedSession {
        use crate::adapters::contracts::StorageKind;
        ParsedSession {
            record: IndexedSession {
                opaque_ref: format!("fsr_{reference}"),
                tool: "claude".into(),
                canonical_ref: reference.into(),
                root: None,
                storage_kind: StorageKind::File,
                row: Map::new(),
                revision: "r1".into(),
                source_identity: Value::Null,
            },
            rows: if text.is_empty() {
                Vec::new()
            } else {
                vec![RecordRow {
                    message: 1,
                    turn: 1,
                    role: "user".into(),
                    text: text.into(),
                    tool_text: String::new(),
                    clipped: 0,
                }]
            },
            failed,
        }
    }

    /// 回归：批事务里的毒会话不能连坐同批健康会话。
    ///
    /// 整批事务失败后必须降级为逐条独立事务：健康会话与读取失败（failed=1）
    /// 的记账各自落库，只有真正写失败的那条丢弃等下轮重试，搜索不受影响。
    #[test]
    fn poisoned_batch_write_degrades_to_per_session_transactions() {
        let temp = tempfile::tempdir().unwrap();
        let content = index_at(&temp);
        // 用触发器构造一条稳定写失败的记录：text='POISON' 的插入必然报错。
        content
            .with_db(|connection| {
                connection.execute_batch(
                    "CREATE TRIGGER poison BEFORE INSERT ON records
                     WHEN new.text = 'POISON'
                     BEGIN SELECT RAISE(ABORT, 'poisoned row'); END;",
                )?;
                Ok(())
            })
            .unwrap()
            .unwrap();
        let batch = vec![
            parsed("/healthy-1", "hello world", 0),
            parsed("/poison", "POISON", 0),
            parsed("/read-failed", "", 1),
            parsed("/healthy-2", "goodbye", 0),
        ];
        let failed = content.write_batch_degrading(&batch);
        assert_eq!(failed, vec![("claude".to_string(), "/poison".to_string())]);

        let stored = content.stored_revisions().unwrap();
        assert!(stored.contains_key(&("claude".into(), "/healthy-1".into())));
        assert!(stored.contains_key(&("claude".into(), "/healthy-2".into())));
        // 读取失败的 failed 记账独立落库，不被同批他人失败回滚。
        assert!(stored.contains_key(&("claude".into(), "/read-failed".into())));
        // 毒会话不落 revision，下轮对账重试。
        assert!(!stored.contains_key(&("claude".into(), "/poison".into())));

        // 搜索不因毒会话失败，健康内容立即可搜。
        let (hits, _) = content.search(&["hello world".to_string()], false).unwrap();
        assert_eq!(hits.len(), 1);

        // 重跑同一批（模拟下一轮）：健康会话已入库幂等重写，毒会话仍只丢自己。
        let failed = content.write_batch_degrading(&batch);
        assert_eq!(failed.len(), 1);
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

    /// 回归：只读路径不跑迁移，旧 `user_version` 的库上第一批只读查询必须
    /// 先借道写连接把 schema 摆正，而不是在旧表结构上直接报错
    /// （表现为 daemon 启动窗口内 `daemon.status` 瞬时报 internal）。
    #[test]
    fn read_only_path_migrates_schema_before_first_query() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("content-index.sqlite3");
        {
            let connection = Connection::open(&path).unwrap();
            connection.execute_batch("PRAGMA user_version = 1").unwrap();
        }
        let content = Arc::new(ContentIndex::new(Some(path)));
        // 不先走任何写路径，直接只读查询。
        let stored = content.stored_revisions().unwrap();
        assert!(stored.is_empty());
        assert!(content.indexed_session_keys().unwrap().is_some());
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

        // 增量截断与「先全量 join 再切」等价：多段拼接含分隔符，恰好填满不算截断。
        let mut exact = Message::new("user");
        exact.blocks.push(Block::text("a".repeat(RECORD_TEXT_CAP)));
        let (text, _, clipped) = extract(&exact);
        assert_eq!(text.chars().count(), RECORD_TEXT_CAP);
        assert!(!clipped);

        let mut joined = Message::new("user");
        joined.blocks.push(Block::text("ab"));
        joined.blocks.push(Block::text("cd"));
        assert_eq!(extract(&joined).0, "ab\ncd");

        // 分隔符本身把长度顶过上限时也要记成截断。
        let mut spill = Message::new("user");
        spill
            .blocks
            .push(Block::text("a".repeat(RECORD_TEXT_CAP - 1)));
        spill.blocks.push(Block::text("b"));
        let (text, _, clipped) = extract(&spill);
        assert_eq!(text.chars().count(), RECORD_TEXT_CAP);
        assert!(clipped);

        // 多字节字符按字符计，不按字节。
        let mut wide = Message::new("user");
        wide.blocks.push(Block::text("中".repeat(RECORD_TEXT_CAP + 5)));
        let (text, _, clipped) = extract(&wide);
        assert_eq!(text.chars().count(), RECORD_TEXT_CAP);
        assert!(clipped);

        // 回归：某一段把上限撑满后，后续每一段都必须继续被挡住。早期实现只在
        // 循环正常结束时才回写计数，于是「截断后」的下一段又能写满一整个上限，
        // 整条记录膨胀到 CAP 的若干倍（实测让全库正文多出 25%）。
        let mut many = Message::new("user");
        for _ in 0..5 {
            many.blocks.push(Block::text("x".repeat(RECORD_TEXT_CAP)));
        }
        let (text, _, clipped) = extract(&many);
        assert_eq!(text.chars().count(), RECORD_TEXT_CAP);
        assert!(clipped);
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
        assert_eq!(
            matched,
            HashSet::from([("claude".to_string(), "/a".to_string())])
        );
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
        assert_eq!(
            keys,
            HashSet::from([("claude".to_string(), "/a".to_string())])
        );
        let clipped = content.clipped_rows_by_session().unwrap();
        assert_eq!(clipped.len(), 1);
        assert_eq!(clipped[&("claude".into(), "/a".into())], 2);
    }

    /// 回归：action panic 不能泄漏只读连接名额——早期实现没有 unwind 守卫，
    /// MAX_READERS 次 panic 后 opened 再也减不回去，所有读永久回落写连接。
    #[test]
    fn reader_slot_is_reclaimed_when_the_action_panics() {
        let temp = tempfile::tempdir().unwrap();
        let content = index_at(&temp);
        // 先建库，让只读连接开得出来。
        assert!(content.with_db(|_| Ok(())).unwrap().is_some());
        for _ in 0..(MAX_READERS + 1) {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = content.with_read_db(|_| -> rusqlite::Result<()> {
                    panic!("模拟查询回调 panic");
                });
            }));
            assert!(outcome.is_err());
        }
        // 名额全部收回：panic 路径丢弃连接并递减 opened。
        {
            let pool = content.readers.lock().unwrap();
            assert_eq!(pool.opened, 0);
            assert!(pool.idle.is_empty());
        }
        // 后续读仍走只读连接（会重新开出并归还到池）。
        assert!(content.indexed_session_keys().unwrap().is_some());
        let pool = content.readers.lock().unwrap();
        assert_eq!(pool.opened, 1);
        assert_eq!(pool.idle.len(), 1);
    }

    /// 读走独立只读连接后，必须仍然立刻看到写连接刚提交的内容——池化连接各有
    /// 自己的页缓存，只要不残留长事务，每条语句都读最新已提交状态。
    #[test]
    fn pooled_readers_observe_writes_committed_after_they_were_opened() {
        let temp = tempfile::tempdir().unwrap();
        let content = index_at(&temp);
        let insert = |reference: &str, text: &str| {
            content
                .with_db(|connection| {
                    write_transaction(connection, |connection| {
                        connection.execute(
                            "INSERT INTO records(tool, ref, message, turn, role, text, tool_text,
                             clipped) VALUES ('claude', ?, 1, 1, 'user', ?, '', 0)",
                            rusqlite::params![reference, text],
                        )?;
                        connection.execute(
                            "INSERT OR REPLACE INTO indexed_sessions(tool, ref, revision,
                             record_rows, clipped_rows, failed, indexed_at)
                             VALUES ('claude', ?, 'r1', 1, 0, 0, 0)",
                            rusqlite::params![reference],
                        )?;
                        Ok(())
                    })
                })
                .unwrap();
        };

        insert("/first", "alpha 共同前缀");
        // 先读一轮，把只读连接真正开出来并放回池子。
        assert_eq!(content.indexed_session_keys().unwrap().unwrap().len(), 1);
        assert_eq!(
            content
                .sessions_matching_literals(&["共同前缀".to_string()], false)
                .unwrap()
                .unwrap()
                .len(),
            1
        );

        // 同一批池化连接必须看见之后提交的新行。
        insert("/second", "beta 共同前缀");
        assert_eq!(content.indexed_session_keys().unwrap().unwrap().len(), 2);
        assert_eq!(
            content
                .sessions_matching_literals(&["共同前缀".to_string()], false)
                .unwrap()
                .unwrap()
                .len(),
            2
        );
        let (hits, _) = content.search(&["共同前缀".to_string()], false).unwrap();
        assert_eq!(hits.len(), 2);

        // 关闭后读侧同样如实报「没有索引」，不能拿着旧连接假装可用。
        content.close();
        assert!(content.indexed_session_keys().unwrap().is_none());
    }
}
