//! Codex rollout 索引与子 Agent 拓扑恢复。
//!
//! 语义事实源：`engine/adapters/codex/topology.py`。

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::adapters::shared::scanner::iter_lines;
use crate::adapters::shared::writing::python_json_dumps;
use crate::errors::{DomainError, DomainResult};
use crate::jsonutil::FileStat;
use crate::model::{tool_result_text, AgentEdge, BlockKind, Session, ToolCall};
use crate::system::paths::home_dir;
use crate::tool_ops::CanonicalOp;

/// rollout 身份的磁盘缓存；与 Python 的 `ScanCache(version=2)` 格式互认。
fn meta_cache_path() -> PathBuf {
    home_dir().join(".ferry").join("rollout-meta-cache.json")
}

const META_CACHE_VERSION: i64 = 2;

/// 一条 rollout 的线程身份。
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Identity {
    pub id: String,
    pub root_id: String,
    pub parent_id: Option<String>,
    pub forked_from_id: Option<String>,
    pub agent_id: Option<String>,
    pub agent_path: Option<String>,
    pub agent_type: Option<String>,
    pub agent_nickname: Option<String>,
    pub agent_role: Option<String>,
    pub model_provider: Option<String>,
    pub model: Option<String>,
    pub depth: Option<i64>,
}

/// `session_id(meta, fallback)`：Codex 的会话标识只认 `payload.id`。
pub fn session_id(meta: &Map<String, Value>) -> Option<String> {
    meta.get("id").map(python_text)
}

fn python_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => crate::adapters::shared::dialect::python_str(other),
    }
}

fn object_of(value: Option<&Value>) -> Option<&Map<String, Value>> {
    value.and_then(Value::as_object)
}

fn truthy_str(value: Option<&Value>) -> Option<String> {
    let value = value?;
    match value {
        Value::Null => None,
        Value::String(text) if text.is_empty() => None,
        Value::Bool(false) => None,
        other => Some(python_text(other)),
    }
}

/// 从 session_meta 载荷解析线程身份；缺少 `id` 时返回 `None`。
pub fn identity(meta: &Map<String, Value>) -> Option<Identity> {
    let empty = Map::new();
    let source = object_of(meta.get("source")).unwrap_or(&empty);
    let subagent = object_of(source.get("subagent")).unwrap_or(&empty);
    let spawn = object_of(subagent.get("thread_spawn")).unwrap_or(&empty);

    let current_id = session_id(meta)?;
    let root_id = truthy_str(meta.get("session_id"))
        .or_else(|| truthy_str(spawn.get("session_id")))
        .unwrap_or_else(|| current_id.clone());
    let parent_id = truthy_str(meta.get("parent_thread_id"))
        .or_else(|| truthy_str(spawn.get("parent_thread_id")))
        .or_else(|| truthy_str(subagent.get("parent_thread_id")));
    Some(Identity {
        id: current_id,
        root_id,
        forked_from_id: truthy_str(meta.get("forked_from_id"))
            .or_else(|| truthy_str(spawn.get("forked_from_id")))
            .or_else(|| parent_id.clone()),
        agent_id: truthy_str(subagent.get("agent_id"))
            .or_else(|| truthy_str(spawn.get("agent_id")))
            .or_else(|| truthy_str(meta.get("agent_id"))),
        agent_path: truthy_str(subagent.get("agent_path"))
            .or_else(|| truthy_str(spawn.get("agent_path")))
            .or_else(|| truthy_str(meta.get("agent_path"))),
        agent_type: truthy_str(subagent.get("agent_type"))
            .or_else(|| truthy_str(spawn.get("agent_type")))
            .or_else(|| truthy_str(meta.get("agent_type"))),
        agent_nickname: truthy_str(spawn.get("agent_nickname"))
            .or_else(|| truthy_str(meta.get("agent_nickname"))),
        agent_role: truthy_str(spawn.get("agent_role"))
            .or_else(|| truthy_str(meta.get("agent_role"))),
        model_provider: truthy_str(meta.get("model_provider")),
        model: truthy_str(meta.get("model")),
        // `subagent.get("depth", spawn.get("depth"))`：subagent 里有键就用它。
        depth: match subagent.get("depth") {
            Some(value) => value.as_i64(),
            None => spawn.get("depth").and_then(Value::as_i64),
        },
        parent_id,
    })
}

/// 只读到第一条 `session_meta` 的 payload。
fn first_meta(path: &Path) -> Map<String, Value> {
    let Ok(lines) = iter_lines(path) else {
        return Map::new();
    };
    for line in lines {
        let Ok(line) = line else {
            return Map::new();
        };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Value>(&line) else {
            // Python 的 `except json.JSONDecodeError` 包住整个循环：一旦解析失败
            // 就放弃整份文件。
            return Map::new();
        };
        if record.get("type").and_then(Value::as_str) == Some("session_meta") {
            return object_of(record.get("payload"))
                .cloned()
                .unwrap_or_default();
        }
    }
    Map::new()
}

/// 向上找名为 `sessions` 的祖先目录；找不到就取父目录。
pub fn sessions_root(path: &Path) -> PathBuf {
    let mut current = path.parent();
    while let Some(candidate) = current {
        if candidate.file_name().and_then(|name| name.to_str()) == Some("sessions") {
            return candidate.to_path_buf();
        }
        current = candidate.parent();
    }
    path.parent().unwrap_or(path).to_path_buf()
}

/// 递归收集 `rollout*.jsonl`；结果排序，保证同一目录树的遍历顺序稳定。
pub(super) fn rollout_files(root: &Path) -> Vec<PathBuf> {
    let mut hits: Vec<PathBuf> = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("rollout") && name.ends_with(".jsonl"))
        })
        .collect();
    hits.sort();
    hits
}

// ---------------------------------------------------------------------------
// 身份缓存（~/.ferry/rollout-meta-cache.json）
// ---------------------------------------------------------------------------

struct MetaCache {
    path: PathBuf,
    data: Map<String, Value>,
    dirty: bool,
}

impl MetaCache {
    fn load(path: PathBuf) -> Self {
        let data = fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        Self {
            path,
            data,
            dirty: false,
        }
    }

    /// 外层 `None` = 未命中；内层空表 = 「已知不是 rollout」。
    fn get(&self, key: &str, stat: &FileStat) -> Option<Map<String, Value>> {
        let hit = self.data.get(key)?.as_object()?;
        let matches = hit.get("version").and_then(Value::as_i64) == Some(META_CACHE_VERSION)
            && hit.get("mtime").and_then(Value::as_i64) == Some(stat.mtime_ns as i64)
            && hit.get("size").and_then(Value::as_i64) == Some(stat.size as i64);
        if !matches {
            return None;
        }
        Some(
            hit.get("meta")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default(),
        )
    }

    fn put(&mut self, key: &str, stat: &FileStat, meta: Map<String, Value>) {
        let mut entry = Map::new();
        entry.insert("version".into(), Value::from(META_CACHE_VERSION));
        entry.insert("mtime".into(), Value::from(stat.mtime_ns as i64));
        entry.insert("size".into(), Value::from(stat.size as i64));
        entry.insert("meta".into(), Value::Object(meta));
        self.data.insert(key.to_string(), Value::Object(entry));
        self.dirty = true;
    }

    /// 读回磁盘最新 → 合并本实例增量 → 原子写回；失败静默（对齐 `except OSError`）。
    fn flush(&self) {
        if !self.dirty {
            return;
        }
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let mut merged = fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        for (key, value) in &self.data {
            let newer = match merged.get(key).and_then(Value::as_object) {
                Some(current) => {
                    let mtime = |entry: &Map<String, Value>| {
                        entry.get("mtime").and_then(Value::as_i64).unwrap_or(-1)
                    };
                    value.as_object().map(mtime).unwrap_or(-1) >= mtime(current)
                }
                None => true,
            };
            if newer {
                merged.insert(key.clone(), value.clone());
            }
        }
        let temporary = self.path.with_file_name(format!(
            "{}.{}.tmp",
            self.path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            std::process::id()
        ));
        if fs::write(
            &temporary,
            serde_json::to_string(&Value::Object(merged)).unwrap_or_default(),
        )
        .is_ok()
        {
            let _ = fs::rename(&temporary, &self.path);
        }
    }
}

/// 保序的 `thread_id → (rollout 路径, 身份)` 索引。
#[derive(Default)]
pub struct RolloutIndex {
    entries: Vec<(String, PathBuf, Identity)>,
    positions: HashMap<String, usize>,
}

impl RolloutIndex {
    fn insert(&mut self, path: PathBuf, ident: Identity) {
        match self.positions.get(&ident.id) {
            // Python 的 dict 赋值：覆盖值但保留首次出现的位置。
            Some(position) => self.entries[*position] = (ident.id.clone(), path, ident),
            None => {
                self.positions.insert(ident.id.clone(), self.entries.len());
                self.entries.push((ident.id.clone(), path, ident));
            }
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &(String, PathBuf, Identity)> {
        self.entries.iter()
    }
}

/// 扫描整个 sessions 目录，建立 `thread_id → (路径, 身份)` 索引。
pub fn rollout_index(path: &Path, sessions_dir: Option<&Path>) -> RolloutIndex {
    let root = match sessions_dir {
        Some(dir) => dir.to_path_buf(),
        None => sessions_root(path),
    };
    let mut candidates = if root.exists() {
        rollout_files(&root)
    } else {
        Vec::new()
    };
    if !candidates.iter().any(|candidate| candidate == path) {
        candidates.push(path.to_path_buf());
    }
    let mut cache = MetaCache::load(meta_cache_path());
    let mut index = RolloutIndex::default();
    for candidate in candidates {
        let Ok(metadata) = fs::metadata(&candidate) else {
            continue;
        };
        let stat = FileStat::from_metadata(&metadata);
        let key = candidate.to_string_lossy().into_owned();
        let cached = cache.get(&key, &stat);
        let ident = match cached {
            Some(entry) => entry,
            None => {
                let meta = first_meta(&candidate);
                let ident = if meta.is_empty() {
                    Map::new()
                } else {
                    identity(&meta)
                        .and_then(|ident| serde_json::to_value(ident).ok())
                        .and_then(|value| value.as_object().cloned())
                        .unwrap_or_default()
                };
                cache.put(&key, &stat, ident.clone());
                ident
            }
        };
        if ident.is_empty() {
            continue;
        }
        if let Ok(ident) = serde_json::from_value::<Identity>(Value::Object(ident)) {
            index.insert(candidate, ident);
        }
    }
    cache.flush();
    index
}

// ---------------------------------------------------------------------------
// 子 Agent 树装配
// ---------------------------------------------------------------------------

fn spawn_calls(session: &Session) -> Vec<ToolCall> {
    session
        .messages
        .iter()
        .flat_map(|message| message.blocks.iter())
        .filter(|block| block.kind == BlockKind::Tool)
        .filter_map(|block| block.tool.as_ref())
        .filter(|tool| tool.op.as_deref() == Some(CanonicalOp::AGENT_SPAWN))
        .cloned()
        .collect()
}

/// spawn 调用的入参/结果里是否提到该子会话的任一身份串。
fn contains_identity(tool: &ToolCall, child: &Session) -> bool {
    let mut payload = Map::new();
    payload.insert("input".into(), tool.input.clone());
    payload.insert(
        "output".into(),
        Value::from(tool_result_text(tool.result.as_ref())),
    );
    let haystack = python_json_dumps(&Value::Object(payload));
    [
        Some(child.source_id.as_str()),
        child.agent_id.as_deref(),
        child.agent_path.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|value| !value.is_empty() && haystack.contains(value))
}

fn canonical_edge_status(value: Option<&str>) -> Option<String> {
    match value? {
        status @ ("open" | "closed") => Some(status.to_string()),
        "completed" | "failed" | "cancelled" | "canceled" => Some("closed".to_string()),
        "in_progress" | "queued" => Some("open".to_string()),
        _ => None,
    }
}

fn attach_tree(
    session: &mut Session,
    by_parent: &HashMap<String, Vec<String>>,
    sessions: &mut HashMap<String, Session>,
    seen: &mut HashSet<String>,
    edge_statuses: &HashMap<String, String>,
) {
    if !seen.insert(session.source_id.clone()) {
        return;
    }
    let calls = spawn_calls(session);
    let candidates = by_parent
        .get(&session.source_id)
        .cloned()
        .unwrap_or_default();

    // 第一遍：按 spawn 调用顺序挑出能对上号的子会话，其余保持原顺序追加。
    let mut ordered: Vec<String> = Vec::with_capacity(candidates.len());
    let mut selected: HashSet<String> = HashSet::new();
    for tool in &calls {
        let hit = candidates.iter().find(|id| {
            !selected.contains(*id)
                && sessions
                    .get(*id)
                    .is_some_and(|child| contains_identity(tool, child))
        });
        if let Some(id) = hit.cloned() {
            selected.insert(id.clone());
            ordered.push(id);
        }
    }
    ordered.extend(
        candidates
            .iter()
            .filter(|id| !selected.contains(*id))
            .cloned(),
    );

    let mut used_calls: HashSet<usize> = HashSet::new();
    for child_id in ordered {
        if seen.contains(&child_id) {
            continue;
        }
        let Some(mut child) = sessions.remove(&child_id) else {
            continue;
        };
        let matched = calls
            .iter()
            .enumerate()
            .find(|(index, tool)| !used_calls.contains(index) && contains_identity(tool, &child))
            .map(|(index, tool)| {
                used_calls.insert(index);
                tool
            });
        if matched.is_none() && !calls.is_empty() {
            let mut params = Map::new();
            params.insert("child_id".into(), Value::from(child.source_id.as_str()));
            session.lose("session.subagent_unlinked", params);
        }
        let prompt = matched
            .and_then(|tool| tool.input.get("prompt"))
            .map(python_text)
            .unwrap_or_default();
        let mut edge = AgentEdge::new(session.source_id.as_str(), child.source_id.as_str());
        edge.source_call_id = matched.and_then(|tool| tool.source_call_id.clone());
        edge.spawn_message_id = matched.and_then(|tool| tool.source_message_id.clone());
        edge.result_message_id = matched.and_then(|tool| tool.source_result_id.clone());
        edge.agent_id = child.agent_id.clone();
        edge.agent_path = child.agent_path.clone();
        edge.agent_type = child.agent_type.clone();
        edge.prompt = prompt;
        edge.status = matched
            .and_then(|tool| tool.result.as_ref())
            .and_then(|result| canonical_edge_status(Some(status_name(result.status))))
            .or_else(|| edge_statuses.get(&child.source_id).cloned());
        edge.association = if matched.is_some() {
            "spawn-call".to_string()
        } else {
            child
                .parent_association
                .clone()
                .unwrap_or_else(|| "parent-metadata".to_string())
        };
        edge.confidence = if matched.is_some() {
            1.0
        } else if child.parent_association.as_deref() == Some("sqlite-parent") {
            0.95
        } else {
            0.75
        };
        attach_tree(&mut child, by_parent, sessions, seen, edge_statuses);
        session.children.push(child);
        session.agent_edges.push(edge);
    }
}

fn status_name(status: crate::model::ToolResultStatus) -> &'static str {
    use crate::model::ToolResultStatus as Status;
    match status {
        Status::Success => "success",
        Status::Error => "error",
        Status::Interrupted => "interrupted",
        Status::Running => "running",
        Status::Pending => "pending",
        Status::Unknown => "unknown",
    }
}

/// `state_5.sqlite` 的 `thread_spawn_edges`：`child → (parent, status)`。
fn registry_edges(root: &Path) -> HashMap<String, (String, String)> {
    let Some(parent) = root.parent() else {
        return HashMap::new();
    };
    let db_path = parent.join("state_5.sqlite");
    if !db_path.exists() {
        return HashMap::new();
    }
    let Ok(resolved) = fs::canonicalize(&db_path) else {
        return HashMap::new();
    };
    let uri = format!("file:{}?mode=ro", resolved.display());
    let Ok(connection) = rusqlite::Connection::open_with_flags(
        &uri,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    ) else {
        return HashMap::new();
    };
    let Ok(mut statement) = connection
        .prepare("SELECT parent_thread_id, child_thread_id, status FROM thread_spawn_edges")
    else {
        return HashMap::new();
    };
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    });
    let Ok(rows) = rows else {
        return HashMap::new();
    };
    rows.filter_map(Result::ok)
        .map(|(parent, child, status)| (child, (parent, status)))
        .collect()
}

/// 读一条 rollout 并递归加载同根的全部后代。
pub fn read_tree(
    rollout: &Path,
    read_one: &dyn Fn(&Path) -> DomainResult<Session>,
    sessions_dir: Option<&Path>,
) -> DomainResult<Session> {
    let index = rollout_index(rollout, sessions_dir);
    let mut root = read_one(rollout)?;
    let edges = registry_edges(&sessions_root(rollout));

    let mut sessions: HashMap<String, Session> = HashMap::new();
    let mut reachable: HashSet<String> = HashSet::new();
    reachable.insert(root.source_id.clone());
    loop {
        let mut added = false;
        for (current_id, candidate, ident) in index.iter() {
            let registry_parent = edges.get(current_id).map(|(parent, _)| parent.clone());
            let parent_id = ident.parent_id.clone().or_else(|| registry_parent.clone());
            let known = parent_id
                .as_ref()
                .is_some_and(|parent| reachable.contains(parent));
            if reachable.contains(current_id) || !known {
                continue;
            }
            reachable.insert(current_id.clone());
            let mut child = read_one(candidate)?;
            if child.parent_id.is_none() {
                if let Some(parent) = registry_parent {
                    child.parent_id = Some(parent);
                    child.parent_association = Some("sqlite-parent".to_string());
                }
            }
            sessions.insert(current_id.clone(), child);
            added = true;
        }
        if !added {
            break;
        }
    }

    let mut by_parent: HashMap<String, Vec<String>> = HashMap::new();
    let mut sorted: Vec<(&String, &Session)> = sessions.iter().collect();
    sorted.sort_by(|left, right| {
        let key = |session: &Session| {
            (
                session.agent_path.clone().unwrap_or_default(),
                session.source_id.clone(),
            )
        };
        key(left.1).cmp(&key(right.1))
    });
    for (id, candidate) in sorted {
        if let Some(parent) = candidate.parent_id.as_ref().filter(|id| !id.is_empty()) {
            by_parent
                .entry(parent.clone())
                .or_default()
                .push(id.clone());
        }
    }

    let edge_statuses: HashMap<String, String> = edges
        .iter()
        .map(|(child, (_parent, status))| (child.clone(), status.clone()))
        .collect();
    attach_tree(
        &mut root,
        &by_parent,
        &mut sessions,
        &mut HashSet::new(),
        &edge_statuses,
    );
    Ok(root)
}

/// 缺少 `session_meta.id` 时的统一错误（对齐 Python 的 KeyError 崩溃语义）。
pub fn missing_identity_error(path: &Path) -> DomainError {
    DomainError::internal(format!(
        "Codex rollout 缺少 session_meta.id: {}",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap()
    }

    #[test]
    fn identity_prefers_the_deepest_subagent_metadata() {
        let ident = identity(&map(json!({
            "id": "child",
            "session_id": "root",
            "source": {"subagent": {
                "agent_id": "a1",
                "depth": 2,
                "thread_spawn": {
                    "parent_thread_id": "parent",
                    "agent_path": "/root/docs",
                    "agent_nickname": "docs",
                    "agent_role": "writer",
                },
            }},
        })))
        .unwrap();
        assert_eq!(ident.id, "child");
        assert_eq!(ident.root_id, "root");
        assert_eq!(ident.parent_id.as_deref(), Some("parent"));
        assert_eq!(ident.forked_from_id.as_deref(), Some("parent"));
        assert_eq!(ident.agent_id.as_deref(), Some("a1"));
        assert_eq!(ident.agent_path.as_deref(), Some("/root/docs"));
        assert_eq!(ident.agent_nickname.as_deref(), Some("docs"));
        assert_eq!(ident.agent_role.as_deref(), Some("writer"));
        assert_eq!(ident.depth, Some(2));
    }

    #[test]
    fn identity_falls_back_to_its_own_id_as_root() {
        let ident = identity(&map(json!({"id": "solo"}))).unwrap();
        assert_eq!(ident.root_id, "solo");
        assert_eq!(ident.parent_id, None);
        assert_eq!(ident.forked_from_id, None);
        // 非法结构（source 不是对象）不能让解析崩掉。
        let odd = identity(&map(json!({"id": "solo", "source": "cli"}))).unwrap();
        assert_eq!(odd.root_id, "solo");
        assert!(identity(&Map::new()).is_none());
    }

    #[test]
    fn sessions_root_walks_up_to_the_sessions_directory() {
        let path = Path::new("/home/u/.codex/sessions/2026/07/25/rollout-a.jsonl");
        assert_eq!(
            sessions_root(path),
            Path::new("/home/u/.codex/sessions").to_path_buf()
        );
        // 没有 sessions 祖先时退回父目录。
        assert_eq!(
            sessions_root(Path::new("/tmp/rollout-a.jsonl")),
            Path::new("/tmp").to_path_buf()
        );
    }

    #[test]
    fn edge_status_normalizes_the_native_vocabulary() {
        assert_eq!(canonical_edge_status(Some("open")).as_deref(), Some("open"));
        assert_eq!(
            canonical_edge_status(Some("completed")).as_deref(),
            Some("closed")
        );
        assert_eq!(
            canonical_edge_status(Some("queued")).as_deref(),
            Some("open")
        );
        assert_eq!(canonical_edge_status(Some("unknown")), None);
        assert_eq!(canonical_edge_status(None), None);
    }

    #[test]
    fn identity_matching_scans_both_input_and_output() {
        let mut child = Session::new("codex", "child-1", "/tmp");
        child.agent_path = Some("/root/docs".into());
        let mut tool = ToolCall::new(
            "spawn_agent",
            Some(CanonicalOp::AGENT_SPAWN.to_string()),
            json!({"prompt": "spawn child-1"}),
        );
        assert!(contains_identity(&tool, &child));
        tool.input = json!({"prompt": "unrelated"});
        assert!(!contains_identity(&tool, &child));
        tool.result = Some(crate::model::text_tool_result(
            "started /root/docs",
            crate::model::ToolResultStatus::Success,
        ));
        assert!(contains_identity(&tool, &child));
    }
}
