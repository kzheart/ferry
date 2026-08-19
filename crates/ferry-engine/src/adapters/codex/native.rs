//! Codex 原生 rollout 树闭包。
//!
//! 只重映射线程身份字段，未知记录和模型历史保持原样。迁移 writer 不参与此流程。

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde_json::{Map, Value};

use crate::adapters::shared::scanner::{iter_lines, split_jsonl_lines};
use crate::adapters::shared::writing::python_json_dumps;
use crate::jsonutil::sha256_hex;

use super::topology::rollout_files;

/// 闭包发现/克隆链路的专用错误（Python 的 `CodexCloneError`）。
#[derive(Clone, Debug)]
pub struct CodexCloneError(pub String);

impl std::fmt::Display for CodexCloneError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for CodexCloneError {}

impl From<CodexCloneError> for crate::errors::DomainError {
    fn from(error: CodexCloneError) -> Self {
        crate::errors::DomainError::internal(error.0)
    }
}

type CloneResult<T> = Result<T, CodexCloneError>;

fn fail<T>(message: impl Into<String>) -> CloneResult<T> {
    Err(CodexCloneError(message.into()))
}

/// 一份 Codex 存储（`~/.codex`）的关键路径。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexStore {
    pub home: PathBuf,
    pub sessions_dir: PathBuf,
    pub state_db: Option<PathBuf>,
}

impl CodexStore {
    /// 从任意 rollout 路径向上找 `sessions` 祖先，推出 home 与注册库位置。
    pub fn for_rollout(path: &Path) -> Self {
        let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let mut sessions = None;
        let mut current = path.parent();
        while let Some(candidate) = current {
            if candidate.file_name().and_then(|name| name.to_str()) == Some("sessions") {
                sessions = Some(candidate.to_path_buf());
                break;
            }
            current = candidate.parent();
        }
        let Some(sessions) = sessions else {
            let parent = path.parent().unwrap_or(&path).to_path_buf();
            return Self {
                home: parent.clone(),
                sessions_dir: parent,
                state_db: None,
            };
        };
        let home = sessions
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| sessions.clone());
        let db = home.join("state_5.sqlite");
        Self {
            home,
            sessions_dir: sessions,
            state_db: db.exists().then_some(db),
        }
    }
}

/// 一条原生 rollout 的身份与内容。
#[derive(Clone, Debug)]
pub struct NativeRollout {
    pub thread_id: String,
    pub root_id: String,
    pub parent_id: Option<String>,
    pub path: PathBuf,
    pub records: Vec<Value>,
    pub digest: String,
}

/// 以某条 rollout 为锚点的可达会话闭包。
#[derive(Clone, Debug)]
pub struct CodexClosure {
    pub anchor_id: String,
    pub root_id: String,
    pub nodes: HashMap<String, NativeRollout>,
    pub parents: HashMap<String, String>,
    pub store: CodexStore,
    pub revision: String,
    pub registry_revision: Option<String>,
    pub pruned_ids: HashSet<String>,
}

/// 承载线程 id 的字段名。
pub const THREAD_KEYS: [&str; 9] = [
    "threadId",
    "thread_id",
    "agent_thread_id",
    "sender_thread_id",
    "new_thread_id",
    "parent_thread_id",
    "child_thread_id",
    "session_id",
    "forked_from_id",
];

/// 值本身可能是内嵌 JSON 的字段名。
pub const JSON_STRING_KEYS: [&str; 5] = ["arguments", "output", "input", "metadata", "state"];

fn read_jsonl(path: &Path) -> CloneResult<(Vec<Value>, String)> {
    let raw = fs::read(path).map_err(|error| {
        CodexCloneError(format!(
            "无法解析 Codex rollout: {}: {error}",
            path.display()
        ))
    })?;
    let text = std::str::from_utf8(&raw).map_err(|error| {
        CodexCloneError(format!(
            "无法解析 Codex rollout: {}: {error}",
            path.display()
        ))
    })?;
    let mut records = Vec::new();
    for line in split_jsonl_lines(text) {
        if line.trim().is_empty() {
            continue;
        }
        let value = serde_json::from_str::<Value>(line).map_err(|error| {
            CodexCloneError(format!(
                "无法解析 Codex rollout: {}: {error}",
                path.display()
            ))
        })?;
        records.push(value);
    }
    Ok((records, sha256_hex(&raw)))
}

fn canonical_meta(records: &[Value]) -> CloneResult<&Map<String, Value>> {
    records
        .iter()
        .find(|record| {
            record.get("type").and_then(Value::as_str) == Some("session_meta")
                && record.get("payload").is_some_and(Value::is_object)
        })
        .and_then(|record| record.get("payload"))
        .and_then(Value::as_object)
        .ok_or_else(|| CodexCloneError("rollout 缺少 canonical session_meta".into()))
}

/// `(thread_id, root_id, parent_id)`。
fn native_identity(meta: &Map<String, Value>) -> CloneResult<(String, String, Option<String>)> {
    let empty = Map::new();
    let source = meta
        .get("source")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let subagent = source
        .get("subagent")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let spawn = subagent
        .get("thread_spawn")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let Some(current) = meta.get("id") else {
        return fail("rollout 缺少 session_meta.id");
    };
    let current = text(current);
    let root = truthy_text(meta.get("session_id"))
        .or_else(|| truthy_text(spawn.get("session_id")))
        .unwrap_or_else(|| current.clone());
    let parent = truthy_text(meta.get("parent_thread_id"))
        .or_else(|| truthy_text(spawn.get("parent_thread_id")))
        .or_else(|| truthy_text(subagent.get("parent_thread_id")));
    Ok((current, root, parent))
}

fn text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => crate::adapters::shared::dialect::python_str(other),
    }
}

fn truthy_text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Null => None,
        Value::Bool(false) => None,
        Value::String(item) if item.is_empty() => None,
        other => Some(text(other)),
    }
}

fn rollout(path: &Path) -> CloneResult<NativeRollout> {
    let (records, digest) = read_jsonl(path)?;
    let (thread_id, root_id, parent_id) = native_identity(canonical_meta(&records)?)?;
    Ok(NativeRollout {
        thread_id,
        root_id,
        parent_id,
        path: fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()),
        records,
        digest,
    })
}

/// 索引阶段只读到第一条 `session_meta`，避免加载全库所有历史正文。
fn rollout_identity(path: &Path) -> CloneResult<NativeRollout> {
    let lines = iter_lines(path).map_err(|error| {
        CodexCloneError(format!(
            "无法索引 Codex rollout: {}: {error}",
            path.display()
        ))
    })?;
    for line in lines {
        let line = line.map_err(|error| {
            CodexCloneError(format!(
                "无法索引 Codex rollout: {}: {error}",
                path.display()
            ))
        })?;
        if line.trim().is_empty() {
            continue;
        }
        let record = serde_json::from_str::<Value>(&line).map_err(|error| {
            CodexCloneError(format!(
                "无法索引 Codex rollout: {}: {error}",
                path.display()
            ))
        })?;
        if record.get("type").and_then(Value::as_str) != Some("session_meta") {
            continue;
        }
        let Some(payload) = record.get("payload").and_then(Value::as_object) else {
            continue;
        };
        let (thread_id, root_id, parent_id) = native_identity(payload)?;
        return Ok(NativeRollout {
            thread_id,
            root_id,
            parent_id,
            path: fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()),
            records: Vec::new(),
            digest: String::new(),
        });
    }
    fail(format!(
        "rollout 缺少 canonical session_meta: {}",
        path.display()
    ))
}

fn open_readonly(db_path: &Path) -> CloneResult<Connection> {
    let resolved = fs::canonicalize(db_path).unwrap_or_else(|_| db_path.to_path_buf());
    let uri = format!("file:{}?mode=ro", resolved.display());
    Connection::open_with_flags(
        &uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(|error| CodexCloneError(format!("读取 Codex 注册库失败: {error}")))
}

fn db_edges(db_path: Option<&PathBuf>) -> CloneResult<HashSet<(String, String)>> {
    let Some(db_path) = db_path.filter(|path| path.exists()) else {
        return Ok(HashSet::new());
    };
    let connection = open_readonly(db_path)?;
    let exists: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='thread_spawn_edges'",
            [],
            |row| row.get(0),
        )
        .ok();
    if exists.is_none() {
        return Ok(HashSet::new());
    }
    let mut statement = connection
        .prepare("SELECT parent_thread_id, child_thread_id FROM thread_spawn_edges")
        .map_err(|error| CodexCloneError(format!("读取 Codex 注册库失败: {error}")))?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| CodexCloneError(format!("读取 Codex 注册库失败: {error}")))?;
    let mut edges = HashSet::new();
    for row in rows {
        let (parent, child) =
            row.map_err(|error| CodexCloneError(format!("读取 Codex 注册库失败: {error}")))?;
        edges.insert((parent, child));
    }
    Ok(edges)
}

fn closure_revision(nodes: &HashMap<String, NativeRollout>) -> String {
    let mut ids: Vec<&String> = nodes.keys().collect();
    ids.sort();
    let mut payload = Vec::new();
    for id in ids {
        let node = &nodes[id];
        payload.extend_from_slice(id.as_bytes());
        payload.push(0);
        payload.extend_from_slice(node.path.to_string_lossy().as_bytes());
        payload.push(0);
        payload.extend_from_slice(node.digest.as_bytes());
        payload.push(0);
    }
    format!("sha256:{}", sha256_hex(&payload))
}

/// 以 `anchor_path` 为锚点发现整棵可达 rollout 树。
pub fn discover_closure(
    anchor_path: &Path,
    store: Option<CodexStore>,
) -> CloneResult<CodexClosure> {
    let anchor_path = fs::canonicalize(anchor_path).unwrap_or_else(|_| anchor_path.to_path_buf());
    let store = store.unwrap_or_else(|| CodexStore::for_rollout(&anchor_path));
    let mut candidates = if store
        .sessions_dir
        .file_name()
        .and_then(|name| name.to_str())
        == Some("sessions")
    {
        rollout_files(&store.sessions_dir)
    } else {
        vec![anchor_path.clone()]
    };
    let resolved: Vec<PathBuf> = candidates
        .iter()
        .map(|path| fs::canonicalize(path).unwrap_or_else(|_| path.clone()))
        .collect();
    if !resolved.contains(&anchor_path) {
        candidates.push(anchor_path.clone());
    }

    let mut index: HashMap<String, NativeRollout> = HashMap::new();
    let mut by_path: HashMap<PathBuf, NativeRollout> = HashMap::new();
    for path in candidates {
        let node = match rollout_identity(&path) {
            Ok(node) => node,
            Err(error) => {
                let resolved = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
                if resolved == anchor_path {
                    return Err(error);
                }
                continue;
            }
        };
        if let Some(existing) = index.get(&node.thread_id) {
            if existing.path != node.path {
                return fail(format!(
                    "重复 Codex thread id {}: {} / {}",
                    node.thread_id,
                    existing.path.display(),
                    node.path.display()
                ));
            }
        }
        by_path.insert(node.path.clone(), node.clone());
        index.insert(node.thread_id.clone(), node);
    }
    let Some(anchor) = by_path.get(&anchor_path).cloned() else {
        return fail(format!("找不到目标 rollout: {}", anchor_path.display()));
    };

    let mut parents: HashMap<String, String> = index
        .values()
        .filter_map(|node| {
            node.parent_id
                .clone()
                .map(|parent| (node.thread_id.clone(), parent))
        })
        .collect();
    let mut edges: Vec<(String, String)> = db_edges(store.state_db.as_ref())?.into_iter().collect();
    edges.sort();
    for (parent, child) in edges {
        if !index.contains_key(&child) || !index.contains_key(&parent) {
            continue;
        }
        if let Some(declared) = parents.get(&child) {
            if declared != &parent {
                return fail(format!("Codex 文件与 SQLite 父关系冲突: {child}"));
            }
        }
        parents.entry(child).or_insert(parent);
    }

    let mut root_id = anchor.thread_id.clone();
    let mut seen: HashSet<String> = HashSet::new();
    while let Some(parent) = parents.get(&root_id).cloned() {
        if !seen.insert(root_id.clone()) {
            return fail("Codex 会话树存在父链环");
        }
        root_id = parent;
    }
    if !index.contains_key(&root_id) {
        return fail(format!("Codex 根会话 rollout 缺失: {root_id}"));
    }

    let mut children: HashMap<String, Vec<String>> = HashMap::new();
    let mut ordered: Vec<(&String, &String)> = parents.iter().collect();
    ordered.sort();
    for (child, parent) in ordered {
        children
            .entry(parent.clone())
            .or_default()
            .push(child.clone());
    }
    let mut reachable: HashSet<String> = HashSet::new();
    let mut stack = vec![root_id.clone()];
    while let Some(current) = stack.pop() {
        if reachable.contains(&current) {
            return fail("Codex 会话树存在环或重复父节点");
        }
        if !index.contains_key(&current) {
            return fail(format!("Codex 子会话 rollout 缺失: {current}"));
        }
        reachable.insert(current.clone());
        if let Some(kids) = children.get(&current) {
            stack.extend(kids.iter().cloned());
        }
    }

    let mut nodes = HashMap::new();
    let mut sorted: Vec<&String> = reachable.iter().collect();
    sorted.sort();
    for id in sorted {
        nodes.insert(id.clone(), rollout(&index[id].path)?);
    }
    let relevant_parents: HashMap<String, String> = parents
        .into_iter()
        .filter(|(child, _)| reachable.contains(child))
        .collect();

    let revision = closure_revision(&nodes);
    let ids: HashSet<String> = nodes.keys().cloned().collect();
    let registry_revision = registry_revision(store.state_db.as_ref(), &ids)?;
    Ok(CodexClosure {
        anchor_id: anchor.thread_id,
        root_id,
        nodes,
        parents: relevant_parents,
        store,
        revision,
        registry_revision,
        pruned_ids: HashSet::new(),
    })
}

/// 递归收集记录里出现的、属于 `known_ids` 的线程引用。
pub fn collect_thread_refs(
    value: &Value,
    known_ids: &HashSet<String>,
    key: Option<&str>,
) -> HashSet<String> {
    let mut refs = HashSet::new();
    match value {
        Value::Object(entries) => {
            for (child_key, child) in entries {
                refs.extend(collect_thread_refs(child, known_ids, Some(child_key)));
            }
        }
        Value::Array(items) => {
            for child in items {
                refs.extend(collect_thread_refs(child, known_ids, key));
            }
        }
        Value::String(item) => {
            let key = key.unwrap_or("");
            if THREAD_KEYS.contains(&key) && known_ids.contains(item) {
                refs.insert(item.clone());
            } else if JSON_STRING_KEYS.contains(&key) && item.starts_with(['[', '{']) {
                if let Ok(inner) = serde_json::from_str::<Value>(item) {
                    refs.extend(collect_thread_refs(&inner, known_ids, None));
                }
            }
        }
        _ => {}
    }
    refs
}

/// 被删除记录引用到的直接子树整体从闭包里摘掉，并刷新 revision。
pub fn prune_referenced_subtrees(
    closure: &mut CodexClosure,
    records: &[Value],
) -> CloneResult<HashSet<String>> {
    let direct: HashSet<String> = closure
        .parents
        .iter()
        .filter(|(_, parent)| **parent == closure.anchor_id)
        .map(|(child, _)| child.clone())
        .collect();
    let referenced = collect_thread_refs(&Value::Array(records.to_vec()), &direct, None);
    if referenced.is_empty() {
        return Ok(HashSet::new());
    }
    let mut children: HashMap<String, Vec<String>> = HashMap::new();
    for (child, parent) in &closure.parents {
        children
            .entry(parent.clone())
            .or_default()
            .push(child.clone());
    }
    let mut removed: HashSet<String> = HashSet::new();
    let mut stack: Vec<String> = referenced.into_iter().collect();
    while let Some(current) = stack.pop() {
        if !removed.insert(current.clone()) {
            continue;
        }
        if let Some(kids) = children.get(&current) {
            stack.extend(kids.iter().cloned());
        }
    }
    for id in &removed {
        closure.nodes.remove(id);
        closure.parents.remove(id);
    }
    closure.pruned_ids.extend(removed.iter().cloned());
    let ids: HashSet<String> = closure.nodes.keys().cloned().collect();
    closure.registry_revision = registry_revision(closure.store.state_db.as_ref(), &ids)?;
    closure.revision = closure_revision(&closure.nodes);
    Ok(removed)
}

/// `PRAGMA table_xinfo` 的可见列（隐藏列被排除）。
pub(super) fn table_columns(db: &Connection, table: &str) -> Vec<String> {
    let query = format!("PRAGMA table_xinfo(\"{table}\")");
    let Ok(mut statement) = db.prepare(&query) else {
        return Vec::new();
    };
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(1)?, row.get::<_, i64>(6)?))
    });
    let Ok(rows) = rows else {
        return Vec::new();
    };
    rows.filter_map(Result::ok)
        .filter(|(_, hidden)| *hidden == 0)
        .map(|(name, _)| name)
        .collect()
}

/// SQLite 一行的 Python 元组 `repr`，用于对齐 `sorted(rows, key=repr)`。
fn python_row_repr(row: &[Value]) -> String {
    let parts: Vec<String> = row
        .iter()
        .map(crate::adapters::shared::dialect::python_repr)
        .collect();
    if parts.len() == 1 {
        format!("({},)", parts[0])
    } else {
        format!("({})", parts.join(", "))
    }
}

fn cell_value(row: &rusqlite::Row<'_>, index: usize) -> Value {
    match row.get_ref(index) {
        Ok(rusqlite::types::ValueRef::Null) => Value::Null,
        Ok(rusqlite::types::ValueRef::Integer(value)) => Value::from(value),
        Ok(rusqlite::types::ValueRef::Real(value)) => {
            serde_json::Number::from_f64(value).map_or(Value::Null, Value::Number)
        }
        Ok(rusqlite::types::ValueRef::Text(bytes)) => {
            Value::from(String::from_utf8_lossy(bytes).into_owned())
        }
        // `default=str` 把 bytes 序列化成 Python 的 repr 形态。
        Ok(rusqlite::types::ValueRef::Blob(bytes)) => Value::from(format!("{bytes:?}")),
        Err(_) => Value::Null,
    }
}

/// 注册库中与该闭包相关的行的稳定指纹。
pub fn registry_revision(
    db_path: Option<&PathBuf>,
    ids: &HashSet<String>,
) -> CloneResult<Option<String>> {
    let Some(db_path) = db_path.filter(|path| path.exists()) else {
        return Ok(None);
    };
    if ids.is_empty() {
        return Ok(None);
    }
    let connection = open_readonly(db_path)?;
    // Python 用 `tuple(ids)`（set 迭代序），但 IN 子句与随后的 sorted 让结果稳定。
    let mut sorted_ids: Vec<&String> = ids.iter().collect();
    sorted_ids.sort();
    let placeholders = vec!["?"; sorted_ids.len()].join(",");
    let single: Vec<&dyn rusqlite::ToSql> = sorted_ids
        .iter()
        .map(|id| *id as &dyn rusqlite::ToSql)
        .collect();
    let doubled: Vec<&dyn rusqlite::ToSql> = single.iter().chain(single.iter()).copied().collect();

    let tables: [(&str, String, &Vec<&dyn rusqlite::ToSql>); 3] = [
        ("threads", format!("id IN ({placeholders})"), &single),
        (
            "thread_spawn_edges",
            format!("parent_thread_id IN ({placeholders}) OR child_thread_id IN ({placeholders})"),
            &doubled,
        ),
        (
            "thread_dynamic_tools",
            format!("thread_id IN ({placeholders})"),
            &single,
        ),
    ];

    // sort_keys=True：键按字典序插入即可等价。
    let mut snapshot: Vec<(String, Value)> = Vec::new();
    for (table, where_clause, args) in tables {
        let columns = table_columns(&connection, table);
        if columns.is_empty() {
            continue;
        }
        let query = format!("SELECT * FROM \"{table}\" WHERE {where_clause}");
        let mut statement = connection
            .prepare(&query)
            .map_err(|error| CodexCloneError(format!("读取 Codex 注册库失败: {error}")))?;
        let width = statement.column_count();
        let rows = statement
            .query_map(args.as_slice(), |row| {
                Ok((0..width).map(|index| cell_value(row, index)).collect())
            })
            .map_err(|error| CodexCloneError(format!("读取 Codex 注册库失败: {error}")))?;
        let mut collected: Vec<Vec<Value>> = Vec::new();
        for row in rows {
            collected.push(
                row.map_err(|error| CodexCloneError(format!("读取 Codex 注册库失败: {error}")))?,
            );
        }
        collected.sort_by_key(|row| python_row_repr(row));
        let mut entry = Map::new();
        entry.insert(
            "columns".into(),
            Value::Array(columns.into_iter().map(Value::from).collect()),
        );
        entry.insert(
            "rows".into(),
            Value::Array(collected.into_iter().map(Value::Array).collect()),
        );
        snapshot.push((table.to_string(), Value::Object(entry)));
    }
    snapshot.sort_by(|left, right| left.0.cmp(&right.0));
    let mut payload = Map::new();
    for (table, entry) in snapshot {
        payload.insert(table, entry);
    }
    let raw = python_json_dumps(&Value::Object(payload));
    Ok(Some(format!("sha256:{}", sha256_hex(raw.as_bytes()))))
}

/// 从 `~/.codex/.ferry/transactions/*.json` 做崩溃恢复。
pub fn recover_transactions(store: &CodexStore) {
    let directory = store.home.join(".ferry").join("transactions");
    if !directory.exists() {
        return;
    }
    let Ok(entries) = fs::read_dir(&directory) else {
        return;
    };
    let mut journals: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect();
    journals.sort();
    for journal in journals {
        // 无法证明归属时不做破坏性恢复，保留日志供人工检查。
        if recover_one(store, &journal).is_none() {
            continue;
        }
    }
}

fn recover_one(store: &CodexStore, journal: &Path) -> Option<()> {
    let data: Value = serde_json::from_str(&fs::read_to_string(journal).ok()?).ok()?;
    let ids: Vec<String> = data
        .get("ids")
        .and_then(Value::as_array)
        .map(|items| items.iter().map(text).collect())
        .unwrap_or_default();
    let paths: Vec<PathBuf> = data
        .get("paths")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|value| PathBuf::from(text(value)))
                .collect()
        })
        .unwrap_or_default();
    if let Some(state_db) = store.state_db.as_ref().filter(|path| path.exists()) {
        if !ids.is_empty() {
            let connection = Connection::open(state_db).ok()?;
            connection
                .busy_timeout(std::time::Duration::from_secs(5))
                .ok()?;
            let placeholders = vec!["?"; ids.len()].join(",");
            let single: Vec<&dyn rusqlite::ToSql> =
                ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();
            if !table_columns(&connection, "thread_spawn_edges").is_empty() {
                let doubled: Vec<&dyn rusqlite::ToSql> =
                    single.iter().chain(single.iter()).copied().collect();
                connection
                    .execute(
                        &format!(
                            "DELETE FROM thread_spawn_edges WHERE parent_thread_id IN \
                             ({placeholders}) OR child_thread_id IN ({placeholders})"
                        ),
                        doubled.as_slice(),
                    )
                    .ok()?;
            }
            if !table_columns(&connection, "threads").is_empty() {
                connection
                    .execute(
                        &format!("DELETE FROM threads WHERE id IN ({placeholders})"),
                        single.as_slice(),
                    )
                    .ok()?;
            }
        }
    }
    let sessions =
        fs::canonicalize(&store.sessions_dir).unwrap_or_else(|_| store.sessions_dir.clone());
    for path in paths {
        let Ok(resolved) = fs::canonicalize(&path) else {
            continue;
        };
        if !resolved
            .ancestors()
            .skip(1)
            .any(|parent| parent == sessions)
        {
            continue;
        }
        let Ok(node) = rollout_identity(&resolved) else {
            continue;
        };
        if ids.contains(&node.thread_id) {
            let _ = fs::remove_file(&resolved);
        }
    }
    if let Some(stage) = data.get("stage_dir").and_then(Value::as_str) {
        if !stage.is_empty() {
            let _ = fs::remove_dir_all(stage);
        }
    }
    let _ = fs::remove_file(journal);
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write_rollout(dir: &Path, name: &str, payload: Value, extra: &[Value]) -> PathBuf {
        let path = dir.join(name);
        let mut text = serde_json::to_string(&json!({
            "type": "session_meta", "payload": payload
        }))
        .unwrap();
        text.push('\n');
        for record in extra {
            text.push_str(&serde_json::to_string(record).unwrap());
            text.push('\n');
        }
        fs::write(&path, text).unwrap();
        path
    }

    fn sessions_dir(root: &Path) -> PathBuf {
        let dir = root.join(".codex").join("sessions").join("2026/07/25");
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn store_locates_the_sessions_ancestor_and_registry() {
        let temp = tempfile::tempdir().unwrap();
        let dir = sessions_dir(temp.path());
        let path = write_rollout(&dir, "rollout-a.jsonl", json!({"id": "a"}), &[]);
        let store = CodexStore::for_rollout(&path);
        assert!(store.sessions_dir.ends_with("sessions"));
        assert!(store.home.ends_with(".codex"));
        assert_eq!(store.state_db, None);

        fs::write(store.home.join("state_5.sqlite"), b"").unwrap();
        let with_db = CodexStore::for_rollout(&path);
        assert!(with_db.state_db.is_some());
    }

    #[test]
    fn closure_walks_parent_links_to_the_root() {
        let temp = tempfile::tempdir().unwrap();
        let dir = sessions_dir(temp.path());
        write_rollout(&dir, "rollout-root.jsonl", json!({"id": "root"}), &[]);
        let child = write_rollout(
            &dir,
            "rollout-child.jsonl",
            json!({"id": "child", "parent_thread_id": "root"}),
            &[],
        );
        let closure = discover_closure(&child, None).unwrap();
        assert_eq!(closure.anchor_id, "child");
        assert_eq!(closure.root_id, "root");
        assert_eq!(closure.nodes.len(), 2);
        assert!(closure.revision.starts_with("sha256:"));
        assert_eq!(closure.registry_revision, None);
    }

    #[test]
    fn parent_cycles_and_missing_roots_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let dir = sessions_dir(temp.path());
        let a = write_rollout(
            &dir,
            "rollout-a.jsonl",
            json!({"id": "a", "parent_thread_id": "b"}),
            &[],
        );
        write_rollout(
            &dir,
            "rollout-b.jsonl",
            json!({"id": "b", "parent_thread_id": "a"}),
            &[],
        );
        assert!(discover_closure(&a, None).unwrap_err().0.contains("父链环"));

        let temp = tempfile::tempdir().unwrap();
        let dir = sessions_dir(temp.path());
        let orphan = write_rollout(
            &dir,
            "rollout-orphan.jsonl",
            json!({"id": "orphan", "parent_thread_id": "gone"}),
            &[],
        );
        assert!(discover_closure(&orphan, None)
            .unwrap_err()
            .0
            .contains("根会话 rollout 缺失"));
    }

    #[test]
    fn sqlite_edges_cross_check_the_declared_parent() {
        let temp = tempfile::tempdir().unwrap();
        let dir = sessions_dir(temp.path());
        write_rollout(&dir, "rollout-root.jsonl", json!({"id": "root"}), &[]);
        write_rollout(&dir, "rollout-other.jsonl", json!({"id": "other"}), &[]);
        let child = write_rollout(
            &dir,
            "rollout-child.jsonl",
            json!({"id": "child", "parent_thread_id": "root"}),
            &[],
        );
        let store = CodexStore::for_rollout(&child);
        let db_path = store.home.join("state_5.sqlite");
        let db = Connection::open(&db_path).unwrap();
        db.execute_batch(
            "CREATE TABLE thread_spawn_edges (parent_thread_id TEXT, \
             child_thread_id TEXT PRIMARY KEY, status TEXT);
             INSERT INTO thread_spawn_edges VALUES ('other', 'child', 'closed');",
        )
        .unwrap();
        drop(db);
        let error = discover_closure(&child, None).unwrap_err();
        assert!(error.0.contains("与 SQLite 父关系冲突"));
    }

    #[test]
    fn sqlite_edges_supply_missing_parents_and_a_registry_revision() {
        let temp = tempfile::tempdir().unwrap();
        let dir = sessions_dir(temp.path());
        write_rollout(&dir, "rollout-root.jsonl", json!({"id": "root"}), &[]);
        let child = write_rollout(&dir, "rollout-child.jsonl", json!({"id": "child"}), &[]);
        let store = CodexStore::for_rollout(&child);
        let db_path = store.home.join("state_5.sqlite");
        let db = Connection::open(&db_path).unwrap();
        db.execute_batch(
            "CREATE TABLE threads (id TEXT PRIMARY KEY, title TEXT);
             CREATE TABLE thread_spawn_edges (parent_thread_id TEXT, \
             child_thread_id TEXT PRIMARY KEY, status TEXT);
             INSERT INTO threads VALUES ('root', 'r'), ('child', 'c');
             INSERT INTO thread_spawn_edges VALUES ('root', 'child', 'closed');",
        )
        .unwrap();
        drop(db);
        let closure = discover_closure(&child, None).unwrap();
        assert_eq!(closure.root_id, "root");
        assert_eq!(
            closure.parents.get("child").map(String::as_str),
            Some("root")
        );
        let revision = closure.registry_revision.clone().unwrap();
        assert!(revision.starts_with("sha256:"));
        // 指纹是内容确定的：重新发现同一闭包必须给出同样的值。
        let again = discover_closure(&child, None).unwrap();
        assert_eq!(again.registry_revision, Some(revision));
    }

    #[test]
    fn thread_references_are_found_through_embedded_json() {
        let known: HashSet<String> = ["c1".to_string()].into_iter().collect();
        let records = json!([
            {"payload": {"arguments": "{\"agent_thread_id\": \"c1\"}"}},
        ]);
        assert_eq!(collect_thread_refs(&records, &known, None), known.clone());
        // 非 THREAD_KEYS 的键即使值相同也不算引用。
        let unrelated = json!([{"payload": {"note": "c1"}}]);
        assert!(collect_thread_refs(&unrelated, &known, None).is_empty());
    }

    #[test]
    fn pruning_removes_whole_referenced_subtrees() {
        let temp = tempfile::tempdir().unwrap();
        let dir = sessions_dir(temp.path());
        let root = write_rollout(&dir, "rollout-root.jsonl", json!({"id": "root"}), &[]);
        write_rollout(
            &dir,
            "rollout-c1.jsonl",
            json!({"id": "c1", "parent_thread_id": "root"}),
            &[],
        );
        write_rollout(
            &dir,
            "rollout-c2.jsonl",
            json!({"id": "c2", "parent_thread_id": "c1"}),
            &[],
        );
        let mut closure = discover_closure(&root, None).unwrap();
        let before = closure.revision.clone();
        let removed = prune_referenced_subtrees(
            &mut closure,
            &[json!({"payload": {"agent_thread_id": "c1"}})],
        )
        .unwrap();
        assert_eq!(
            removed,
            ["c1".to_string(), "c2".to_string()].into_iter().collect()
        );
        assert_eq!(closure.nodes.len(), 1);
        assert_ne!(closure.revision, before);
        assert_eq!(closure.pruned_ids.len(), 2);
    }

    #[test]
    fn transaction_recovery_removes_owned_artifacts() {
        let temp = tempfile::tempdir().unwrap();
        let dir = sessions_dir(temp.path());
        let victim = write_rollout(&dir, "rollout-tmp.jsonl", json!({"id": "tmp"}), &[]);
        let store = CodexStore::for_rollout(&victim);
        let stage = store.home.join("stage");
        fs::create_dir_all(&stage).unwrap();
        let journal_dir = store.home.join(".ferry").join("transactions");
        fs::create_dir_all(&journal_dir).unwrap();
        fs::write(
            journal_dir.join("t1.json"),
            serde_json::to_string(&json!({
                "ids": ["tmp"],
                "paths": [victim.to_string_lossy()],
                "stage_dir": stage.to_string_lossy(),
            }))
            .unwrap(),
        )
        .unwrap();
        recover_transactions(&store);
        assert!(!victim.exists());
        assert!(!stage.exists());
        assert!(!journal_dir.join("t1.json").exists());
    }
}
