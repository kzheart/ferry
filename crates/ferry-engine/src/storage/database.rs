//! Ferry 自有状态 SQLite 的连接与 schema 组合根。
//!
//! 只有 Engine 打开并写入此数据库。schema 版本白名单 `(0, 9, 10)`，v9→v10 是
//! 单跳迁移（DROP `deletion_recoveries`），v0 直接建 v10 全量表（§2.3 第 18 条）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;
use serde_json::Value;

use crate::operations::history_store::MigrationHistoryStore;
use crate::operations::metadata_store::SessionMetadataStore;
use crate::operations::state_store::OperationStore;
use crate::operations::types::{EngineError, EngineResult};
use crate::runtime::store::RuntimeSessionStore;

pub const SCHEMA_VERSION: i64 = 10;

/// 状态库文件名；`state_dir()` 下唯一。
pub const STATE_DATABASE_FILENAME: &str = "ferry-state.sqlite3";

/// epoch 毫秒。等价 `int(time.time() * 1000)`。
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// 可注入时钟：TTL 惰性过期与审计时间戳都读它。
///
/// Python 侧测试用 `monkeypatch.setattr(plan_store, "now_ms", ...)` 拨钟；
/// Rust 侧改成显式端口，避免全局可变状态在并行测试里互相污染。
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}

/// 真实时钟。
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        now_ms()
    }
}

/// [`crate::jsonutil::canonical_json`] 的能力包包装；NaN/Inf 折成校验错误。
pub fn canonical_json(value: &Value) -> EngineResult<String> {
    Ok(crate::jsonutil::canonical_json(value)?)
}

pub fn digest_json(value_json: &str) -> String {
    crate::jsonutil::digest_json(value_json)
}

pub fn digest_value(value: &Value) -> EngineResult<String> {
    Ok(digest_json(&canonical_json(value)?))
}

/// 连接工厂 + 进程内互斥。
///
/// 对齐 Python 的 `(self._connect, self._lock)` 二元组：连接每次操作现开现关
/// （不跨线程共享 `sqlite3.Connection`），锁保证同一个库文件上的写不并发。
pub struct StateConnector {
    path: PathBuf,
    lock: Mutex<()>,
}

impl StateConnector {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            lock: Mutex::new(()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 打开一条连接并施加 PRAGMA（§2.3 第 16 条）。
    fn connect(&self) -> EngineResult<Connection> {
        let connection = Connection::open(&self.path)?;
        // journal_mode 会返回一行结果，必须用 query_row 而不是 execute。
        connection.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 30000;",
        )?;
        Ok(connection)
    }

    /// 在锁内跑一段连接内的工作；连接在返回时关闭（等价 Python 的即用即弃）。
    pub fn with_connection<T>(
        &self,
        work: impl FnOnce(&Connection) -> EngineResult<T>,
    ) -> EngineResult<T> {
        let _guard = self.lock.lock().unwrap_or_else(|error| error.into_inner());
        let connection = self.connect()?;
        work(&connection)
    }
}

impl std::fmt::Debug for StateConnector {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StateConnector")
            .field("path", &self.path)
            .finish()
    }
}

/// 状态库组合根：四个 store 共享同一个连接工厂与同一把锁。
#[derive(Debug)]
pub struct StateDatabase {
    pub path: PathBuf,
    pub operations: OperationStore,
    pub runtime_sessions: RuntimeSessionStore,
    pub metadata: SessionMetadataStore,
    pub migration_history: MigrationHistoryStore,
}

impl StateDatabase {
    /// 打开（必要时建立）状态库。
    ///
    /// `recover_interrupted` 只在 `OperationService` 的路径上为真：元数据 / 历史
    /// 等读写路径不得把正在执行的 Operation 标为中断（§2.3 第 20 条）。
    pub fn open(path: impl Into<PathBuf>, recover_interrupted: bool) -> EngineResult<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| EngineError::Internal {
                error_type: "OSError",
                message: format!("无法创建状态目录 {}: {error}", parent.display()),
            })?;
        }
        let connector = Arc::new(StateConnector::new(path.clone()));
        initialize(&connector)?;
        let database = Self {
            path,
            operations: OperationStore::new(Arc::clone(&connector)),
            runtime_sessions: RuntimeSessionStore::new(Arc::clone(&connector)),
            metadata: SessionMetadataStore::new(Arc::clone(&connector)),
            migration_history: MigrationHistoryStore::new(connector),
        };
        if recover_interrupted {
            database.operations.recover_interrupted()?;
        }
        Ok(database)
    }
}

/// schema v10 的全量建表脚本；SQL 与 `database.py:93-161` 逐字对齐。
/// BEGIN/COMMIT 由 [`initialize`] 持有：写锁内要先复查版本再决定是否执行。
const CREATE_SCHEMA_V10: &str = r#"
                    CREATE TABLE operation_plans (
                        plan_id TEXT PRIMARY KEY,
                        kind TEXT NOT NULL,
                        input_json TEXT NOT NULL,
                        preview_json TEXT NOT NULL,
                        input_digest TEXT NOT NULL,
                        preview_digest TEXT NOT NULL,
                        base_revision TEXT NOT NULL,
                        document_revision TEXT,
                        created_at INTEGER NOT NULL,
                        expires_at INTEGER NOT NULL,
                        status TEXT NOT NULL,
                        result_json TEXT,
                        error_type TEXT,
                        updated_at INTEGER NOT NULL
                    );
                    CREATE TABLE operation_audit (
                        sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                        plan_id TEXT NOT NULL,
                        event TEXT NOT NULL,
                        at INTEGER NOT NULL,
                        details_json TEXT NOT NULL,
                        FOREIGN KEY(plan_id) REFERENCES operation_plans(plan_id)
                    );
                    CREATE INDEX operation_audit_plan
                        ON operation_audit(plan_id, sequence);
                    CREATE TABLE session_metadata (
                        tool TEXT NOT NULL,
                        session_id TEXT NOT NULL,
                        value_json TEXT NOT NULL,
                        updated_at INTEGER NOT NULL,
                        PRIMARY KEY(tool, session_id)
                    );
                    CREATE TABLE migration_history (
                        sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                        history_id TEXT NOT NULL UNIQUE,
                        entry_json TEXT NOT NULL
                    );
                    CREATE TABLE runtime_sessions (
                        session_id TEXT PRIMARY KEY,
                        metadata_json TEXT NOT NULL,
                        created_at TEXT NOT NULL,
                        updated_at TEXT NOT NULL
                    );
                    CREATE INDEX runtime_sessions_recent
                        ON runtime_sessions(updated_at DESC);
                    CREATE TABLE runtime_messages (
                        session_id TEXT NOT NULL,
                        ordinal INTEGER NOT NULL,
                        message_json TEXT NOT NULL,
                        PRIMARY KEY(session_id, ordinal),
                        FOREIGN KEY(session_id)
                            REFERENCES runtime_sessions(session_id)
                            ON DELETE CASCADE
                    );
                    CREATE TABLE runtime_events (
                        session_id TEXT NOT NULL,
                        seq INTEGER NOT NULL,
                        event_json TEXT NOT NULL,
                        PRIMARY KEY(session_id, seq),
                        FOREIGN KEY(session_id)
                            REFERENCES runtime_sessions(session_id)
                            ON DELETE CASCADE
                    );
                    PRAGMA user_version = 10;
"#;

/// v9 → v10 单跳迁移：删除恢复系统退役。
const MIGRATE_V9_TO_V10: &str = r#"
                    DROP TABLE IF EXISTS deletion_recoveries;
                    PRAGMA user_version = 10;
"#;

fn initialize(connector: &StateConnector) -> EngineResult<()> {
    connector.with_connection(|connection| {
        let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if !matches!(version, 0 | 9 | SCHEMA_VERSION) {
            return Err(EngineError::runtime(format!(
                "Ferry state schema 不受支持: {version}"
            )));
        }
        if version == SCHEMA_VERSION {
            return Ok(());
        }
        // 首建/迁移与并发开库互斥：BEGIN IMMEDIATE 拿到写锁后在事务内复查版本，
        // 输掉竞态的一方（另一连接已建好 schema）直接放弃，避免 CREATE 撞表。
        connection.execute_batch("BEGIN IMMEDIATE;")?;
        let current: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        let script = match current {
            0 => Some(CREATE_SCHEMA_V10),
            9 => Some(MIGRATE_V9_TO_V10),
            SCHEMA_VERSION => None,
            other => {
                let _ = connection.execute_batch("ROLLBACK;");
                return Err(EngineError::runtime(format!(
                    "Ferry state schema 不受支持: {other}"
                )));
            }
        };
        let result = match script {
            Some(script) => connection.execute_batch(script),
            None => Ok(()),
        };
        if let Err(error) = result {
            let _ = connection.execute_batch("ROLLBACK;");
            return Err(error.into());
        }
        connection.execute_batch("COMMIT;")?;
        Ok(())
    })
}

static INSTANCES: LazyLock<Mutex<HashMap<PathBuf, Arc<StateDatabase>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 按库文件复用 `StateDatabase` 实例（对齐 `get_state_database`）。
pub fn get_state_database(
    path: impl AsRef<Path>,
    recover_interrupted: bool,
) -> EngineResult<Arc<StateDatabase>> {
    let key = path.as_ref().to_path_buf();
    let mut instances = INSTANCES.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(existing) = instances.get(&key) {
        return Ok(Arc::clone(existing));
    }
    let database = Arc::new(StateDatabase::open(key.clone(), recover_interrupted)?);
    instances.insert(key, Arc::clone(&database));
    Ok(database)
}

/// 仅供测试：清空实例缓存，避免临时目录复用带来的串扰。
pub fn clear_state_database_cache() {
    INSTANCES
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clear();
}

/// `EngineContext` 下 Ferry 自有状态库的位置。
pub fn state_database_path(state_dir: impl AsRef<Path>) -> PathBuf {
    state_dir.as_ref().join(STATE_DATABASE_FILENAME)
}

/// 按 state_dir 打开状态库（不复用实例、不触发崩溃恢复）。
pub fn state_database(state_dir: impl AsRef<Path>) -> EngineResult<StateDatabase> {
    StateDatabase::open(state_database_path(state_dir), false)
}

/// 同 [`state_database`]，但复用实例。
pub fn cached_state_database(state_dir: impl AsRef<Path>) -> EngineResult<Arc<StateDatabase>> {
    get_state_database(state_database_path(state_dir), false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_version(path: &Path) -> i64 {
        let connection = Connection::open(path).unwrap();
        connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn fresh_database_creates_the_v10_schema() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join(STATE_DATABASE_FILENAME);
        StateDatabase::open(&path, false).unwrap();
        assert_eq!(user_version(&path), 10);

        let connection = Connection::open(&path).unwrap();
        let mut statement = connection
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap();
        let tables: Vec<String> = statement
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(Result::unwrap)
            .filter(|name| !name.starts_with("sqlite_"))
            .collect();
        assert_eq!(
            tables,
            vec![
                "migration_history",
                "operation_audit",
                "operation_plans",
                "runtime_events",
                "runtime_messages",
                "runtime_sessions",
                "session_metadata",
            ]
        );
        let mut indexes = connection
            .prepare("SELECT name FROM sqlite_master WHERE type = 'index' ORDER BY name")
            .unwrap();
        let names: Vec<String> = indexes
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(Result::unwrap)
            .filter(|name| !name.starts_with("sqlite_"))
            .collect();
        assert_eq!(
            names,
            vec!["operation_audit_plan", "runtime_sessions_recent"]
        );
    }

    #[test]
    fn schema_version_whitelist_is_exactly_zero_nine_ten() {
        for version in [1, 2, 3, 4, 5, 6, 7, 8, 11, 99] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join(STATE_DATABASE_FILENAME);
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch(&format!("PRAGMA user_version = {version}"))
                .unwrap();
            drop(connection);
            let error = StateDatabase::open(&path, false).unwrap_err();
            assert_eq!(error.error_type(), "RuntimeError");
            assert!(
                error.message().contains("schema 不受支持"),
                "version={version} message={}",
                error.message()
            );
        }
    }

    #[test]
    fn v9_migrates_in_a_single_hop_and_drops_deletion_recoveries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(STATE_DATABASE_FILENAME);
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE deletion_recoveries (id TEXT PRIMARY KEY);
                 PRAGMA user_version = 9;",
            )
            .unwrap();
        drop(connection);

        StateDatabase::open(&path, false).unwrap();

        assert_eq!(user_version(&path), 10);
        let connection = Connection::open(&path).unwrap();
        let remaining: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'deletion_recoveries'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0);
    }

    #[test]
    fn pragmas_match_the_python_connection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(STATE_DATABASE_FILENAME);
        let database = StateDatabase::open(&path, false).unwrap();
        let connector = StateConnector::new(database.path.clone());
        connector
            .with_connection(|connection| {
                let journal: String = connection
                    .query_row("PRAGMA journal_mode", [], |row| row.get(0))
                    .unwrap();
                assert_eq!(journal, "wal");
                let foreign_keys: i64 = connection
                    .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
                    .unwrap();
                assert_eq!(foreign_keys, 1);
                let busy: i64 = connection
                    .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
                    .unwrap();
                assert_eq!(busy, 30_000);
                Ok(())
            })
            .unwrap();
    }
}
