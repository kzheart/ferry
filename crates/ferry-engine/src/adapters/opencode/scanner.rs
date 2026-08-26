//! OpenCode SQLite 存储扫描与会话级指纹索引。
//!
//! 指纹必须是**会话级**的：所有 OpenCode 会话同住一个库，若把整库 stat 混进
//! 指纹，任何其它会话的写入都会让本会话的引用与迁移计划失效。会话级指纹要整库
//! 逐行哈希（上千会话约数秒），所以做三层缓存：
//! 1. 进程内 `FINGERPRINT_INDEX`；
//! 2. 跨进程落盘 `~/.ferry/opencode-fingerprints.json`（version=1）；
//! 3. 扫描收尾的后台线程重建。
//!
//! 扫描路径（`scan_fingerprint`）容忍落后一轮的快照，Agent 严格路径
//! （`fingerprint`）则按库戳记同步重建。

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use rusqlite::Connection;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::adapters::contracts::{ScanCache, ScanRow};
use crate::adapters::shared::scanner::{
    add_tokens, dominant_model, empty_tokens, has_tokens, session_roots, Tokens,
};
use crate::errors::DomainResult;
use crate::jsonutil::FileStat;
use crate::system::snapshots::data_dir;
use crate::system::sqlite;

use super::store;

/// 落盘快照的版本号；不匹配即整体丢弃。
const FINGERPRINT_STORE_VERSION: i64 = 1;
const STRICT_FINGERPRINT_CACHE_MAX_ENTRIES: usize = 256;

/// 库戳记：`[(路径, dev, ino, mtime_ns, size)]`，取不到 stat 时是 `[路径, null]`。
type Stamp = Vec<Value>;

#[derive(Clone, Debug, Default)]
struct FingerprintIndex {
    stamp: Stamp,
    /// `会话 id → 父 id`。
    sessions: BTreeMap<String, Option<String>>,
    /// `会话 id → 该会话全部行的 sha256`。
    revisions: BTreeMap<String, [u8; 32]>,
    /// `父 id → 子 id 列表`。
    children: BTreeMap<String, Vec<String>>,
}

#[derive(Clone)]
struct StrictFingerprintEntry {
    stamp: Stamp,
    value: Option<String>,
}

#[derive(Default)]
struct StrictFingerprintCache {
    entries: HashMap<String, StrictFingerprintEntry>,
    order: VecDeque<String>,
}

impl StrictFingerprintCache {
    fn get(&mut self, session_id: &str, stamp: &Stamp) -> Option<Option<String>> {
        let value = self
            .entries
            .get(session_id)
            .filter(|entry| &entry.stamp == stamp)
            .map(|entry| entry.value.clone())?;
        self.order.retain(|key| key != session_id);
        self.order.push_back(session_id.to_string());
        Some(value)
    }

    fn insert(&mut self, session_id: &str, entry: StrictFingerprintEntry) {
        self.entries.insert(session_id.to_string(), entry);
        self.order.retain(|key| key != session_id);
        self.order.push_back(session_id.to_string());
        while self.entries.len() > STRICT_FINGERPRINT_CACHE_MAX_ENTRIES {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            self.entries.remove(&oldest);
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }
}

struct IndexState {
    current: Option<Arc<FingerprintIndex>>,
    /// 后台重建线程是否在跑（对齐 `_REBUILD_THREAD.is_alive()`）。
    rebuilding: bool,
    strict: StrictFingerprintCache,
}

fn state() -> MutexGuard<'static, IndexState> {
    static STATE: OnceLock<Mutex<IndexState>> = OnceLock::new();
    STATE
        .get_or_init(|| {
            Mutex::new(IndexState {
                current: None,
                rebuilding: false,
                strict: StrictFingerprintCache::default(),
            })
        })
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 全量重建时持有的独占锁：重建要整库读，不加锁会同时重建 N 遍。
fn rebuild_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 清空进程内索引（单测隔离用）。
pub fn reset_fingerprint_index() {
    let mut state = state();
    state.current = None;
    state.rebuilding = false;
    state.strict.clear();
}

fn fingerprint_store_path() -> PathBuf {
    data_dir().join("opencode-fingerprints.json")
}

// ---------------------------------------------------------------------------
// 扫描
// ---------------------------------------------------------------------------

fn message_tokens(data: &Value) -> Tokens {
    let tokens = data.get("tokens").cloned().unwrap_or(Value::Null);
    let number = |value: Option<&Value>| value.and_then(Value::as_i64).unwrap_or(0);
    let cache = tokens.get("cache").cloned().unwrap_or(Value::Null);
    Tokens {
        input: number(tokens.get("input")),
        output: number(tokens.get("output")) + number(tokens.get("reasoning")),
        cache_read: number(cache.get("read")),
        cache_write: number(cache.get("write")),
    }
}

fn usage_by_model_value(by_model: &[(String, Tokens)]) -> Value {
    let mut result = serde_json::Map::new();
    for (model, tokens) in by_model {
        if !model.is_empty() && has_tokens(tokens) {
            result.insert(model.clone(), tokens.to_value());
        }
    }
    Value::Object(result)
}

/// 从 message 表按会话累加 token（session 表的 rollup 列覆盖不全）。
fn aggregate_usage(
    connection: &Connection,
) -> rusqlite::Result<BTreeMap<String, Vec<(String, Tokens)>>> {
    let mut by_session: BTreeMap<String, Vec<(String, Tokens)>> = BTreeMap::new();
    let mut statement = connection.prepare("SELECT session_id, data FROM message")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let session_id: String = match row.get(0) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let blob: String = match row.get(1) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let Ok(data) = serde_json::from_str::<Value>(&blob) else {
            continue;
        };
        if data.get("role") != Some(&Value::from("assistant")) {
            continue;
        }
        let model = data
            .get("modelID")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .or_else(|| data.get("model").and_then(Value::as_str))
            .unwrap_or("")
            .to_string();
        if model.is_empty() && data.get("tokens").is_none_or(Value::is_null) {
            continue;
        }
        let bucket = by_session.entry(session_id).or_default();
        let tokens = message_tokens(&data);
        match bucket.iter_mut().find(|(name, _)| *name == model) {
            Some((_, accumulator)) => add_tokens(accumulator, &tokens),
            None => {
                let mut accumulator = empty_tokens();
                add_tokens(&mut accumulator, &tokens);
                bucket.push((model, accumulator));
            }
        }
    }
    Ok(by_session)
}

/// 扫描全库；库不存在返回空清单（不是错误）。
///
/// `cache` 不参与：扫描是一次整库查询，没有可缓存的按文件粒度。
pub fn scan(_cache: &dyn ScanCache) -> DomainResult<Vec<ScanRow>> {
    let database = store::database_path();
    if !database.exists() {
        return Ok(Vec::new());
    }
    let Ok(connection) = open_readonly(&database) else {
        return Ok(Vec::new());
    };

    let mut counts: BTreeMap<String, i64> = BTreeMap::new();
    if let Ok(mut statement) =
        connection.prepare("SELECT session_id, COUNT(*) FROM message GROUP BY session_id")
    {
        if let Ok(rows) = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        }) {
            for entry in rows.flatten() {
                counts.insert(entry.0, entry.1);
            }
        }
    }
    let usage = aggregate_usage(&connection).unwrap_or_default();

    let mut rows: Vec<ScanRow> = Vec::new();
    let mut statement = match connection
        .prepare("SELECT id, title, directory, time_updated, time_created, parent_id FROM session")
    {
        Ok(statement) => statement,
        Err(_) => return Ok(Vec::new()),
    };
    let records = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<i64>>(3)?,
            row.get::<_, Option<i64>>(4)?,
            row.get::<_, Option<String>>(5)?,
        ))
    });
    let Ok(records) = records else {
        return Ok(Vec::new());
    };
    for record in records.flatten() {
        let (id, title, directory, updated, created, parent) = record;
        let by_model = usage.get(&id).cloned().unwrap_or_default();
        let mut tokens = empty_tokens();
        for (_, model_tokens) in &by_model {
            add_tokens(&mut tokens, model_tokens);
        }
        let mut row = ScanRow::new();
        row.insert("tool".into(), Value::from("opencode"));
        row.insert("id".into(), Value::from(id.as_str()));
        row.insert("title".into(), Value::from(title.unwrap_or_default()));
        row.insert("dir".into(), Value::from(directory.unwrap_or_default()));
        row.insert("updated".into(), Value::from(updated.unwrap_or(0)));
        row.insert("created".into(), created.map_or(Value::Null, Value::from));
        row.insert(
            "count".into(),
            Value::from(counts.get(&id).copied().unwrap_or(0)),
        );
        // opencode 会话不落文件：路径恒为空串、体积恒为 0。
        row.insert("size".into(), Value::from(0));
        row.insert("path".into(), Value::from(""));
        row.insert("parent_id".into(), parent.map_or(Value::Null, Value::from));
        row.insert(
            "tokens".into(),
            if has_tokens(&tokens) {
                tokens.to_value()
            } else {
                Value::Null
            },
        );
        row.insert("model".into(), Value::from(dominant_model(&by_model)));
        row.insert("usage_by_model".into(), usage_by_model_value(&by_model));
        rows.push(row);
    }

    Ok(session_roots(rows)?
        .into_iter()
        .filter(|root| root.get("count").and_then(Value::as_i64).unwrap_or(0) != 0)
        .collect())
}

fn open_readonly(database: &Path) -> rusqlite::Result<Connection> {
    sqlite::open_readonly(database)
}

// ---------------------------------------------------------------------------
// 库戳记
// ---------------------------------------------------------------------------

/// 只 stat `.db` 与 `-wal`。
///
/// **故意排除 `-shm`**：它只是 WAL 的共享内存索引，连只读连接都会更新它的
/// mtime。把它算进戳记会让指纹缓存被自己的读取动作反复失效，每次扫描都全量
/// 重读整库；数据变更必然体现在 `.db` 或 `-wal` 上，排除它不损失正确性。
pub fn database_stamp() -> Stamp {
    let database = store::database_path();
    let wal = {
        let mut name = database.as_os_str().to_os_string();
        name.push("-wal");
        PathBuf::from(name)
    };
    [database, wal]
        .into_iter()
        .map(|path| {
            let text = path.to_string_lossy().into_owned();
            match std::fs::metadata(&path) {
                Err(_) => Value::Array(vec![Value::from(text), Value::Null]),
                Ok(metadata) => {
                    let stat = FileStat::from_metadata(&metadata);
                    Value::Array(vec![
                        Value::from(text),
                        Value::from(stat.dev),
                        Value::from(stat.ino),
                        Value::from(stat.mtime_ns as i64),
                        Value::from(stat.size),
                    ])
                }
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 指纹索引重建
// ---------------------------------------------------------------------------

/// 一行的哈希输入：`len(payload).to_bytes(8,"big") + payload`，
/// payload 是 `[表名, *列值]` 的紧凑 JSON。长度前缀让不同分段不可能互相碰撞。
fn hash_row(digest: &mut Sha256, table: &str, row: &[Value]) {
    let mut payload = vec![Value::from(table)];
    payload.extend(row.iter().cloned());
    let text = crate::jsonutil::canonical_json(&Value::Array(payload)).unwrap_or_default();
    let bytes = text.as_bytes();
    digest.update((bytes.len() as u64).to_be_bytes());
    digest.update(bytes);
}

fn table_names(connection: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut statement = connection.prepare("SELECT name FROM sqlite_master WHERE type='table'")?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    rows.collect()
}

/// `(会话→父, 会话→行摘要, 父→子列表)`。
type IndexTriple = (
    BTreeMap<String, Option<String>>,
    BTreeMap<String, [u8; 32]>,
    BTreeMap<String, Vec<String>>,
);

/// 整库逐行哈希，得到 [`IndexTriple`]。
fn read_fingerprint_index() -> rusqlite::Result<IndexTriple> {
    let database = store::database_path();
    let resolved = std::fs::canonicalize(&database).unwrap_or(database);
    let connection = open_readonly(&resolved)?;
    connection.execute_batch("BEGIN")?;

    let tables = table_names(&connection)?;
    if !tables.iter().any(|name| name == "session") {
        return Ok((BTreeMap::new(), BTreeMap::new(), BTreeMap::new()));
    }

    let session_columns = store::table_columns(&connection, "session")?;
    let session_id_index = session_columns
        .iter()
        .position(|name| name == "id")
        .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
    let parent_index = session_columns
        .iter()
        .position(|name| name == "parent_id")
        .ok_or(rusqlite::Error::QueryReturnedNoRows)?;

    let mut sessions: BTreeMap<String, Option<String>> = BTreeMap::new();
    let mut digests: BTreeMap<String, Sha256> = BTreeMap::new();
    {
        let mut statement = connection.prepare("SELECT * FROM \"session\" ORDER BY \"id\"")?;
        let width = session_columns.len();
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let values: Vec<Value> = (0..width)
                .map(|index| {
                    store::cell_to_json(
                        row.get_ref(index)
                            .unwrap_or(rusqlite::types::ValueRef::Null),
                    )
                })
                .collect();
            let session_id =
                crate::adapters::shared::dialect::python_str(&values[session_id_index]);
            let parent = match &values[parent_index] {
                Value::Null => None,
                other => Some(crate::adapters::shared::dialect::python_str(other)),
            };
            sessions.insert(session_id.clone(), parent);
            let mut digest = Sha256::new();
            hash_row(&mut digest, "session", &values);
            digests.insert(session_id, digest);
        }
    }

    for table in ["message", "part"] {
        if !tables.iter().any(|name| name == table) {
            continue;
        }
        let columns = store::table_columns(&connection, table)?;
        let Some(session_index) = columns.iter().position(|name| name == "session_id") else {
            continue;
        };
        let mut statement = connection.prepare(&format!(
            "SELECT * FROM \"{table}\" ORDER BY \"session_id\", \"id\""
        ))?;
        let width = columns.len();
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let values: Vec<Value> = (0..width)
                .map(|index| {
                    store::cell_to_json(
                        row.get_ref(index)
                            .unwrap_or(rusqlite::types::ValueRef::Null),
                    )
                })
                .collect();
            let session_id = crate::adapters::shared::dialect::python_str(&values[session_index]);
            if let Some(digest) = digests.get_mut(&session_id) {
                hash_row(digest, table, &values);
            }
        }
    }

    let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (session_id, parent) in &sessions {
        if let Some(parent) = parent {
            children
                .entry(parent.clone())
                .or_default()
                .push(session_id.clone());
        }
    }
    let revisions = digests
        .into_iter()
        .map(|(session_id, digest)| (session_id, digest.finalize().into()))
        .collect();
    Ok((sessions, revisions, children))
}

/// 只读取目标会话及其 SQLite parent_id 子树，保持与整库索引相同的逐行哈希口径。
fn read_target_fingerprint_index(session_id: &str) -> rusqlite::Result<IndexTriple> {
    let database = store::database_path();
    let resolved = std::fs::canonicalize(&database).unwrap_or(database);
    let connection = open_readonly(&resolved)?;
    connection.execute_batch("BEGIN")?;

    let tables = table_names(&connection)?;
    if !tables.iter().any(|name| name == "session") {
        return Ok((BTreeMap::new(), BTreeMap::new(), BTreeMap::new()));
    }
    let session_columns = store::table_columns(&connection, "session")?;
    let session_id_index = session_columns
        .iter()
        .position(|name| name == "id")
        .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
    let parent_index = session_columns
        .iter()
        .position(|name| name == "parent_id")
        .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
    let subtree = |table: &str, join_column: &str, order: &str| {
        format!(
            "WITH RECURSIVE subtree(id) AS (\
                 SELECT id FROM session WHERE id = ?1 \
                 UNION \
                 SELECT child.id FROM session child JOIN subtree parent \
                   ON child.parent_id = parent.id\
             ) \
             SELECT source.* FROM \"{table}\" source JOIN subtree \
               ON source.\"{join_column}\" = subtree.id ORDER BY {order}"
        )
    };

    let mut sessions: BTreeMap<String, Option<String>> = BTreeMap::new();
    let mut digests: BTreeMap<String, Sha256> = BTreeMap::new();
    {
        let mut statement = connection.prepare(&subtree("session", "id", "source.id"))?;
        let width = session_columns.len();
        let mut rows = statement.query([session_id])?;
        while let Some(row) = rows.next()? {
            let values: Vec<Value> = (0..width)
                .map(|index| {
                    store::cell_to_json(
                        row.get_ref(index)
                            .unwrap_or(rusqlite::types::ValueRef::Null),
                    )
                })
                .collect();
            let current = crate::adapters::shared::dialect::python_str(&values[session_id_index]);
            let parent = match &values[parent_index] {
                Value::Null => None,
                other => Some(crate::adapters::shared::dialect::python_str(other)),
            };
            sessions.insert(current.clone(), parent);
            let mut digest = Sha256::new();
            hash_row(&mut digest, "session", &values);
            digests.insert(current, digest);
        }
    }

    for table in ["message", "part"] {
        if !tables.iter().any(|name| name == table) {
            continue;
        }
        let columns = store::table_columns(&connection, table)?;
        let Some(session_index) = columns.iter().position(|name| name == "session_id") else {
            continue;
        };
        let mut statement = connection.prepare(&subtree(
            table,
            "session_id",
            "source.session_id, source.id",
        ))?;
        let width = columns.len();
        let mut rows = statement.query([session_id])?;
        while let Some(row) = rows.next()? {
            let values: Vec<Value> = (0..width)
                .map(|index| {
                    store::cell_to_json(
                        row.get_ref(index)
                            .unwrap_or(rusqlite::types::ValueRef::Null),
                    )
                })
                .collect();
            let owner = crate::adapters::shared::dialect::python_str(&values[session_index]);
            if let Some(digest) = digests.get_mut(&owner) {
                hash_row(digest, table, &values);
            }
        }
    }

    let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (current, parent) in &sessions {
        if let Some(parent) = parent {
            children
                .entry(parent.clone())
                .or_default()
                .push(current.clone());
        }
    }
    let revisions = digests
        .into_iter()
        .map(|(current, digest)| (current, digest.finalize().into()))
        .collect();
    Ok((sessions, revisions, children))
}

fn load_fingerprint_store() -> Option<FingerprintIndex> {
    let raw = std::fs::read_to_string(fingerprint_store_path()).ok()?;
    let data: Value = serde_json::from_str(&raw).ok()?;
    let entries = data.as_object()?;
    if entries.get("version").and_then(Value::as_i64) != Some(FINGERPRINT_STORE_VERSION) {
        return None;
    }
    if entries.get("db").and_then(Value::as_str)
        != Some(store::database_path().to_string_lossy().as_ref())
    {
        return None;
    }
    let stamp = entries.get("stamp")?.as_array()?.clone();
    let sessions_raw = entries.get("sessions")?.as_object()?;
    let revisions_raw = entries.get("revisions")?.as_object()?;
    if sessions_raw.len() != revisions_raw.len()
        || sessions_raw
            .keys()
            .any(|key| !revisions_raw.contains_key(key))
    {
        return None;
    }
    let mut revisions = BTreeMap::new();
    for (session_id, value) in revisions_raw {
        let text = value.as_str()?;
        if text.len() != 64 {
            return None;
        }
        let mut digest = [0u8; 32];
        for (index, slot) in digest.iter_mut().enumerate() {
            *slot = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok()?;
        }
        revisions.insert(session_id.clone(), digest);
    }
    let sessions: BTreeMap<String, Option<String>> = sessions_raw
        .iter()
        .map(|(session_id, parent)| {
            let parent = match parent {
                Value::Null => None,
                other => Some(crate::adapters::shared::dialect::python_str(other)),
            };
            (session_id.clone(), parent)
        })
        .collect();
    let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (session_id, parent) in &sessions {
        if let Some(parent) = parent {
            children
                .entry(parent.clone())
                .or_default()
                .push(session_id.clone());
        }
    }
    Some(FingerprintIndex {
        stamp,
        sessions,
        revisions,
        children,
    })
}

fn save_fingerprint_store(index: &FingerprintIndex) {
    let mut payload = Map::new();
    payload.insert("version".into(), Value::from(FINGERPRINT_STORE_VERSION));
    payload.insert(
        "db".into(),
        Value::from(store::database_path().to_string_lossy().into_owned()),
    );
    payload.insert("stamp".into(), Value::Array(index.stamp.clone()));
    payload.insert(
        "sessions".into(),
        Value::Object(
            index
                .sessions
                .iter()
                .map(|(session_id, parent)| {
                    (
                        session_id.clone(),
                        parent.clone().map_or(Value::Null, Value::from),
                    )
                })
                .collect(),
        ),
    );
    payload.insert(
        "revisions".into(),
        Value::Object(
            index
                .revisions
                .iter()
                .map(|(session_id, digest)| {
                    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
                    (session_id.clone(), Value::from(hex))
                })
                .collect(),
        ),
    );
    let store_path = fingerprint_store_path();
    let Some(parent) = store_path.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let temporary = store_path.with_file_name(format!(
        "{}.{}.{:?}.tmp",
        store_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        std::process::id(),
        std::thread::current().id(),
    ));
    if std::fs::write(&temporary, Value::Object(payload).to_string()).is_ok() {
        let _ = std::fs::rename(&temporary, &store_path);
    }
}

/// 全量重建；调用方必须持有 [`rebuild_lock`]。
///
/// 三次尝试拿一份前后戳记一致的快照。**不稳定的结果同样发布**：库若被持续写入
/// （多个 opencode 进程同时开着就是这样），稳定永远等不到，不发布就等于缓存恒空，
/// 于是扫描的每一行都会再触发一次整库重建。发布时 stamp 记 `after`，扫描路径本就
/// 容忍旧快照，严格路径见 stamp 不匹配仍会同步重建，语义安全。
fn rebuild_index_locked() -> Option<Arc<FingerprintIndex>> {
    let mut current = None;
    for _ in 0..3 {
        let before = database_stamp();
        let Ok((sessions, revisions, children)) = read_fingerprint_index() else {
            return None;
        };
        let after = database_stamp();
        let index = Arc::new(FingerprintIndex {
            stamp: after.clone(),
            sessions,
            revisions,
            children,
        });
        let stable = before == after;
        current = Some(Arc::clone(&index));
        state().current = Some(Arc::clone(&index));
        save_fingerprint_store(&index);
        if stable {
            break;
        }
    }
    current
}

/// 扫描收尾钩子：快照落后于库时在后台补一次重建。
///
/// 重建持整库读，与扫描并行会把扫描拖慢数倍，所以**不在扫描过程中**调度。
pub fn ensure_fingerprint_index_fresh() {
    if !store::database_path().exists() {
        return;
    }
    let stamp = database_stamp();
    {
        let state = state();
        if state
            .current
            .as_ref()
            .is_some_and(|index| index.stamp == stamp)
        {
            return;
        }
    }
    schedule_background_rebuild();
}

fn schedule_background_rebuild() {
    {
        let mut state = state();
        if state.rebuilding {
            return;
        }
        state.rebuilding = true;
    }
    let spawned = std::thread::Builder::new()
        .name("opencode-fingerprint-rebuild".into())
        .spawn(|| {
            {
                let _guard = rebuild_lock();
                let stamp = database_stamp();
                let fresh = state()
                    .current
                    .as_ref()
                    .is_some_and(|index| index.stamp == stamp);
                if !fresh {
                    rebuild_index_locked();
                }
            }
            state().rebuilding = false;
        });
    if spawned.is_err() {
        state().rebuilding = false;
    }
}

/// 取当前索引；`allow_stale` 决定库变更后是吃旧快照还是同步重建。
fn current_index(allow_stale: bool) -> Option<Arc<FingerprintIndex>> {
    let stamp = database_stamp();
    let mut cached = state().current.clone();
    if cached.as_ref().is_some_and(|index| index.stamp == stamp) {
        return cached;
    }
    if cached.is_none() {
        let guard = rebuild_lock();
        cached = state().current.clone();
        if cached.is_none() {
            // 进程冷启动：先吃上一次进程落盘的成果。库没变它就是新鲜的；
            // 库变了它仍可作为扫描路径的旧快照，严格路径会按 stamp 重建。
            if let Some(loaded) = load_fingerprint_store() {
                let loaded = Arc::new(loaded);
                state().current = Some(Arc::clone(&loaded));
                cached = Some(loaded);
            }
        }
        if cached.is_none() {
            // 连旧快照都没有，stale 与否都只能同步建一次。
            return rebuild_index_locked();
        }
        drop(guard);
        if cached
            .as_ref()
            .is_some_and(|index| index.stamp == database_stamp())
        {
            return cached;
        }
    }
    // 有旧快照但库已变化：扫描路径吃旧快照，重建由扫描收尾的
    // ensure_fingerprint_index_fresh 统一调度；严格路径同步重建。
    if allow_stale {
        return cached;
    }
    let _guard = rebuild_lock();
    let current = state().current.clone();
    if current
        .as_ref()
        .is_some_and(|index| index.stamp == database_stamp())
    {
        return current;
    }
    rebuild_index_locked()
}

fn fingerprint_from_index(session_id: &str, cached: &FingerprintIndex) -> Option<String> {
    cached.sessions.contains_key(session_id).then_some(())?;
    let mut digest = Sha256::new();
    let mut pending = vec![session_id.to_string()];
    let mut seen: HashSet<String> = HashSet::new();
    while let Some(current) = pending.pop() {
        if !seen.insert(current.clone()) {
            continue;
        }
        let parent = match cached.sessions.get(&current) {
            Some(Some(parent)) => parent.clone(),
            // Python 的 f-string 会把 None 渲染成字面量 "None"。
            Some(None) => "None".to_string(),
            None => continue,
        };
        digest.update(format!("\0{current}\0{parent}\0").as_bytes());
        if let Some(revision) = cached.revisions.get(&current) {
            digest.update(revision);
        }
        if let Some(children) = cached.children.get(&current) {
            let mut ordered = children.clone();
            ordered.sort_unstable();
            ordered.reverse();
            pending.extend(ordered);
        }
    }
    let hex: String = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Some(format!("sha256:{hex}"))
}

fn subtree_fingerprint(session_id: &str, allow_stale: bool) -> Option<String> {
    if !store::database_path().exists() {
        return None;
    }
    let cached = current_index(allow_stale)?;
    let fingerprint = fingerprint_from_index(session_id, &cached);
    if fingerprint.is_none() && allow_stale && cached.stamp != database_stamp() {
        // 快照落后于库：比快照更新的会话还没进快照。给占位指纹保住索引条目。
        return Some(format!("sha256:pending-{session_id}"));
    }
    fingerprint
}

fn compute_strict_fingerprint(session_id: &str) -> Option<StrictFingerprintEntry> {
    let mut last = None;
    for _ in 0..3 {
        let before = database_stamp();
        let (sessions, revisions, children) = read_target_fingerprint_index(session_id).ok()?;
        let stamp = database_stamp();
        let index = FingerprintIndex {
            stamp: stamp.clone(),
            sessions,
            revisions,
            children,
        };
        last = Some(StrictFingerprintEntry {
            stamp,
            value: fingerprint_from_index(session_id, &index),
        });
        if before == index.stamp {
            break;
        }
    }
    last
}

/// Agent 路径的严格指纹：库变化后只哈希目标 SQLite 子树，并按 stamp 有界缓存。
pub fn fingerprint(session_id: &str) -> Option<String> {
    if !store::database_path().exists() {
        return None;
    }
    let stamp = database_stamp();
    if let Some(cached) = state().strict.get(session_id, &stamp) {
        return cached;
    }
    let entry = compute_strict_fingerprint(session_id)?;
    let value = entry.value.clone();
    state().strict.insert(session_id, entry);
    value
}

/// 扫描路径的指纹：容忍落后一轮的快照。
///
/// 全量扫描对每个会话都要指纹，库一有写入就同步整库重建会把每次刷新拖慢数秒。
/// 扫描期间吃旧快照：UI 列表的新鲜度由 session 表的 `updated` 保证，Agent 读写
/// 路径仍走严格的 [`fingerprint`]。
pub fn scan_fingerprint(session_id: &str) -> Option<String> {
    subtree_fingerprint(session_id, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct NoCache;

    impl ScanCache for NoCache {
        fn get(&self, _path: &Path, _stat: &FileStat) -> Option<Option<ScanRow>> {
            None
        }
        fn put(&self, _path: &Path, _stat: &FileStat, _meta: Option<ScanRow>) {}
        fn get_digest(&self, _path: &Path, _stat: &FileStat) -> Option<String> {
            None
        }
        fn put_digest(&self, _path: &Path, _stat: &FileStat, _digest: &str) {}
        fn flush(&self) {}
    }

    use store::tests::fixture;

    /// 单测独占库路径与索引（进程内静态状态）。
    fn guard() -> MutexGuard<'static, ()> {
        let guard = store::tests::exclusive();
        reset_fingerprint_index();
        guard
    }

    #[test]
    fn a_missing_database_scans_to_nothing() {
        let _guard = guard();
        let root = tempfile::tempdir().unwrap();
        store::set_database_path_override(Some(root.path().join("absent.db")));
        let rows = scan(&NoCache).unwrap();
        assert!(fingerprint("anything").is_none());
        store::set_database_path_override(None);
        assert!(rows.is_empty());
    }

    #[test]
    fn the_database_stamp_never_includes_the_shm_sidecar() {
        let _guard = guard();
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("opencode.db");
        std::fs::write(&database, b"x").unwrap();
        std::fs::write(root.path().join("opencode.db-shm"), b"x").unwrap();
        store::set_database_path_override(Some(database.clone()));
        let stamp = database_stamp();
        store::set_database_path_override(None);

        assert_eq!(stamp.len(), 2);
        let paths: Vec<&str> = stamp
            .iter()
            .map(|entry| entry[0].as_str().unwrap())
            .collect();
        assert!(paths[0].ends_with("opencode.db"));
        assert!(paths[1].ends_with("opencode.db-wal"));
        assert!(!paths.iter().any(|path| path.ends_with("-shm")));
        // 不存在的 -wal 只留 [路径, null] 两项。
        assert_eq!(stamp[1].as_array().unwrap().len(), 2);
        assert_eq!(stamp[0].as_array().unwrap().len(), 5);
    }

    #[test]
    fn fingerprints_are_session_scoped_and_change_with_content() {
        let _guard = guard();
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("opencode.db");
        store::tests::materialize(&database, &fixture("case-01-plain"));
        store::set_database_path_override(Some(database.clone()));
        // 落盘快照隔离到临时目录，避免污染真实 ~/.ferry。
        let data_dir = root.path().join("ferry-data");
        let _env =
            crate::system::paths::testing::EnvGuard::acquire().set("FERRY_DATA_DIR", &data_dir);

        let plain = fixture("case-01-plain");
        let session_id = plain["session"]["id"].as_str().unwrap();
        let first = fingerprint(session_id).expect("已知会话必有指纹");
        assert!(first.starts_with("sha256:"));
        // 幂等：库不变 → 指纹不变。
        assert_eq!(fingerprint(session_id).as_deref(), Some(first.as_str()));
        assert_eq!(fingerprint("not-a-session"), None);

        // 库变更 → 指纹变化（严格路径同步重建）。
        reset_fingerprint_index();
        std::fs::remove_file(&database).unwrap();
        store::tests::materialize(&database, &fixture("case-02-tools"));
        let tools = fixture("case-02-tools");
        let other = fingerprint(tools["session"]["id"].as_str().unwrap()).unwrap();
        assert_ne!(other, first);

        store::set_database_path_override(None);
    }

    #[test]
    fn strict_fingerprint_matches_full_index_and_ignores_unrelated_writes() {
        let _guard = guard();
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("opencode.db");
        store::tests::materialize(&database, &fixture("case-01-plain"));
        let payload = fixture("case-01-plain");
        let root_id = payload["session"]["id"].as_str().unwrap();
        {
            let connection = Connection::open(&database).unwrap();
            connection
                .execute(
                    "INSERT INTO session (id, parent_id) VALUES (?1, ?2)",
                    ["child-session", root_id],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO session (id) VALUES (?1)",
                    ["unrelated-session"],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO message (id, session_id, data, time_created) \
                     VALUES ('child-message', 'child-session', '{}', 1)",
                    [],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO message (id, session_id, data, time_created) \
                     VALUES ('unrelated-message', 'unrelated-session', '{}', 1)",
                    [],
                )
                .unwrap();
        }
        store::set_database_path_override(Some(database.clone()));
        let (sessions, revisions, children) = read_fingerprint_index().unwrap();
        let full = FingerprintIndex {
            stamp: database_stamp(),
            sessions,
            revisions,
            children,
        };
        let expected = fingerprint_from_index(root_id, &full).unwrap();
        assert_eq!(fingerprint(root_id).as_deref(), Some(expected.as_str()));

        {
            let connection = Connection::open(&database).unwrap();
            connection
                .execute(
                    "INSERT INTO part (id, message_id, session_id, data, time_created) \
                     VALUES ('unrelated-part', 'unrelated-message', 'unrelated-session', '{}', 2)",
                    [],
                )
                .unwrap();
        }
        assert_eq!(fingerprint(root_id).as_deref(), Some(expected.as_str()));

        {
            let connection = Connection::open(&database).unwrap();
            connection
                .execute(
                    "INSERT INTO part (id, message_id, session_id, data, time_created) \
                     VALUES ('child-part', 'child-message', 'child-session', '{}', 2)",
                    [],
                )
                .unwrap();
        }
        assert_ne!(fingerprint(root_id).as_deref(), Some(expected.as_str()));
        store::set_database_path_override(None);
    }

    #[test]
    fn the_scan_path_returns_a_placeholder_for_sessions_newer_than_the_snapshot() {
        let _guard = guard();
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("opencode.db");
        store::tests::materialize(&database, &fixture("case-01-plain"));
        store::set_database_path_override(Some(database.clone()));
        let data_dir = root.path().join("ferry-data");
        let _env =
            crate::system::paths::testing::EnvGuard::acquire().set("FERRY_DATA_DIR", &data_dir);

        // 扫描路径先建一次全库索引，再把库换掉但不重建 → 快照落后。
        let plain = fixture("case-01-plain");
        assert!(scan_fingerprint(plain["session"]["id"].as_str().unwrap()).is_some());
        std::fs::remove_file(&database).unwrap();
        store::tests::materialize(&database, &fixture("case-02-tools"));

        let tools = fixture("case-02-tools");
        let newcomer = tools["session"]["id"].as_str().unwrap();
        assert_eq!(
            scan_fingerprint(newcomer),
            Some(format!("sha256:pending-{newcomer}"))
        );
        // 严格路径不接受占位值，会同步重建成真实指纹。
        let strict = fingerprint(newcomer).unwrap();
        assert!(strict.starts_with("sha256:"));
        assert!(!strict.contains("pending"));

        store::set_database_path_override(None);
    }

    #[test]
    fn the_persisted_snapshot_survives_a_cold_process() {
        let _guard = guard();
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("opencode.db");
        store::tests::materialize(&database, &fixture("case-01-plain"));
        store::set_database_path_override(Some(database.clone()));
        let data_dir = root.path().join("ferry-data");
        let _env =
            crate::system::paths::testing::EnvGuard::acquire().set("FERRY_DATA_DIR", &data_dir);

        let plain = fixture("case-01-plain");
        let session_id = plain["session"]["id"].as_str().unwrap();
        let expected = scan_fingerprint(session_id).unwrap();
        assert!(data_dir.join("opencode-fingerprints.json").is_file());

        // 模拟冷启动：清空进程内索引，只剩落盘快照。
        reset_fingerprint_index();
        assert_eq!(
            scan_fingerprint(session_id).as_deref(),
            Some(expected.as_str())
        );

        // version 不匹配的快照整体作废。
        std::fs::write(
            data_dir.join("opencode-fingerprints.json"),
            json!({"version": 99}).to_string(),
        )
        .unwrap();
        reset_fingerprint_index();
        assert!(load_fingerprint_store().is_none());

        store::set_database_path_override(None);
    }

    /// 黄金对照：扫描行必须与 `tests/golden/scan/opencode/` 的基线逐字段一致。
    ///
    /// `_normalized.environment_dependent_fields` 里的字段由运行环境决定
    /// （opencode 只有 `updated` / `own_updated`），对照前先抹掉。
    #[test]
    fn scan_rows_match_the_python_golden_baseline() {
        let _guard = guard();
        for case in ["case-01-plain", "case-02-tools"] {
            let root = tempfile::tempdir().unwrap();
            let database = root.path().join("opencode.db");
            store::tests::materialize(&database, &fixture(case));
            store::set_database_path_override(Some(database));
            let rows = scan(&NoCache).unwrap();
            store::set_database_path_override(None);

            let golden: Value = serde_json::from_str(
                &std::fs::read_to_string(
                    Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../../tests/golden/scan/opencode")
                        .join(format!("{case}.json")),
                )
                .expect("黄金基线可读"),
            )
            .unwrap();
            let volatile: Vec<String> = golden["_normalized"]["environment_dependent_fields"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap().to_string())
                .collect();
            assert_eq!(volatile, ["updated", "own_updated"]);

            let strip = |value: &Value| -> Value {
                fn walk(value: &Value, volatile: &[String]) -> Value {
                    match value {
                        Value::Object(entries) => Value::Object(
                            entries
                                .iter()
                                .filter(|(key, _)| !volatile.contains(key))
                                .map(|(key, item)| (key.clone(), walk(item, volatile)))
                                .collect(),
                        ),
                        Value::Array(items) => {
                            Value::Array(items.iter().map(|item| walk(item, volatile)).collect())
                        }
                        other => other.clone(),
                    }
                }
                walk(value, &volatile)
            };
            assert_eq!(
                strip(&Value::Array(rows.into_iter().map(Value::Object).collect())),
                strip(&golden["rows"]),
                "{case} 的扫描行与黄金基线不一致"
            );
        }
    }

    #[test]
    fn hash_row_length_prefix_prevents_boundary_collisions() {
        let mut left = Sha256::new();
        hash_row(&mut left, "session", &[json!("ab"), json!("c")]);
        let mut right = Sha256::new();
        hash_row(&mut right, "session", &[json!("a"), json!("bc")]);
        assert_ne!(left.finalize(), right.finalize());
    }
}
