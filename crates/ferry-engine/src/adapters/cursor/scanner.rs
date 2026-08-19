//! Cursor 会话枚举与会话级指纹。
//!
//! 枚举以 `composerHeaders` 表为准（它才有索引化的 workspaceId / isSubagent /
//! isArchived / recency，Cursor 侧边栏就按它渲染），内容取 `composerData:<id>`。
//! **列表页只查这两处，绝不碰 bubble**：本机 bubble 有 153 789 行 / 1.15 GB，
//! 而 composerData 只有 269 行 / 10 MB。
//!
//! 指纹必须是**会话级**的：所有 Cursor 会话同住一个库，把整库 stat 混进指纹会
//! 让任何其它会话的写入都作废本会话的引用与迁移计划。会话内容一旦变化，
//! `composerData` 的 `fullConversationHeadersOnly` 与 header 行的 recency 必然
//! 跟着变，所以指纹只需覆盖这两处，不必逐条 bubble 哈希——整库一遍只有 10 MB，
//! 同步重建即可，不需要 opencode 那套落盘快照与后台线程。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

use rusqlite::Connection;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::adapters::contracts::{ScanCache, ScanRow};
use crate::adapters::shared::scanner::session_roots;
use crate::errors::DomainResult;
use crate::jsonutil::FileStat;

use super::native_schema::{ComposerData, Head};
use super::store;

/// 库戳记：`[(路径, dev, ino, mtime_ns, size)]`，取不到 stat 时是 `[路径, null]`。
type Stamp = Vec<Value>;

/// 一个会话在库里的原始元数据（header 行 + composerData）。
struct NativeSession {
    id: String,
    head: Head,
    head_raw: String,
    created: i64,
    updated: i64,
    data: Option<ComposerData>,
    data_raw: String,
}

impl NativeSession {
    /// 父会话：只认 `subagentInfo.parentComposerId`。
    fn parent_id(&self) -> Option<&str> {
        self.head
            .subagent_info
            .as_ref()?
            .parent_composer_id
            .as_deref()
            .filter(|value| !value.is_empty())
    }

    fn title(&self) -> String {
        let data_name = self.data.as_ref().and_then(|data| data.name.as_deref());
        self.head
            .name
            .as_deref()
            .or(data_name)
            .or(self.head.subtitle.as_deref())
            .unwrap_or_default()
            .to_string()
    }

    /// 工作目录：head 优先，缺失时回落 composerData（v16 只有 40/176 带它）。
    fn cwd(&self) -> String {
        let from = |identifier: Option<&super::native_schema::WorkspaceIdentifier>| {
            identifier?.uri.as_ref()?.local_path().map(str::to_string)
        };
        from(self.head.workspace_identifier.as_ref())
            .or_else(|| {
                from(
                    self.data
                        .as_ref()
                        .and_then(|data| data.workspace_identifier.as_ref()),
                )
            })
            .unwrap_or_default()
    }

    fn message_count(&self) -> i64 {
        self.data
            .as_ref()
            .map_or(0, |data| data.headers.len() as i64)
    }
}

/// 一次整库读取的结果，扫描与指纹共用。
fn read_sessions(connection: &Connection) -> rusqlite::Result<Vec<NativeSession>> {
    let mut composer_data: BTreeMap<String, String> = BTreeMap::new();
    {
        // key 上有 UNIQUE 索引，GLOB 前缀是范围扫描，不会碰 bubble 行。
        let mut statement = connection
            .prepare("SELECT key, value FROM cursorDiskKV WHERE key GLOB 'composerData:*'")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let key = store::text_cell(row.get_ref(0)?);
            let Some(id) = key.strip_prefix("composerData:") else {
                continue;
            };
            composer_data.insert(id.to_string(), store::text_cell(row.get_ref(1)?));
        }
    }

    let mut statement = connection.prepare(
        "SELECT composerId, createdAt, lastUpdatedAt, recency, value FROM composerHeaders",
    )?;
    let mut rows = statement.query([])?;
    let mut sessions = Vec::new();
    while let Some(row) = rows.next()? {
        let id = store::text_cell(row.get_ref(0)?);
        if id.is_empty() {
            continue;
        }
        let created: i64 = row.get::<_, Option<i64>>(1)?.unwrap_or(0);
        let last_updated: Option<i64> = row.get(2)?;
        let recency: Option<i64> = row.get(3)?;
        let head_raw = store::text_cell(row.get_ref(4)?);
        let data_raw = composer_data.remove(&id).unwrap_or_default();
        sessions.push(NativeSession {
            head: serde_json::from_str(&head_raw).unwrap_or_default(),
            created,
            // recency 就是 Cursor 的排序键：lastUpdatedAt 存在时等于它，否则 createdAt。
            updated: recency.or(last_updated).unwrap_or(created),
            data: serde_json::from_str(&data_raw).ok(),
            head_raw,
            data_raw,
            id,
        });
    }
    Ok(sessions)
}

/// 扫描全库；库不存在或结构漂移返回空清单（不是错误，只让 cursor 一栏空着）。
///
/// `cache` 不参与：扫描是一次整库查询，没有可缓存的按文件粒度。
pub fn scan(_cache: &dyn ScanCache) -> DomainResult<Vec<ScanRow>> {
    let database = store::database_path();
    if !database.exists() {
        return Ok(Vec::new());
    }
    let Ok(connection) = store::open_readonly(&database) else {
        return Ok(Vec::new());
    };
    if store::validate_schema(&connection).is_err() {
        return Ok(Vec::new());
    }
    let Ok(sessions) = read_sessions(&connection) else {
        return Ok(Vec::new());
    };

    let rows: Vec<ScanRow> = sessions
        .iter()
        .map(|session| {
            let mut row = ScanRow::new();
            row.insert("tool".into(), Value::from("cursor"));
            row.insert("id".into(), Value::from(session.id.as_str()));
            row.insert("title".into(), Value::from(session.title()));
            row.insert("dir".into(), Value::from(session.cwd()));
            row.insert("updated".into(), Value::from(session.updated));
            row.insert("created".into(), Value::from(session.created));
            row.insert("count".into(), Value::from(session.message_count()));
            // Cursor 会话不落文件：路径恒为空串、体积恒为 0。
            row.insert("size".into(), Value::from(0));
            row.insert("path".into(), Value::from(""));
            row.insert(
                "parent_id".into(),
                session.parent_id().map_or(Value::Null, Value::from),
            );
            // tokenCount 全库为 0、usageData 全空：Cursor 不落用量，给 null 而不是零桶。
            row.insert("tokens".into(), Value::Null);
            row.insert(
                "model".into(),
                Value::from(
                    session
                        .data
                        .as_ref()
                        .and_then(ComposerData::model)
                        .unwrap_or_default(),
                ),
            );
            row
        })
        .collect();

    // 空会话（本机 269 里有 155 个）在 Cursor 里是从未发过消息的草稿；按树汇总后
    // 仍为 0 条才丢弃，这样「自己空但带子代理」的父会话不会连子树一起消失。
    Ok(session_roots(rows)?
        .into_iter()
        .filter(|root| root.get("count").and_then(Value::as_i64).unwrap_or(0) != 0)
        .collect())
}

// ---------------------------------------------------------------------------
// 库戳记
// ---------------------------------------------------------------------------

/// 只 stat `.db` 与 `-wal`。
///
/// **故意排除 `-shm`**：它只是 WAL 的共享内存索引，连只读连接都会更新它的
/// mtime。把它算进戳记会让指纹缓存被自己的读取动作反复失效；数据变更必然体现在
/// 主库或 `-wal` 上，排除它不损失正确性。
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

/// 活索引轮询探针：只认会话集合本身的变化。
///
/// 不能沿用 [`database_stamp`]：Cursor 运行期间 `state.vscdb-wal` 几乎一直在变
/// （同一个库还存着大量 UI 状态），stat 派生的戳记会把活索引打成持续全量重扫。
/// 这里改从内容派生——`composerHeaders` 的行数与最新时间戳，会话真变了才变。
/// 库缺失或打不开时给 `[路径, null, null]`（对齐 opencode 取不到 stat 的形状）：
/// 是一个稳定令牌，不会自己抖动，也不会被 live.rs 当成探测失败。
pub fn watch_stamp() -> Value {
    let database = store::database_path();
    let path = Value::from(database.to_string_lossy().into_owned());
    let unknown = Value::Array(vec![path.clone(), Value::Null, Value::Null]);
    let Ok(connection) = store::open_readonly(&database) else {
        return unknown;
    };
    // 两列都可空：空库时 count=0、max 为 NULL。
    let probed = connection.query_row(
        "SELECT COUNT(*), MAX(COALESCE(lastUpdatedAt, createdAt)) FROM composerHeaders",
        [],
        |row| {
            Ok((
                row.get::<_, Option<i64>>(0)?.unwrap_or(0),
                row.get::<_, Option<i64>>(1)?,
            ))
        },
    );
    match probed {
        Ok((count, latest)) => Value::Array(vec![
            path,
            Value::from(count),
            latest.map_or(Value::Null, Value::from),
        ]),
        Err(_) => unknown,
    }
}

// ---------------------------------------------------------------------------
// 会话级指纹
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
struct FingerprintIndex {
    stamp: Stamp,
    /// `会话 id → 自身内容摘要`。
    revisions: BTreeMap<String, [u8; 32]>,
    /// `会话 id → 父 id`。
    parents: BTreeMap<String, Option<String>>,
    /// `父 id → 子 id 列表`。
    children: BTreeMap<String, Vec<String>>,
}

fn state() -> MutexGuard<'static, Option<FingerprintIndex>> {
    static STATE: OnceLock<Mutex<Option<FingerprintIndex>>> = OnceLock::new();
    STATE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 重建时的独占锁：扫描会并行为每一行取指纹，不加锁会同时重建 N 遍。
fn rebuild_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 清空进程内索引（单测隔离用）。
pub fn reset_fingerprint_index() {
    *state() = None;
}

fn build_index() -> Option<FingerprintIndex> {
    let database = store::database_path();
    if !database.exists() {
        return None;
    }
    let connection = store::open_readonly(&database).ok()?;
    store::validate_schema(&connection).ok()?;
    let stamp = database_stamp();
    let sessions = read_sessions(&connection).ok()?;

    let mut revisions = BTreeMap::new();
    let mut parents: BTreeMap<String, Option<String>> = BTreeMap::new();
    let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for session in &sessions {
        let mut digest = Sha256::new();
        // 长度前缀让不同分段不可能互相碰撞。
        for part in [
            session.id.as_str(),
            session.head_raw.as_str(),
            session.data_raw.as_str(),
        ] {
            digest.update((part.len() as u64).to_be_bytes());
            digest.update(part.as_bytes());
        }
        digest.update(session.updated.to_be_bytes());
        revisions.insert(session.id.clone(), digest.finalize().into());
        let parent = session.parent_id().map(str::to_string);
        if let Some(parent) = &parent {
            children
                .entry(parent.clone())
                .or_default()
                .push(session.id.clone());
        }
        parents.insert(session.id.clone(), parent);
    }
    Some(FingerprintIndex {
        stamp,
        revisions,
        parents,
        children,
    })
}

fn current_index() -> Option<FingerprintIndex> {
    let stamp = database_stamp();
    if let Some(index) = state().as_ref() {
        if index.stamp == stamp {
            return Some(index.clone());
        }
    }
    let _guard = rebuild_lock();
    // 拿到锁时可能已被别的线程重建过。
    if let Some(index) = state().as_ref() {
        if index.stamp == database_stamp() {
            return Some(index.clone());
        }
    }
    let index = build_index()?;
    *state() = Some(index.clone());
    Some(index)
}

/// 会话及其全部子代理的内容指纹。
///
/// 会话不在库里时返回 `None`——调用方据此把该行踢出索引。
pub fn fingerprint(session_id: &str) -> Option<String> {
    let index = current_index()?;
    if !index.parents.contains_key(session_id) {
        return None;
    }
    let mut digest = Sha256::new();
    let mut pending = vec![session_id.to_string()];
    let mut seen: Vec<String> = Vec::new();
    while let Some(current) = pending.pop() {
        if seen.contains(&current) {
            continue;
        }
        let Some(parent) = index.parents.get(&current) else {
            continue;
        };
        seen.push(current.clone());
        digest.update(format!("\0{current}\0{}\0", parent.as_deref().unwrap_or("")).as_bytes());
        if let Some(revision) = index.revisions.get(&current) {
            digest.update(revision);
        }
        if let Some(children) = index.children.get(&current) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::cursor::store::tests::{exclusive, materialize};
    use serde_json::json;
    use std::path::Path;

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

    fn guard() -> MutexGuard<'static, ()> {
        let guard = exclusive();
        reset_fingerprint_index();
        guard
    }

    pub(crate) fn fixture() -> Value {
        json!({"sessions": [
            {
                "id": "root-1",
                "header": {
                    "name": "Explore project structure",
                    "subtitle": "Read README.md",
                    "createdAt": 1_787_000_000_000i64,
                    "lastUpdatedAt": 1_787_000_009_000i64,
                    "unifiedMode": "agent",
                    "workspaceIdentifier": {"id": "3d6aae0c", "uri": {
                        "$mid": 1, "scheme": "file",
                        "fsPath": "/Users/u/work", "path": "/Users/u/work",
                        "external": "file:///Users/u/work"}},
                },
                "composerData": {"_v": 17,
                    "fullConversationHeadersOnly": [{"bubbleId": "b1", "type": 1},
                                                    {"bubbleId": "b2", "type": 2}],
                    "modelConfig": {"modelName": "grok-4.5"}},
                "bubbles": {"b1": {"_v": 3, "type": 1, "text": "go"},
                            "b2": {"_v": 3, "type": 2, "text": "done"}},
            },
            {
                "id": "sub-1",
                "subagent": true,
                "header": {"name": "explore", "createdAt": 1_787_000_005_000i64,
                           "subagentInfo": {"parentComposerId": "root-1",
                                            "subagentTypeName": "explore",
                                            "toolCallId": "call_1"}},
                "composerData": {"_v": 16,
                    "fullConversationHeadersOnly": [{"bubbleId": "s1", "type": 1}],
                    "modelConfig": {"modelName": "model-twevgy"}},
                "bubbles": {"s1": {"_v": 3, "type": 1, "text": "sub"}},
            },
            {
                "id": "draft-1",
                "header": {"name": null, "createdAt": 1_787_000_007_000i64,
                           "isArchived": true,
                           "workspaceIdentifier": {"id": "1783251917755"}},
                "composerData": {"_v": 16, "fullConversationHeadersOnly": []},
            },
        ]})
    }

    fn with_fixture(root: &Path) -> PathBuf {
        let database = root.join("state.vscdb");
        materialize(&database, &fixture());
        store::set_database_path_override(Some(database.clone()));
        database
    }

    #[test]
    fn a_missing_database_scans_to_nothing() {
        let _guard = guard();
        let root = tempfile::tempdir().unwrap();
        store::set_database_path_override(Some(root.path().join("absent.vscdb")));
        let rows = scan(&NoCache).unwrap();
        assert!(fingerprint("anything").is_none());
        store::set_database_path_override(None);
        assert!(rows.is_empty());
    }

    #[test]
    fn scan_rows_carry_the_id_shaped_metadata_and_nest_subagents() {
        let _guard = guard();
        let root = tempfile::tempdir().unwrap();
        with_fixture(root.path());
        let rows = scan(&NoCache).unwrap();
        store::set_database_path_override(None);

        // 空草稿被丢弃，子代理挂进父会话而不是占一个顶层槽位。
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row["tool"], json!("cursor"));
        assert_eq!(row["id"], json!("root-1"));
        assert_eq!(row["title"], json!("Explore project structure"));
        assert_eq!(row["dir"], json!("/Users/u/work"));
        assert_eq!(row["model"], json!("grok-4.5"));
        assert_eq!(row["path"], json!(""));
        assert_eq!(row["size"], json!(0));
        assert_eq!(row["tokens"], json!(null));
        assert_eq!(row["own_count"], json!(2));
        // 汇总含子代理。
        assert_eq!(row["count"], json!(3));
        assert_eq!(row["child_count"], json!(1));
        let child = &row["children"][0];
        assert_eq!(child["id"], json!("sub-1"));
        assert_eq!(child["parent_id"], json!("root-1"));
        assert_eq!(child["root_id"], json!("root-1"));
        // 没有工作区的会话 dir 为空串而不是报错。
        assert_eq!(child["dir"], json!(""));
    }

    #[test]
    fn the_database_stamp_never_includes_the_shm_sidecar() {
        let _guard = guard();
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("state.vscdb");
        std::fs::write(&database, b"x").unwrap();
        std::fs::write(root.path().join("state.vscdb-shm"), b"x").unwrap();
        store::set_database_path_override(Some(database));
        let stamp = database_stamp();
        store::set_database_path_override(None);

        assert_eq!(stamp.len(), 2);
        let paths: Vec<&str> = stamp
            .iter()
            .map(|entry| entry[0].as_str().unwrap())
            .collect();
        assert!(paths[0].ends_with("state.vscdb"));
        assert!(paths[1].ends_with("state.vscdb-wal"));
        assert!(!paths.iter().any(|path| path.ends_with("-shm")));
        assert_eq!(stamp[0].as_array().unwrap().len(), 5);
        assert_eq!(stamp[1].as_array().unwrap().len(), 2);
    }

    #[test]
    fn the_watch_stamp_ignores_ui_writes_but_follows_sessions() {
        let _guard = guard();
        let root = tempfile::tempdir().unwrap();
        let database = with_fixture(root.path());

        let before = watch_stamp();
        assert_eq!(before.as_array().unwrap().len(), 3);
        assert_eq!(before[1], json!(3));

        // Cursor 会往同一个库狂写 UI 状态：这类写入不得让活索引重扫。
        {
            let connection = Connection::open(&database).unwrap();
            connection
                .execute(
                    "INSERT INTO ItemTable (key, value) VALUES ('ui.state', 'x')",
                    [],
                )
                .unwrap();
        }
        assert_eq!(watch_stamp(), before);

        // 新会话必须被看见。
        {
            let connection = Connection::open(&database).unwrap();
            connection
                .execute(
                    "INSERT INTO composerHeaders (composerId, createdAt, lastUpdatedAt, value) \
                     VALUES ('root-2', 1787000020000, 1787000030000, '{}')",
                    [],
                )
                .unwrap();
        }
        let after = watch_stamp();
        assert_eq!(after[1], json!(4));
        assert_ne!(after, before);

        store::set_database_path_override(None);
    }

    #[test]
    fn fingerprints_are_session_scoped_and_follow_content() {
        let _guard = guard();
        let root = tempfile::tempdir().unwrap();
        let database = with_fixture(root.path());

        let first = fingerprint("root-1").expect("已知会话必有指纹");
        assert!(first.starts_with("sha256:"));
        assert_eq!(fingerprint("root-1").as_deref(), Some(first.as_str()));
        assert_ne!(fingerprint("sub-1").unwrap(), first);
        assert_eq!(fingerprint("not-a-session"), None);

        // 只改 sub-1 的内容：父会话指纹含子树，必须跟着变。
        reset_fingerprint_index();
        std::fs::remove_file(&database).unwrap();
        let mut changed = fixture();
        changed["sessions"][1]["composerData"]["fullConversationHeadersOnly"] =
            json!([{"bubbleId": "s1"}, {"bubbleId": "s2"}]);
        materialize(&database, &changed);
        assert_ne!(fingerprint("root-1").unwrap(), first);

        store::set_database_path_override(None);
    }
}
