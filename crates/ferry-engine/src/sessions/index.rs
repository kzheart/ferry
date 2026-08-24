//! Opaque 会话引用、消息 locator 与 revision 索引。
//!
//! 三条不可动摇的语义：
//! 1. `fsr_` 是**随机签发**的稳定句柄（不从内容派生），按 `(tool, canonical_ref)`
//!    墓碑复用；内容变化只改 revision，不换发 ref。
//! 2. revision 是 `{tool,ref,updated,size,file_identity}` 的
//!    **非 ASCII 转义成 `\uXXXX` 的紧凑 JSON** 的 sha256——与
//!    `jsonutil::canonical_json`（非 ASCII 原样输出）不是同一套，非 ASCII 路径
//!    上两者会分叉，故本模块自带 [`stable_json`]。
//! 3. `sessions.changed` 的 generation 严格 +1 且在索引锁内推送；bootstrap
//!    首扫不推增量。

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, LazyLock, Mutex, MutexGuard};

use base64::Engine as _;
use rand::TryRngCore as _;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::adapters::contracts::{
    AgentAdapter, Fingerprint, ScanCache, ScanRow, SessionBrowser, StorageKind,
};
use crate::contracts::session_ref::is_opaque_session_ref;
use crate::errors::{DomainError, DomainResult};
use crate::jsonutil::FileStat;
use crate::system::git;
use crate::system::paths::{is_within, realpath_strict};

use super::scan_progress::TRACKER;

/// ref 是稳定的会话句柄，但钉内容的读取/编辑路径在会话被写入后需要重扫。
/// agent 只看得到结构化错误的 params，所以恢复办法必须以数据形式给出。
///
/// 措辞对调用方中立：不点名任何 RPC 方法。`session_search` 只对 ui caller 开放，
/// 照着方法名重试的 CLI 会撞 `caller_not_allowed`——恢复提示不能把人引到死路。
/// 测试逐字断言。
pub const REF_RECOVERY_HINT: &str = "the session changed since the last scan; run a session search \
                                     again to re-index it, then retry with the ref from the results";

const DIGEST_CACHE_LIMIT: usize = 50_000;
const PARALLEL_CANONICALIZE_THRESHOLD: usize = 64;

fn canonicalize_workers() -> usize {
    8.min(
        std::thread::available_parallelism()
            .map(|value| value.get())
            .unwrap_or(4),
    )
}

static CANONICALIZE_POOL: LazyLock<rayon::ThreadPool> = LazyLock::new(|| {
    rayon::ThreadPoolBuilder::new()
        .num_threads(canonicalize_workers())
        .thread_name(|index| format!("canonicalize-{index}"))
        .build()
        .expect("规范化线程池必须可创建")
});

/// 能力包共享的运行端口（Python `EngineContext` 的 sessions 子集）。
///
/// WP-E 组合根提供实现；sessions 只依赖这个 trait，不认识 registry。
pub trait SessionPorts: Send + Sync {
    fn adapter(&self, name: &str) -> DomainResult<&AgentAdapter>;
    /// 装配顺序（对齐 Python `ports.adapters()` 的元组序）。
    fn adapters(&self) -> Vec<String>;
    fn cache_factory(&self) -> Arc<dyn ScanCache>;
}

// ---------- revision ----------

/// 与 `json.dumps(obj, sort_keys=True, separators=(",", ":"))` 逐字节一致。
///
/// **ensure_ascii 保持 Python 默认的 True**：非 ASCII 一律转成 `\uXXXX`
/// （astral 平面转代理对）。revision 的输入含路径与目录成员名，这一位错了
/// 全部中文路径的 revision 都对不上。
pub fn stable_json(value: &Value) -> String {
    let mut out = String::new();
    write_stable(&mut out, value);
    out
}

fn write_stable(out: &mut String, value: &Value) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(number) => out.push_str(&number.to_string()),
        Value::String(text) => write_ascii_string(out, text),
        Value::Array(items) => {
            out.push('[');
            for (position, item) in items.iter().enumerate() {
                if position > 0 {
                    out.push(',');
                }
                write_stable(out, item);
            }
            out.push(']');
        }
        Value::Object(entries) => {
            let mut keys: Vec<&String> = entries.keys().collect();
            keys.sort_unstable();
            out.push('{');
            for (position, key) in keys.into_iter().enumerate() {
                if position > 0 {
                    out.push(',');
                }
                write_ascii_string(out, key);
                out.push(':');
                write_stable(out, &entries[key]);
            }
            out.push('}');
        }
    }
}

fn write_ascii_string(out: &mut String, text: &str) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other if !(' '..='\u{7e}').contains(&other) => {
                let code = other as u32;
                if code > 0xFFFF {
                    let adjusted = code - 0x1_0000;
                    let _ = write!(
                        out,
                        "\\u{:04x}\\u{:04x}",
                        0xD800 + (adjusted >> 10),
                        0xDC00 + (adjusted & 0x3FF)
                    );
                } else {
                    let _ = write!(out, "\\u{code:04x}");
                }
            }
            other => out.push(other),
        }
    }
    out.push('"');
}

/// `_revision`：`{tool,ref,updated,size,file_identity}` 的 sha256（小写 hex）。
pub fn revision(tool: &str, canonical_ref: &str, row: &ScanRow, identity: &Value) -> String {
    let mut payload = Map::new();
    payload.insert("tool".into(), Value::from(tool));
    payload.insert("ref".into(), Value::from(canonical_ref));
    payload.insert(
        "updated".into(),
        row.get("updated").cloned().unwrap_or(Value::Null),
    );
    payload.insert(
        "size".into(),
        row.get("size").cloned().unwrap_or(Value::Null),
    );
    payload.insert("file_identity".into(), identity.clone());
    let stable = stable_json(&Value::Object(payload));
    let mut out = String::with_capacity(64);
    for byte in Sha256::digest(stable.as_bytes()) {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// `"fsr_"` / `"fml_"` + `secrets.token_urlsafe(18)`：18 字节 CSPRNG，
/// URL-safe base64 无填充 ⇒ 恒为 24 字符。
fn issue_token(prefix: &str) -> String {
    let mut bytes = [0u8; 18];
    rand::rngs::OsRng
        .try_fill_bytes(&mut bytes)
        .expect("CSPRNG 不可用");
    format!(
        "{prefix}{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    )
}

// ---------- 记录 ----------

#[derive(Clone, Debug, PartialEq)]
pub struct IndexedSession {
    pub opaque_ref: String,
    pub tool: String,
    pub canonical_ref: String,
    pub root: Option<String>,
    pub storage_kind: StorageKind,
    pub row: ScanRow,
    pub revision: String,
    /// 三形态：file = `[[dev,ino,mtime_ns,size,"hex"], fingerprint]`；
    /// directory = `[["rel", [dev,ino,mtime_ns,size,"hex"]], ...]`（按 rel 排序）；
    /// id = fingerprint 原值。
    pub source_identity: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IndexedMessage {
    pub opaque_locator: String,
    pub session_ref: String,
    pub tool: String,
    pub revision: String,
    pub native_locator: String,
    pub role: String,
    pub editable: bool,
}

/// 会话行的 UI 出参：原始行 + opaque ref 与内容 revision。
///
/// 若扫描行未带 `branch`，按 `dir` 现读一次 `.git/HEAD`（进程内按目录缓存）。
pub fn session_dto(record: &IndexedSession) -> Map<String, Value> {
    let mut payload = record.row.clone();
    payload.insert("ref".into(), Value::from(record.opaque_ref.as_str()));
    payload.insert("revision".into(), Value::from(record.revision.as_str()));
    let has_branch = payload
        .get("branch")
        .and_then(Value::as_str)
        .is_some_and(|branch| !branch.is_empty());
    if !has_branch {
        if let Some(dir) = payload.get("dir").and_then(Value::as_str) {
            if let Some(branch) = git::branch_of(dir) {
                payload.insert("branch".into(), Value::from(branch));
            }
        }
    }
    payload
}

// ---------- 身份计算 ----------

type StatKey = (u64, u64, i64, u64);
type DigestCache = HashMap<StatKey, String>;

/// 身份计算的失败分类。
enum IdentityError {
    /// 哈希会话内容时文件被追加写入：文件仍在，只是这一刻拍不出稳定快照。
    Race,
    /// OSError / ValueError / AgentReferenceError：这一行本轮不入索引。
    Unavailable,
}

fn file_stat(path: &Path) -> Result<FileStat, IdentityError> {
    std::fs::metadata(path)
        .map(|metadata| FileStat::from_metadata(&metadata))
        .map_err(|_| IdentityError::Unavailable)
}

fn stat_key(stat: &FileStat) -> StatKey {
    (stat.dev, stat.ino, stat.mtime_ns as i64, stat.size)
}

fn identity_value(key: StatKey, digest: &str) -> Value {
    Value::Array(vec![
        Value::from(key.0),
        Value::from(key.1),
        Value::from(key.2),
        Value::from(key.3),
        Value::from(digest),
    ])
}

/// 会话文件身份 = stat 四元组 + 内容摘要。
///
/// `digest_cache` 只在全量扫描时传入；`digest_store`（ScanCache）是同一份摘要
/// 的跨进程缓存，校验四元组与进程内缓存完全一致，所以命中的安全语义相同。
fn path_identity(
    path: &Path,
    digest_cache: Option<&Mutex<DigestCache>>,
    digest_store: Option<&dyn ScanCache>,
) -> Result<(StatKey, String), IdentityError> {
    if let Some(cache) = digest_cache {
        let probe = file_stat(path)?;
        let key = stat_key(&probe);
        let mut cached = cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&key)
            .cloned();
        if cached.is_none() {
            if let Some(store) = digest_store {
                cached = store.get_digest(path, &probe);
                if let Some(digest) = cached.as_ref() {
                    let mut guard = cache
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    if guard.len() >= DIGEST_CACHE_LIMIT {
                        guard.clear();
                    }
                    guard.insert(key, digest.clone());
                }
            }
        }
        if let Some(digest) = cached {
            return Ok((key, digest));
        }
    }
    // 活跃会话随时在被 CLI 追加：哈希中途 stat 变了就整体重试，连续三次
    // 都撞上才承认这一刻拍不出稳定快照。
    let mut settled: Option<(FileStat, String)> = None;
    for _attempt in 0..3 {
        let before = file_stat(path)?;
        let mut digest = Sha256::new();
        let mut stream = std::fs::File::open(path).map_err(|_| IdentityError::Unavailable)?;
        let mut chunk = vec![0u8; 1024 * 1024];
        loop {
            let read = stream
                .read(&mut chunk)
                .map_err(|_| IdentityError::Unavailable)?;
            if read == 0 {
                break;
            }
            digest.update(&chunk[..read]);
        }
        let after = file_stat(path)?;
        if stat_key(&before) == stat_key(&after) {
            let mut hex = String::with_capacity(64);
            for byte in digest.finalize() {
                let _ = write!(hex, "{byte:02x}");
            }
            settled = Some((after, hex));
            break;
        }
    }
    let Some((after, digest)) = settled else {
        return Err(IdentityError::Race);
    };
    let key = stat_key(&after);
    if let Some(cache) = digest_cache {
        let mut guard = cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if guard.len() >= DIGEST_CACHE_LIMIT {
            guard.clear();
        }
        guard.insert(key, digest.clone());
        if let Some(store) = digest_store {
            store.put_digest(path, &after, &digest);
        }
    }
    Ok((key, digest))
}

fn agent_fingerprint(browser: &dyn SessionBrowser, reference: &str) -> DomainResult<Fingerprint> {
    browser.agent_fingerprint(reference)
}

/// 扫描路径的指纹：adapter 可提供容忍旧快照的变体；resolve 的钉内容校验走严格版。
fn scan_fingerprint(browser: &dyn SessionBrowser, reference: &str) -> DomainResult<Fingerprint> {
    match browser.scan_fingerprint(reference) {
        Some(result) => result,
        None => agent_fingerprint(browser, reference),
    }
}

/// 目录型会话（grok bundle）的身份：权威成员逐个算 `path_identity` 后按相对路径排序。
fn directory_identity(
    path: &Path,
    browser: &dyn SessionBrowser,
    digest_cache: Option<&Mutex<DigestCache>>,
    digest_store: Option<&dyn ScanCache>,
) -> Result<Value, IdentityError> {
    let members = browser
        .authoritative_members(&path.to_string_lossy())
        .ok_or(IdentityError::Unavailable)? // 未声明权威成员
        .map_err(|_| IdentityError::Unavailable)?;
    if members.is_empty() {
        return Err(IdentityError::Unavailable);
    }
    let mut identities: Vec<(String, Value)> = Vec::with_capacity(members.len());
    for raw_member in &members {
        let candidate = PathBuf::from(raw_member);
        let candidate = if candidate.is_absolute() {
            candidate
        } else {
            path.join(candidate)
        };
        let resolved = realpath_strict(&candidate).map_err(|_| IdentityError::Unavailable)?;
        let resolved_text = resolved.to_string_lossy().into_owned();
        if !resolved.is_file() || !is_within(&resolved_text, &path.to_string_lossy()) {
            return Err(IdentityError::Unavailable);
        }
        let relative = resolved
            .strip_prefix(path)
            .map_err(|_| IdentityError::Unavailable)?
            .to_string_lossy()
            .into_owned();
        let (key, digest) = path_identity(&resolved, digest_cache, digest_store)?;
        identities.push((relative, identity_value(key, &digest)));
    }
    identities.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(Value::Array(
        identities
            .into_iter()
            .map(|(relative, member)| Value::Array(vec![Value::from(relative), member]))
            .collect(),
    ))
}

// ---------- 索引 ----------

/// `(tool, canonical_ref)`。
type SessionKey = (String, String);
/// `(session_ref, native_locator, role)`。
type MessageKey = (String, String, String);

#[derive(Default)]
struct IndexState {
    by_opaque: HashMap<String, IndexedSession>,
    /// Python dict 的插入序在 `snapshot_with_status` 里是可观测的，必须保留。
    order: Vec<String>,
    opaque_by_key: HashMap<SessionKey, String>,
    messages_by_opaque: HashMap<String, IndexedMessage>,
    opaque_by_message_key: HashMap<MessageKey, String>,
    tool_status: Map<String, Value>,
    bootstrapped: bool,
    generation: i64,
}

impl IndexState {
    fn insert_record(&mut self, record: IndexedSession) {
        if !self.by_opaque.contains_key(&record.opaque_ref) {
            self.order.push(record.opaque_ref.clone());
        }
        self.by_opaque.insert(record.opaque_ref.clone(), record);
    }

    fn remove_record(&mut self, opaque: &str) -> Option<IndexedSession> {
        let removed = self.by_opaque.remove(opaque);
        if removed.is_some() {
            self.order.retain(|item| item != opaque);
        }
        removed
    }

    /// `(tool, canonical) → ref` 的映射保留为墓碑：同一会话被误判消失或原地
    /// 重建后重新入索引时仍拿回原 ref。
    fn drop_message_locators(&mut self, opaque: &str) {
        let stale: Vec<String> = self
            .messages_by_opaque
            .iter()
            .filter(|(_, message)| message.session_ref == opaque)
            .map(|(locator, _)| locator.clone())
            .collect();
        for locator in stale {
            if let Some(message) = self.messages_by_opaque.remove(&locator) {
                self.opaque_by_message_key.remove(&(
                    message.session_ref,
                    message.native_locator,
                    message.role,
                ));
            }
        }
    }
}

type RefreshOutcome = Result<(Map<String, Value>, Vec<IndexedSession>), DomainError>;

/// 一次进行中的全量刷新：后到者在 `done` 上等待并复用结果。
struct RefreshFlight {
    outcome: Mutex<Option<RefreshOutcome>>,
    done: Condvar,
}

/// 增量推送回调。**不得回调进索引自身**（Python 侧靠 RLock 容忍重入，
/// Rust 侧是普通 Mutex；真实 sink 只是往 stdout 写一帧事件）。
pub type DeltaSink = Arc<dyn Fn(&Value) + Send + Sync>;

pub struct AgentSessionIndex {
    ports: Arc<dyn SessionPorts>,
    state: Mutex<IndexState>,
    digest_cache: Mutex<DigestCache>,
    refresh_lock: Mutex<Option<Arc<RefreshFlight>>>,
    /// 全量与单工具刷新互斥：两者交错会让旧的扫描结果覆盖新索引。
    mutate_lock: Mutex<()>,
    on_delta: Mutex<Option<DeltaSink>>,
}

/// `_canonicalize` 的四元返回。
struct Canonicalized {
    canonical: Option<String>,
    root: Option<String>,
    #[allow(dead_code)]
    storage_kind: Option<StorageKind>,
    identity: Option<Value>,
}

impl Canonicalized {
    fn dropped(storage_kind: Option<StorageKind>) -> Self {
        Self {
            canonical: None,
            root: None,
            storage_kind,
            identity: None,
        }
    }
}

impl AgentSessionIndex {
    pub fn new(ports: Arc<dyn SessionPorts>) -> Self {
        // Python 里 `adapters/shared/scanner.py` 在模块加载时就 import 了
        // `sessions.scan_progress.TRACKER`，进度上报**永远是接通的**。Rust 把
        // 方向反转成注册式后，等价物就是「索引一存在，出口就已注册」——放在
        // 组合根会让漏调变成静默的零进度。`OnceLock`，重复调用无副作用。
        super::scan_progress::install_tracker();
        Self {
            ports,
            state: Mutex::new(IndexState::default()),
            digest_cache: Mutex::new(HashMap::new()),
            refresh_lock: Mutex::new(None),
            mutate_lock: Mutex::new(()),
            on_delta: Mutex::new(None),
        }
    }

    pub fn ports(&self) -> &Arc<dyn SessionPorts> {
        &self.ports
    }

    /// 注册 `sessions.changed` 的增量出口（WP-E 接线）。
    pub fn set_on_delta(&self, sink: Option<DeltaSink>) {
        *self
            .on_delta
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = sink;
    }

    fn locked(&self) -> MutexGuard<'_, IndexState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 不做任何校验的记录查表。
    ///
    /// Python 侧 operations 与 sessions 共享同一个 `IndexedSession` 实例，Rust
    /// 侧 `operations::types::IndexedSession` 是收窄后的投影（没有
    /// `root`/`storage_kind`/`source_identity`），组合根的桥接层要靠 opaque ref
    /// 把完整记录取回来。**不校验、不触碰文件系统**——校验仍归 [`Self::resolve`]。
    pub fn record(&self, opaque_ref: &str) -> Option<IndexedSession> {
        self.locked().by_opaque.get(opaque_ref).cloned()
    }

    pub fn refresh(&self) -> DomainResult<Vec<IndexedSession>> {
        Ok(self.refresh_with_status()?.1)
    }

    /// 首次扫描后只刷新请求中的工具，避免带 `--agent` 的搜索重复扫描全部数据源。
    ///
    /// 与全量扫描保持相同的容错语义：单个工具失败时记录失败状态并清除该工具的
    /// 旧快照，但不让整次搜索失败。未知工具无需扫描，调用方过滤后自然得到空集。
    pub fn refresh_selected(&self, names: &[String]) -> DomainResult<Vec<IndexedSession>> {
        if names.is_empty() || !self.locked().bootstrapped {
            return self.refresh();
        }

        let configured: HashSet<String> = self.ports.adapters().into_iter().collect();
        let mut selected: Vec<&String> = names
            .iter()
            .filter(|name| configured.contains(*name))
            .collect();
        selected.sort_unstable();
        for name in selected {
            if let Err(error) = self.refresh_tool(name) {
                self.record_failed_tool_refresh(name, &error)?;
            }
        }

        Ok(self
            .snapshot_with_status()
            .map(|(_, records, _)| records)
            .unwrap_or_default())
    }

    /// 全量扫库并重建索引；并发调用单飞合并（后到者至多陈旧一轮扫描）。
    pub fn refresh_with_status(&self) -> DomainResult<(Map<String, Value>, Vec<IndexedSession>)> {
        let (flight, leader) = {
            let mut guard = self
                .refresh_lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match guard.as_ref() {
                Some(flight) => (flight.clone(), false),
                None => {
                    let flight = Arc::new(RefreshFlight {
                        outcome: Mutex::new(None),
                        done: Condvar::new(),
                    });
                    *guard = Some(flight.clone());
                    (flight, true)
                }
            }
        };
        if !leader {
            let mut outcome = flight
                .outcome
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            while outcome.is_none() {
                outcome = flight
                    .done
                    .wait(outcome)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
            return outcome.clone().expect("等待结束即有结果");
        }
        let result = self.scan_all();
        {
            let mut guard = self
                .refresh_lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *guard = None;
        }
        {
            let mut outcome = flight
                .outcome
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *outcome = Some(result.clone());
            flight.done.notify_all();
        }
        result
    }

    fn scan_all(&self) -> DomainResult<(Map<String, Value>, Vec<IndexedSession>)> {
        let _guard = self
            .mutate_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut tools = Map::new();
        let mut scanned: Vec<(String, ScanRow)> = Vec::new();
        let cache = self.ports.cache_factory();
        let names = self.ports.adapters();
        TRACKER.begin(&names);
        let outcome = (|| -> DomainResult<Vec<IndexedSession>> {
            for name in &names {
                let adapter = self.ports.adapter(name)?;
                let source_path = Value::from(adapter.manifest.source_path.as_str());
                TRACKER.start_tool(name);
                let tool_started = std::time::Instant::now();
                let scan_result = adapter
                    .require_browser()
                    .and_then(|browser| browser.scan(cache.as_ref()));
                crate::server::serve::log_info(&format!(
                    "扫库 {name}: {:.2}s",
                    tool_started.elapsed().as_secs_f64()
                ));
                let mut status = Map::new();
                match scan_result {
                    Ok(rows) => {
                        status.insert("ok".into(), Value::Bool(true));
                        status.insert("count".into(), Value::from(rows.len()));
                        status.insert("path".into(), source_path);
                        scanned.extend(rows.into_iter().map(|row| (name.clone(), row)));
                    }
                    // 单工具失败不拖垮全量。
                    Err(error) => {
                        status.insert("ok".into(), Value::Bool(false));
                        status.insert(
                            "error".into(),
                            Value::from(super::safety::truncate_text(error.message(), 200).0),
                        );
                        status.insert("path".into(), source_path);
                    }
                }
                tools.insert(name.clone(), Value::Object(status));
                TRACKER.finish_tool(name);
            }
            TRACKER.finalize();
            cache.flush();
            let records = self.index_rows(&scanned, None)?;
            // 扫描主体完成后才做各 adapter 的维护，避免与扫描争抢 CPU。
            for name in &names {
                let Ok(adapter) = self.ports.adapter(name) else {
                    continue;
                };
                let Some(browser) = adapter.browser.as_deref() else {
                    continue;
                };
                if let Some(Err(error)) = browser.post_scan_maintenance() {
                    // 维护失败不影响扫描结果。
                    let _ = error;
                }
            }
            {
                let mut state = self.locked();
                state.tool_status = tools.clone();
                state.bootstrapped = true;
            }
            Ok(records)
        })();
        TRACKER.end();
        outcome.map(|records| (tools, records))
    }

    /// 当前活索引快照；首次全量扫描完成前返回 `None`。
    #[allow(clippy::type_complexity)]
    pub fn snapshot_with_status(&self) -> Option<(Map<String, Value>, Vec<IndexedSession>, i64)> {
        let state = self.locked();
        if !state.bootstrapped {
            return None;
        }
        let records = state
            .order
            .iter()
            .filter_map(|opaque| state.by_opaque.get(opaque).cloned())
            .collect();
        Some((state.tool_status.clone(), records, state.generation))
    }

    pub fn generation(&self) -> i64 {
        self.locked().generation
    }

    /// 只重扫一个工具并增量并入索引；delta 经 `on_delta` 推出。
    pub fn refresh_tool(&self, name: &str) -> DomainResult<()> {
        let adapter = self.ports.adapter(name)?;
        let source_path = Value::from(adapter.manifest.source_path.as_str());
        let _guard = self
            .mutate_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let cache = self.ports.cache_factory();
        let rows = adapter.require_browser()?.scan(cache.as_ref())?;
        cache.flush();
        let count = rows.len();
        let scanned: Vec<(String, ScanRow)> = rows
            .into_iter()
            .map(|row| (name.to_string(), row))
            .collect();
        let mut scope = HashSet::new();
        scope.insert(name.to_string());
        self.index_rows(&scanned, Some(&scope))?;
        let mut status = Map::new();
        status.insert("ok".into(), Value::Bool(true));
        status.insert("count".into(), Value::from(count));
        status.insert("path".into(), source_path);
        self.locked()
            .tool_status
            .insert(name.to_string(), Value::Object(status));
        Ok(())
    }

    fn record_failed_tool_refresh(&self, name: &str, error: &DomainError) -> DomainResult<()> {
        let adapter = self.ports.adapter(name)?;
        let _guard = self
            .mutate_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut scope = HashSet::new();
        scope.insert(name.to_string());
        self.index_rows(&[], Some(&scope))?;

        let mut status = Map::new();
        status.insert("ok".into(), Value::Bool(false));
        status.insert(
            "error".into(),
            Value::from(super::safety::truncate_text(error.message(), 200).0),
        );
        status.insert(
            "path".into(),
            Value::from(adapter.manifest.source_path.as_str()),
        );
        self.locked()
            .tool_status
            .insert(name.to_string(), Value::Object(status));
        Ok(())
    }

    fn digest_store(&self) -> Arc<dyn ScanCache> {
        self.ports.cache_factory()
    }

    fn store_identity_digests(&self, path: &Path, storage_kind: StorageKind, identity: &Value) {
        let store = self.digest_store();
        let entries: Vec<(PathBuf, &Value)> = match storage_kind {
            StorageKind::File => match identity.get(0) {
                Some(member) => vec![(path.to_path_buf(), member)],
                None => return,
            },
            _ => match identity.as_array() {
                Some(members) => members
                    .iter()
                    .filter_map(|member| {
                        let relative = member.get(0)?.as_str()?;
                        Some((path.join(relative), member.get(1)?))
                    })
                    .collect(),
                None => return,
            },
        };
        for (target, member) in entries {
            let Some(fields) = member.as_array() else {
                continue;
            };
            if fields.len() < 5 {
                continue;
            }
            let stat = FileStat {
                dev: fields[0].as_u64().unwrap_or_default(),
                ino: fields[1].as_u64().unwrap_or_default(),
                mtime_ns: i128::from(fields[2].as_i64().unwrap_or_default()),
                size: fields[3].as_u64().unwrap_or_default(),
            };
            if let Some(digest) = fields[4].as_str() {
                store.put_digest(&target, &stat, digest);
            }
        }
        store.flush();
    }

    /// 把扫描行并入索引。`scope` 限定本次完整覆盖的工具集合：只有 scope 内
    /// 工具的缺失会话才会被淘汰（`None` = 全量扫描）。
    pub fn index_rows(
        &self,
        scanned: &[(String, ScanRow)],
        scope: Option<&HashSet<String>>,
    ) -> DomainResult<Vec<IndexedSession>> {
        let digest_store = self.digest_store();
        let canonical_rows = self.canonicalize_all(scanned, digest_store.as_ref())?;
        digest_store.flush();

        let mut records: Vec<IndexedSession> = Vec::new();
        let mut active: HashSet<String> = HashSet::new();
        let mut upserts: Vec<IndexedSession> = Vec::new();
        let mut removals: Vec<String> = Vec::new();
        let mut state = self.locked();
        for ((tool_name, row), resolved) in scanned.iter().zip(canonical_rows) {
            let Some(canonical) = resolved.canonical else {
                continue;
            };
            let Some(identity) = resolved.identity else {
                // 身份竞态（文件正被追加）：沿用上一轮记录，revision 由之后
                // 安静的一轮扫描收敛；首见即竞态则等下一轮再入索引。
                let key = (tool_name.clone(), canonical.clone());
                let prior = state
                    .opaque_by_key
                    .get(&key)
                    .and_then(|opaque| state.by_opaque.get(opaque))
                    .cloned();
                if let Some(prior) = prior {
                    active.insert(prior.opaque_ref.clone());
                    records.push(prior);
                }
                continue;
            };
            let new_revision = revision(tool_name, &canonical, row, &identity);
            // ref 按 (tool, canonical) 签发：内容变化只更新 revision/identity，
            // 不换发 ref。
            let key = (tool_name.clone(), canonical.clone());
            let opaque = match state.opaque_by_key.get(&key) {
                Some(existing) => existing.clone(),
                None => {
                    let issued = issue_token("fsr_");
                    state.opaque_by_key.insert(key, issued.clone());
                    issued
                }
            };
            let record = IndexedSession {
                opaque_ref: opaque.clone(),
                tool: tool_name.clone(),
                canonical_ref: canonical,
                root: resolved.root,
                storage_kind: resolved.storage_kind.unwrap_or(StorageKind::Id),
                row: row.clone(),
                revision: new_revision,
                source_identity: identity,
            };
            let changed = match state.by_opaque.get(&opaque) {
                None => true,
                Some(prior) => prior.revision != record.revision || prior.row != record.row,
            };
            if changed {
                upserts.push(record.clone());
            }
            state.insert_record(record.clone());
            active.insert(opaque);
            records.push(record);
        }
        let stale: Vec<String> = state
            .order
            .iter()
            .filter(|opaque| !active.contains(*opaque))
            .filter(|opaque| match scope {
                None => true,
                Some(scope) => state
                    .by_opaque
                    .get(*opaque)
                    .map(|record| scope.contains(&record.tool))
                    .unwrap_or(false),
            })
            .cloned()
            .collect();
        for opaque in stale {
            state.remove_record(&opaque);
            state.drop_message_locators(&opaque);
            removals.push(opaque);
        }
        // 首次全量扫描（bootstrap）不推增量：前端此时正拿全量快照。
        // 在锁内推送保证 generation 与事件顺序严格一致。
        if state.bootstrapped && (!upserts.is_empty() || !removals.is_empty()) {
            state.generation += 1;
            self.publish(state.generation, &upserts, &removals);
        }
        Ok(records)
    }

    fn publish(&self, generation: i64, upserts: &[IndexedSession], removals: &[String]) {
        let sink = self
            .on_delta
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let Some(sink) = sink else {
            return;
        };
        let mut payload = Map::new();
        payload.insert("generation".into(), Value::from(generation));
        payload.insert(
            "upserts".into(),
            Value::Array(
                upserts
                    .iter()
                    .map(|record| Value::Object(session_dto(record)))
                    .collect(),
            ),
        );
        payload.insert(
            "removals".into(),
            Value::Array(
                removals
                    .iter()
                    .map(|item| Value::from(item.as_str()))
                    .collect(),
            ),
        );
        sink(&Value::Object(payload));
    }

    /// 删除会话后的定点摘除：立刻移出索引并推 removal delta。
    /// `(tool, canonical)` → ref 映射保留为墓碑。
    pub fn evict(&self, tool: &str, canonical_ref: &str) {
        let mut state = self.locked();
        let key = (tool.to_string(), canonical_ref.to_string());
        let Some(opaque) = state.opaque_by_key.get(&key).cloned() else {
            return;
        };
        if !state.by_opaque.contains_key(&opaque) {
            return;
        }
        state.remove_record(&opaque);
        state.drop_message_locators(&opaque);
        if !state.bootstrapped {
            return;
        }
        state.generation += 1;
        self.publish(state.generation, &[], std::slice::from_ref(&opaque));
    }

    /// 把 opaque ref 换回索引记录。
    ///
    /// `pin_content = true`（Agent 读取与编辑路径）要求会话内容与签发时一字未
    /// 变；`false`（UI 只读浏览）只做路径归属与存在性校验。
    pub fn resolve(
        &self,
        tool: &str,
        opaque_ref: &str,
        pin_content: bool,
    ) -> DomainResult<IndexedSession> {
        if !is_opaque_session_ref(opaque_ref) {
            return Err(reference_error(
                "ref 不是 Engine 签发的 opaque ref",
                Map::new(),
            ));
        }
        let record = self.locked().by_opaque.get(opaque_ref).cloned();
        if let Some(record) = record.as_ref() {
            if record.tool != tool {
                // ref 已能唯一定位会话，tool 配错是 agent 高频笔误。
                let mut params = Map::new();
                params.insert("expected_tool".into(), Value::from(record.tool.as_str()));
                params.insert("given_tool".into(), Value::from(tool));
                params.insert("reason".into(), Value::from("tool_mismatch"));
                params.insert(
                    "recovery".into(),
                    Value::from(format!("retry the same ref with tool={}", record.tool)),
                );
                return Err(reference_error(
                    format!("ref 属于 {} 会话，不属于 {tool}", record.tool),
                    params,
                ));
            }
        }
        let Some(record) = record else {
            return Err(reference_error(
                "ref 不在当前扫描索引中",
                failure_params(tool, "unknown_ref"),
            ));
        };
        let browser = self.ports.adapter(tool)?.require_browser()?;
        match record.storage_kind {
            StorageKind::File | StorageKind::Directory => {
                let resolved = realpath_strict(Path::new(&record.canonical_ref)).map_err(|_| {
                    reference_error(
                        "ref 指向的会话已失效",
                        failure_params(tool, "session_missing"),
                    )
                })?;
                let root = realpath_strict(Path::new(record.root.as_deref().unwrap_or("")))
                    .map_err(|_| {
                        reference_error(
                            "ref 指向的会话已失效",
                            failure_params(tool, "session_missing"),
                        )
                    })?;
                let resolved_text = resolved.to_string_lossy().into_owned();
                let type_ok = match record.storage_kind {
                    StorageKind::File => resolved.is_file(),
                    _ => resolved.is_dir(),
                };
                if !is_within(&resolved_text, &root.to_string_lossy()) || !type_ok {
                    return Err(reference_error("ref 超出 Agent 会话根目录", Map::new()));
                }
                if pin_content {
                    let identity = self.pinned_identity(&record, &resolved, browser)?;
                    let missing_fingerprint = record.storage_kind == StorageKind::File
                        && identity.get(1).map(Value::is_null).unwrap_or(true);
                    if missing_fingerprint || record.source_identity != identity {
                        // 摘要缓存可能命中了「stat 没变但内容变了」的旧值：踢掉它。
                        {
                            let mut cache = self
                                .digest_cache
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner());
                            if record.storage_kind == StorageKind::File {
                                if let Some(key) = identity.get(0).and_then(stat_key_of) {
                                    cache.remove(&key);
                                }
                            } else {
                                cache.clear();
                            }
                        }
                        // 同一条陈旧摘要还躺在跨进程缓存里，手上正好有刚算出的
                        // 真实摘要，直接写回。
                        self.store_identity_digests(&resolved, record.storage_kind, &identity);
                        return Err(reference_error(
                            "ref 在扫描后已变化，请重新搜索",
                            failure_params(tool, "session_changed"),
                        ));
                    }
                }
                let adapter_ref = browser.resolve_ref(&resolved_text)?;
                let round_trip = realpath_strict(Path::new(&adapter_ref))
                    .map_err(|_| reference_error("adapter 未能规范解析 ref", Map::new()))?;
                if round_trip != resolved {
                    return Err(reference_error("adapter 未能规范解析 ref", Map::new()));
                }
            }
            StorageKind::Id => {
                // 钉内容才需要严格指纹；只查存在性时用扫描口径的宽松指纹。
                let fingerprint = if pin_content {
                    agent_fingerprint(browser, &record.canonical_ref)?
                } else {
                    scan_fingerprint(browser, &record.canonical_ref)?
                };
                if fingerprint.is_null() {
                    return Err(reference_error(
                        "ref 指向的会话已失效",
                        failure_params(tool, "session_missing"),
                    ));
                }
                if pin_content && fingerprint != record.source_identity {
                    return Err(reference_error(
                        "ref 在扫描后已变化，请重新搜索",
                        failure_params(tool, "session_changed"),
                    ));
                }
            }
        }
        Ok(record)
    }

    fn pinned_identity(
        &self,
        record: &IndexedSession,
        resolved: &Path,
        browser: &dyn SessionBrowser,
    ) -> DomainResult<Value> {
        let stale = || reference_error("ref 指向的会话已失效", Map::new());
        if record.storage_kind == StorageKind::File {
            let (key, digest) = path_identity(resolved, None, None).map_err(|_| stale())?;
            // Python 里 fingerprint 抛 OSError/ValueError 走 session_missing 语义
            // （无 params 的兜底文案），返回 None 才是 session_changed。
            let fingerprint =
                agent_fingerprint(browser, &resolved.to_string_lossy()).map_err(|_| stale())?;
            Ok(Value::Array(vec![
                identity_value(key, &digest),
                fingerprint,
            ]))
        } else {
            directory_identity(resolved, browser, None, None).map_err(|_| stale())
        }
    }

    pub fn issue_message_locator(
        &self,
        record: &IndexedSession,
        native_locator: &str,
        role: &str,
        editable: bool,
    ) -> DomainResult<String> {
        if native_locator.is_empty() || native_locator.chars().count() > 512 {
            return Err(reference_error("消息缺少可编辑定位信息", Map::new()));
        }
        let key = (
            record.opaque_ref.clone(),
            native_locator.to_string(),
            role.to_string(),
        );
        let mut state = self.locked();
        let opaque = match state.opaque_by_message_key.get(&key) {
            Some(existing) => existing.clone(),
            None => {
                let issued = issue_token("fml_");
                state.opaque_by_message_key.insert(key, issued.clone());
                issued
            }
        };
        state.messages_by_opaque.insert(
            opaque.clone(),
            IndexedMessage {
                opaque_locator: opaque.clone(),
                session_ref: record.opaque_ref.clone(),
                tool: record.tool.clone(),
                revision: record.revision.clone(),
                native_locator: native_locator.to_string(),
                role: role.to_string(),
                editable,
            },
        );
        Ok(opaque)
    }

    pub fn resolve_message_locator(
        &self,
        record: &IndexedSession,
        opaque_locator: &str,
    ) -> DomainResult<IndexedMessage> {
        self.resolve_message_locator_parts(
            &record.opaque_ref,
            &record.tool,
            &record.revision,
            opaque_locator,
        )
    }

    /// [`Self::resolve_message_locator`] 的按字段变体。
    ///
    /// 校验只看 `(session_ref, tool, revision)` 三元组，组合根的 operations 桥接层
    /// 手上只有这三个字段（见 `operations::types::IndexedSession`），不必为了调用
    /// 它去重新解析一次完整记录——重新解析会把校验基准换成**当前**索引里的
    /// revision，从而吞掉「记录已过期」这条错误。
    pub fn resolve_message_locator_parts(
        &self,
        session_ref: &str,
        tool: &str,
        revision: &str,
        opaque_locator: &str,
    ) -> DomainResult<IndexedMessage> {
        const HINT: &str = "重新调用 ferry_get_session_context，并原样使用 messages[].locator";
        let mut params = Map::new();
        params.insert("field".into(), Value::from("locator"));
        params.insert("hint".into(), Value::from(HINT));
        if !opaque_locator.starts_with("fml_") {
            return Err(reference_error(
                "locator 不是 Engine 签发的消息引用",
                params,
            ));
        }
        let message = self
            .locked()
            .messages_by_opaque
            .get(opaque_locator)
            .cloned();
        let ok = message.as_ref().is_some_and(|message| {
            message.session_ref == session_ref
                && message.tool == tool
                && message.revision == revision
        });
        if !ok {
            return Err(DomainError::locator_stale(
                Some("消息引用已失效或不属于当前会话"),
                params,
            ));
        }
        Ok(message.expect("上一步已校验存在"))
    }

    fn canonicalize_all(
        &self,
        scanned: &[(String, ScanRow)],
        digest_store: &dyn ScanCache,
    ) -> DomainResult<Vec<Canonicalized>> {
        if scanned.len() < PARALLEL_CANONICALIZE_THRESHOLD {
            return scanned
                .iter()
                .map(|(tool, row)| self.canonicalize(tool, row, digest_store))
                .collect();
        }
        // 摘要之间互不依赖，且哈希与文件读取都是 IO/CPU 混合负载，先并行算完，
        // 再在锁内串行做签发与淘汰。
        CANONICALIZE_POOL.install(|| {
            use rayon::prelude::*;
            scanned
                .par_iter()
                .map(|(tool, row)| self.canonicalize(tool, row, digest_store))
                .collect()
        })
    }

    fn canonicalize(
        &self,
        tool: &str,
        row: &ScanRow,
        digest_store: &dyn ScanCache,
    ) -> DomainResult<Canonicalized> {
        let browser = self.ports.adapter(tool)?.require_browser()?;
        let Some(native) = browser.canonicalize(row) else {
            return Ok(Canonicalized::dropped(None));
        };
        let kind = native.storage_kind();
        if kind == StorageKind::Id {
            // resolve_ref 必须恒等，否则该行不入索引：引用解析不稳定的行，
            // 后续任何按引用回查都会指向别处。adapter 抛错时按「本行不可用」
            // 降级，扫描不因单行缺陷全废。
            let round_trip = browser.resolve_ref(native.canonical_ref());
            if round_trip.as_deref() != Ok(native.canonical_ref()) {
                return Ok(Canonicalized::dropped(Some(StorageKind::Id)));
            }
            let fingerprint = match scan_fingerprint(browser, native.canonical_ref()) {
                Ok(fingerprint) if !fingerprint.is_null() => fingerprint,
                _ => return Ok(Canonicalized::dropped(Some(StorageKind::Id))),
            };
            return Ok(Canonicalized {
                canonical: Some(native.canonical_ref().to_string()),
                root: None,
                storage_kind: Some(StorageKind::Id),
                identity: Some(fingerprint),
            });
        }
        let (Ok(root), Ok(path)) = (
            realpath_strict(Path::new(native.root().unwrap_or(""))),
            realpath_strict(Path::new(native.canonical_ref())),
        ) else {
            return Ok(Canonicalized::dropped(Some(kind)));
        };
        let path_text = path.to_string_lossy().into_owned();
        let root_text = root.to_string_lossy().into_owned();
        if !is_within(&path_text, &root_text) {
            return Ok(Canonicalized::dropped(Some(kind)));
        }
        let type_ok = match kind {
            StorageKind::File => path.is_file(),
            _ => path.is_dir(),
        };
        if !type_ok {
            return Ok(Canonicalized::dropped(Some(kind)));
        }
        let identity = if kind == StorageKind::File {
            match path_identity(&path, Some(&self.digest_cache), Some(digest_store)) {
                Ok((key, digest)) => {
                    match scan_fingerprint(browser, &path_text) {
                        Ok(fingerprint) if !fingerprint.is_null() => Ok(Value::Array(vec![
                            identity_value(key, &digest),
                            fingerprint,
                        ])),
                        // 指纹缺失或 adapter 报错：该行本轮不入索引。
                        _ => return Ok(Canonicalized::dropped(Some(kind))),
                    }
                }
                Err(error) => Err(error),
            }
        } else {
            directory_identity(&path, browser, Some(&self.digest_cache), Some(digest_store))
        };
        match identity {
            Ok(identity) => Ok(Canonicalized {
                canonical: Some(path_text),
                root: Some(root_text),
                storage_kind: Some(kind),
                identity: Some(identity),
            }),
            // 文件还在，只是这一轮拍不出稳定快照：canonical 照常返回、identity
            // 置 None，index_rows 据此沿用上一轮记录。
            Err(IdentityError::Race) => Ok(Canonicalized {
                canonical: Some(path_text),
                root: Some(root_text),
                storage_kind: Some(kind),
                identity: None,
            }),
            Err(IdentityError::Unavailable) => Ok(Canonicalized::dropped(Some(kind))),
        }
    }
}

fn stat_key_of(member: &Value) -> Option<StatKey> {
    let fields = member.as_array()?;
    Some((
        fields.first()?.as_u64()?,
        fields.get(1)?.as_u64()?,
        fields.get(2)?.as_i64()?,
        fields.get(3)?.as_u64()?,
    ))
}

fn failure_params(tool: &str, reason: &str) -> Map<String, Value> {
    let mut params = Map::new();
    params.insert("tool".into(), Value::from(tool));
    params.insert("reason".into(), Value::from(reason));
    params.insert("recovery".into(), Value::from(REF_RECOVERY_HINT));
    params
}

fn reference_error(message: impl Into<String>, params: Map<String, Value>) -> DomainError {
    DomainError::new(
        "agent.reference_invalid",
        "AgentReferenceError",
        message,
        params,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 期望值现场取自 Python：
    /// ```text
    /// python3 -c "import hashlib,json;print(hashlib.sha256(json.dumps(
    ///   {'tool':'claude','ref':'/tmp/a.jsonl','updated':1,'size':2,
    ///    'file_identity':[[1,2,3,4,'ab'],None]},
    ///   sort_keys=True,separators=(',',':')).encode()).hexdigest())"
    /// ```
    #[test]
    fn revision_matches_python_byte_for_byte() {
        let mut row = ScanRow::new();
        row.insert("updated".into(), Value::from(1));
        row.insert("size".into(), Value::from(2));
        let identity = json!([[1, 2, 3, 4, "ab"], null]);
        assert_eq!(
            revision("claude", "/tmp/a.jsonl", &row, &identity),
            "14ec810c693789a3b9d832d77b6ca439d5277f2fdcc8209df1d08687494be3f4"
        );
        assert_eq!(
            stable_json(&json!({
                "tool": "claude", "ref": "/tmp/a.jsonl", "updated": 1, "size": 2,
                "file_identity": [[1, 2, 3, 4, "ab"], null],
            })),
            r#"{"file_identity":[[1,2,3,4,"ab"],null],"ref":"/tmp/a.jsonl","size":2,"tool":"claude","updated":1}"#
        );
    }

    /// 三种 `file_identity` 形态 + 非 ASCII 路径 + 缺键，逐个对 Python 取值。
    ///
    /// 期望值现场取自 Python（`sort_keys=True, separators=(",", ":")`，
    /// `ensure_ascii` 保持默认 True）：
    /// ```text
    /// python3 -c "import hashlib,json
    /// for tool, ref, row, ident in [
    ///   ('grok','/tmp/中文/会话',{'updated':1712345678901,'size':None},
    ///    [['a.json',[1,2,3,4,'aa']],['b/c.json',[1,2,5,6,'bb']]]),
    ///   ('opencode','ses_abc',{'updated':0,'size':0},'stat:deadbeef'),
    ///   ('pi','/tmp/x',{},None)]:
    ///     s = json.dumps({'tool':tool,'ref':ref,'updated':row.get('updated'),
    ///                     'size':row.get('size'),'file_identity':ident},
    ///                    sort_keys=True, separators=(',',':'))
    ///     print(hashlib.sha256(s.encode()).hexdigest())"
    /// ```
    #[test]
    fn revision_covers_every_identity_shape() {
        // 1) 目录型（grok bundle）：成员表 + 中文路径 + size 缺失。
        let mut row = ScanRow::new();
        row.insert("updated".into(), Value::from(1712345678901i64));
        row.insert("size".into(), Value::Null);
        assert_eq!(
            revision(
                "grok",
                "/tmp/中文/会话",
                &row,
                &json!([
                    ["a.json", [1, 2, 3, 4, "aa"]],
                    ["b/c.json", [1, 2, 5, 6, "bb"]]
                ]),
            ),
            "8951577f3a5d01ef70c2b9f175a16c8defe7fc2e3c1d87e230355fcb301cffd2"
        );

        // 2) id 型：identity 是 adapter 指纹字符串本身。
        let mut row = ScanRow::new();
        row.insert("updated".into(), Value::from(0));
        row.insert("size".into(), Value::from(0));
        assert_eq!(
            revision("opencode", "ses_abc", &row, &json!("stat:deadbeef")),
            "2448684651fd135ff1b738444baf6c1aa5c800dc175570d4d4ed5e2d1f3add2b"
        );

        // 3) 扫描行缺 updated/size：`row.get(...)` 回落 None，不是 0 也不是省略键。
        assert_eq!(
            revision("pi", "/tmp/x", &ScanRow::new(), &Value::Null),
            "ef472def249948ad312a224a5a76e5384f96760ac2d08a7d7736327a4362b5e2"
        );
    }

    /// `ensure_ascii=True` 是 revision 的一部分：中文路径必须转成 `\uXXXX`。
    #[test]
    fn stable_json_escapes_non_ascii_like_python() {
        assert_eq!(stable_json(&json!({"a": "中"})), r#"{"a":"\u4e2d"}"#);
        // astral 平面转代理对，与 Python `json.dumps` 一致。
        assert_eq!(stable_json(&json!("🚢")), r#""\ud83d\udea2""#);
        assert_eq!(
            stable_json(&json!({"b": 1, "a": null})),
            r#"{"a":null,"b":1}"#
        );
        assert_eq!(stable_json(&json!("a\tb\"c\\d")), r#""a\tb\"c\\d""#);
        assert_eq!(stable_json(&json!("\u{1}")), r#""\u0001""#);
        assert_eq!(stable_json(&json!([1, [2]])), "[1,[2]]");
    }

    #[test]
    fn issued_refs_are_24_character_url_safe_tokens() {
        let token = issue_token("fsr_");
        assert!(token.starts_with("fsr_"));
        assert_eq!(token.len(), 4 + 24);
        assert!(is_opaque_session_ref(&token));
        assert_ne!(token, issue_token("fsr_"));
    }

    #[test]
    fn identity_value_keeps_the_python_tuple_shape() {
        let value = identity_value((1, 2, 3, 4), "ab");
        assert_eq!(value, json!([1, 2, 3, 4, "ab"]));
        assert_eq!(stat_key_of(&value), Some((1, 2, 3, 4)));
    }
}

/// 黄金扫描行驱动的索引集成测试。
///
/// 适配器（WP-C1..C5）尚未就绪，这里用 [`FakeBrowser`] 直接把
/// `tests/golden/scan/<agent>/<case>.json` 的扫描行物化成真实文件/目录，
/// 驱动 `AgentSessionIndex` 的 revision / `fsr_` 签发 / delta 推送逻辑。
/// scanner 级的黄金对照留给 C 系。
#[cfg(test)]
pub(crate) mod golden_tests {
    use super::*;
    use crate::adapters::contracts::{
        id_reference, AgentAdapter, AgentManifest, NativeSessionReference,
    };
    use crate::model::Session;
    use serde_json::json;
    use std::collections::BTreeMap;

    /// 用黄金扫描行伪造的只读 browser。
    struct FakeBrowser {
        root: PathBuf,
        rows: Mutex<Vec<ScanRow>>,
        members: Mutex<HashMap<String, Vec<String>>>,
        fingerprints: Mutex<HashMap<String, Value>>,
        /// 单飞测试用：`refresh` 真正落到 adapter 的次数。
        scans: std::sync::atomic::AtomicUsize,
        /// 单飞测试用：给扫描加一段可观测的耗时，让后到者一定撞进飞行中。
        slow: std::sync::atomic::AtomicBool,
    }

    impl FakeBrowser {
        fn materialize(root: &Path, golden: &Value) -> Self {
            let root_text = root.to_string_lossy().into_owned();
            let mut rows = Vec::new();
            let mut members = HashMap::new();
            for row in golden["rows"].as_array().expect("黄金文件必须有 rows") {
                let mut row = row.as_object().expect("扫描行是 object").clone();
                let raw = row
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                if raw.is_empty() {
                    // opencode：id 型会话，没有文件路径。
                    rows.push(row);
                    continue;
                }
                let path = raw.replace("<home>", &root_text);
                let names: Vec<String> = row
                    .get("authoritative_members")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                if names.is_empty() {
                    // 文件型：按扫描行声明的 size 写出等长内容。
                    let size = row.get("size").and_then(Value::as_i64).unwrap_or(0) as usize;
                    std::fs::create_dir_all(Path::new(&path).parent().expect("有父目录")).unwrap();
                    std::fs::write(&path, "x".repeat(size)).unwrap();
                } else {
                    // 目录型（grok bundle）：建目录并写出全部权威成员。
                    std::fs::create_dir_all(&path).unwrap();
                    for name in &names {
                        std::fs::write(Path::new(&path).join(name), format!("{name} body"))
                            .unwrap();
                    }
                    members.insert(path.clone(), names);
                }
                row.insert("path".into(), Value::from(path));
                rows.push(row);
            }
            Self {
                root: root.to_path_buf(),
                rows: Mutex::new(rows),
                members: Mutex::new(members),
                fingerprints: Mutex::new(HashMap::new()),
                scans: std::sync::atomic::AtomicUsize::new(0),
                slow: std::sync::atomic::AtomicBool::new(false),
            }
        }

        fn set_fingerprint(&self, reference: &str, value: Value) {
            self.fingerprints
                .lock()
                .unwrap()
                .insert(reference.to_string(), value);
        }
    }

    impl SessionBrowser for FakeBrowser {
        fn scan(&self, _cache: &dyn ScanCache) -> DomainResult<Vec<ScanRow>> {
            self.scans.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if self.slow.load(std::sync::atomic::Ordering::SeqCst) {
                // 让并发的后到者有机会撞进同一次飞行（单飞测试）。
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Ok(self.rows.lock().unwrap().clone())
        }

        fn read(&self, reference: &str) -> DomainResult<Session> {
            Ok(Session::new("fake", reference, "/tmp"))
        }

        fn read_agent(&self, reference: &str) -> DomainResult<Session> {
            self.read(reference)
        }

        fn resolve_ref(&self, reference: &str) -> DomainResult<String> {
            Ok(reference.to_string())
        }

        fn fingerprint(&self, reference: &str) -> DomainResult<Fingerprint> {
            Ok(self
                .fingerprints
                .lock()
                .unwrap()
                .get(reference)
                .cloned()
                .unwrap_or_else(|| Value::from("fp")))
        }

        fn agent_fingerprint(&self, reference: &str) -> DomainResult<Fingerprint> {
            self.fingerprint(reference)
        }

        fn canonicalize(&self, row: &ScanRow) -> Option<NativeSessionReference> {
            let path = row.get("path").and_then(Value::as_str).unwrap_or_default();
            if path.is_empty() {
                return id_reference(row);
            }
            let kind = if self.members.lock().unwrap().contains_key(path) {
                StorageKind::Directory
            } else {
                StorageKind::File
            };
            NativeSessionReference::new(path, Some(self.root.to_string_lossy().into_owned()), kind)
                .ok()
        }

        fn validate_read_scope(&self, _reference: &NativeSessionReference) -> DomainResult<()> {
            Ok(())
        }

        fn authoritative_members(&self, reference: &str) -> Option<DomainResult<Vec<String>>> {
            self.members.lock().unwrap().get(reference).cloned().map(Ok)
        }
    }

    struct FakePorts {
        adapters: BTreeMap<String, AgentAdapter>,
        order: Vec<String>,
        cache: Arc<dyn ScanCache>,
    }

    impl SessionPorts for FakePorts {
        fn adapter(&self, name: &str) -> DomainResult<&AgentAdapter> {
            self.adapters
                .get(name)
                .ok_or_else(|| DomainError::tool_unknown(name))
        }

        fn adapters(&self) -> Vec<String> {
            self.order.clone()
        }

        fn cache_factory(&self) -> Arc<dyn ScanCache> {
            self.cache.clone()
        }
    }

    fn golden_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/golden/scan")
            .canonicalize()
            .expect("黄金基线目录必须存在")
    }

    pub(crate) struct Harness {
        _temp: tempfile::TempDir,
        pub(crate) index: Arc<AgentSessionIndex>,
        browsers: BTreeMap<String, Arc<FakeBrowser>>,
        pub(crate) deltas: Arc<Mutex<Vec<Value>>>,
    }

    pub(crate) fn harness() -> Harness {
        let temp = tempfile::tempdir().expect("临时目录");
        let root = realpath_strict(temp.path()).expect("临时目录可 realpath");
        let mut adapters = BTreeMap::new();
        let mut order = Vec::new();
        let mut browsers = BTreeMap::new();
        for entry in std::fs::read_dir(golden_dir()).expect("读黄金目录") {
            let agent_dir = entry.expect("目录项").path();
            if !agent_dir.is_dir() {
                continue;
            }
            let agent = agent_dir
                .file_name()
                .expect("有目录名")
                .to_string_lossy()
                .into_owned();
            let mut rows: Vec<ScanRow> = Vec::new();
            let mut cases: Vec<PathBuf> = std::fs::read_dir(&agent_dir)
                .expect("读 case")
                .filter_map(Result::ok)
                .map(|item| item.path())
                .collect();
            cases.sort();
            let mut merged = Map::new();
            let mut all_rows = Vec::new();
            for case in cases {
                let golden: Value =
                    serde_json::from_str(&std::fs::read_to_string(&case).expect("读黄金文件"))
                        .expect("黄金文件是合法 JSON");
                all_rows.extend(golden["rows"].as_array().cloned().unwrap_or_default());
            }
            merged.insert("rows".into(), Value::Array(all_rows));
            let browser = Arc::new(FakeBrowser::materialize(&root, &Value::Object(merged)));
            rows.extend(browser.rows.lock().unwrap().iter().cloned());
            assert!(!rows.is_empty(), "{agent} 必须有黄金扫描行");
            let manifest = AgentManifest {
                id: agent.clone(),
                display_name: agent.clone(),
                icon: agent.clone(),
                source_path: root.to_string_lossy().into_owned(),
                capabilities: vec!["browse".into()],
                edit_operations: Vec::new(),
                executables: Vec::new(),
                fallback_bin_dirs: Vec::new(),
            };
            let adapter = AgentAdapter::builder()
                .browser(browser.clone() as Arc<dyn SessionBrowser>)
                .build(manifest)
                .expect("adapter 装配");
            order.push(agent.clone());
            adapters.insert(agent.clone(), adapter);
            browsers.insert(agent, browser);
        }
        let cache: Arc<dyn ScanCache> = Arc::new(super::super::scan_cache::ScanCache::new(Some(
            temp.path().join("scan-cache.json"),
        )));
        let index = Arc::new(AgentSessionIndex::new(Arc::new(FakePorts {
            adapters,
            order,
            cache,
        })));
        let deltas: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = deltas.clone();
        index.set_on_delta(Some(Arc::new(move |payload: &Value| {
            sink.lock().unwrap().push(payload.clone());
        })));
        Harness {
            _temp: temp,
            index,
            browsers,
            deltas,
        }
    }

    #[test]
    fn golden_rows_get_stable_refs_and_hex_revisions() {
        let harness = harness();
        let records = harness.index.refresh().expect("首扫成功");
        // 13 个 case × 每个 1 行。
        assert_eq!(records.len(), 13);
        for record in &records {
            assert!(record.opaque_ref.starts_with("fsr_"));
            assert_eq!(record.opaque_ref.len(), 28);
            assert_eq!(record.revision.len(), 64);
            assert!(record.revision.chars().all(|c| c.is_ascii_hexdigit()));
            assert!(!record.source_identity.is_null());
        }
        // 三种存储形态都覆盖到了。
        // StorageKind 没有 Hash（WP-A 的契约面，不改），用列表判定即可。
        let kinds: Vec<StorageKind> = records.iter().map(|record| record.storage_kind).collect();
        assert!(kinds.contains(&StorageKind::File));
        assert!(kinds.contains(&StorageKind::Directory));
        assert!(kinds.contains(&StorageKind::Id));

        // bootstrap 首扫不推增量。
        assert!(harness.deltas.lock().unwrap().is_empty());

        // 二次刷新：ref 与 revision 完全不变，也不产生 delta。
        let again = harness.index.refresh().expect("二次刷新");
        let before: Vec<(String, String)> = records
            .iter()
            .map(|record| (record.opaque_ref.clone(), record.revision.clone()))
            .collect();
        let after: Vec<(String, String)> = again
            .iter()
            .map(|record| (record.opaque_ref.clone(), record.revision.clone()))
            .collect();
        assert_eq!(before, after);
        assert!(harness.deltas.lock().unwrap().is_empty());
    }

    #[test]
    fn content_change_rolls_the_revision_without_reissuing_the_ref() {
        let harness = harness();
        let records = harness.index.refresh().expect("首扫成功");
        let target = records
            .iter()
            .find(|record| record.storage_kind == StorageKind::File)
            .expect("有文件型会话")
            .clone();
        std::fs::write(&target.canonical_ref, "changed content").unwrap();

        let updated = harness.index.refresh().expect("二次刷新");
        let after = updated
            .iter()
            .find(|record| record.canonical_ref == target.canonical_ref)
            .expect("会话仍在索引里");
        // ref 是稳定句柄，只有 revision 跟着内容走。
        assert_eq!(after.opaque_ref, target.opaque_ref);
        assert_ne!(after.revision, target.revision);

        let deltas = harness.deltas.lock().unwrap();
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0]["generation"], Value::from(1));
        let upserts = deltas[0]["upserts"].as_array().unwrap();
        assert_eq!(upserts.len(), 1);
        assert_eq!(upserts[0]["ref"], Value::from(after.opaque_ref.as_str()));
        assert_eq!(upserts[0]["revision"], Value::from(after.revision.as_str()));
        assert!(deltas[0]["removals"].as_array().unwrap().is_empty());
    }

    #[test]
    fn removed_sessions_push_a_removal_and_reuse_the_tombstoned_ref() {
        let harness = harness();
        let records = harness.index.refresh().expect("首扫成功");
        let target = records
            .iter()
            .find(|record| record.storage_kind == StorageKind::File)
            .expect("有文件型会话")
            .clone();
        let size = std::fs::metadata(&target.canonical_ref).unwrap().len() as usize;
        std::fs::remove_file(&target.canonical_ref).unwrap();

        harness.index.refresh().expect("删除后刷新");
        {
            let deltas = harness.deltas.lock().unwrap();
            assert_eq!(deltas.len(), 1);
            assert_eq!(deltas[0]["generation"], Value::from(1));
            assert_eq!(
                deltas[0]["removals"],
                Value::Array(vec![Value::from(target.opaque_ref.as_str())])
            );
        }

        // 原地重建：墓碑让它拿回同一个 ref。
        std::fs::write(&target.canonical_ref, "x".repeat(size)).unwrap();
        let restored = harness.index.refresh().expect("恢复后刷新");
        let after = restored
            .iter()
            .find(|record| record.canonical_ref == target.canonical_ref)
            .expect("恢复后重新入索引");
        assert_eq!(after.opaque_ref, target.opaque_ref);
        let deltas = harness.deltas.lock().unwrap();
        assert_eq!(deltas.len(), 2);
        assert_eq!(deltas[1]["generation"], Value::from(2));
    }

    #[test]
    fn evict_removes_in_place_and_bumps_the_generation() {
        let harness = harness();
        let records = harness.index.refresh().expect("首扫成功");
        let target = records[0].clone();
        harness.index.evict(&target.tool, &target.canonical_ref);

        assert_eq!(harness.index.generation(), 1);
        let deltas = harness.deltas.lock().unwrap();
        assert_eq!(deltas.len(), 1);
        assert_eq!(
            deltas[0]["removals"],
            Value::Array(vec![Value::from(target.opaque_ref.as_str())])
        );
        // 定点摘除后 resolve 报 unknown_ref。
        let error = harness
            .index
            .resolve(&target.tool, &target.opaque_ref, false)
            .unwrap_err();
        assert_eq!(error.params()["reason"], Value::from("unknown_ref"));
        assert_eq!(error.params()["recovery"], Value::from(REF_RECOVERY_HINT));
    }

    #[test]
    fn resolve_reports_every_invalidation_path_with_exact_params() {
        // 恢复提示不能点名 caller 专属方法：CLI 照做会撞 caller_not_allowed。
        assert!(!REF_RECOVERY_HINT.contains("session_search"));

        let harness = harness();
        let records = harness.index.refresh().expect("首扫成功");
        let file_record = records
            .iter()
            .find(|record| record.storage_kind == StorageKind::File)
            .expect("有文件型会话")
            .clone();
        let other_tool = records
            .iter()
            .find(|record| record.tool != file_record.tool)
            .expect("至少两个 agent")
            .tool
            .clone();

        // 1) 不是 Engine 签发的 ref。
        let error = harness
            .index
            .resolve(&file_record.tool, "not-a-ref", true)
            .unwrap_err();
        assert_eq!(error.code, "agent.reference_invalid");
        assert_eq!(error.message(), "ref 不是 Engine 签发的 opaque ref");
        assert!(error.params().is_empty());

        // 2) tool 配错：给出正确配对而不是让它重新搜索。params 逐字段全等——
        //    agent 只看得到这张表，多一个键少一个键都是 wire 变更。
        let error = harness
            .index
            .resolve(&other_tool, &file_record.opaque_ref, true)
            .unwrap_err();
        assert_eq!(
            Value::Object(error.params().clone()),
            json!({
                "expected_tool": file_record.tool.as_str(),
                "given_tool": other_tool.as_str(),
                "reason": "tool_mismatch",
                "recovery": format!("retry the same ref with tool={}", file_record.tool),
            })
        );

        // 3) 未知 ref。
        let error = harness
            .index
            .resolve(&file_record.tool, "fsr_0000000000000000000000", true)
            .unwrap_err();
        assert_eq!(
            Value::Object(error.params().clone()),
            json!({
                "tool": file_record.tool.as_str(),
                "reason": "unknown_ref",
                "recovery": REF_RECOVERY_HINT,
            })
        );

        // 4) 内容变了：pin_content=true 报 session_changed，false 仍放行。
        std::fs::write(&file_record.canonical_ref, "mutated").unwrap();
        let error = harness
            .index
            .resolve(&file_record.tool, &file_record.opaque_ref, true)
            .unwrap_err();
        assert_eq!(
            Value::Object(error.params().clone()),
            json!({
                "tool": file_record.tool.as_str(),
                "reason": "session_changed",
                "recovery": REF_RECOVERY_HINT,
            })
        );
        assert_eq!(error.message(), "ref 在扫描后已变化，请重新搜索");
        assert!(harness
            .index
            .resolve(&file_record.tool, &file_record.opaque_ref, false)
            .is_ok());

        // 5) 文件没了：session_missing。
        std::fs::remove_file(&file_record.canonical_ref).unwrap();
        let error = harness
            .index
            .resolve(&file_record.tool, &file_record.opaque_ref, false)
            .unwrap_err();
        assert_eq!(
            Value::Object(error.params().clone()),
            json!({
                "tool": file_record.tool.as_str(),
                "reason": "session_missing",
                "recovery": REF_RECOVERY_HINT,
            })
        );

        // 6) id 型会话的指纹消失同样是 session_missing。
        let id_record = records
            .iter()
            .find(|record| record.storage_kind == StorageKind::Id)
            .expect("有 id 型会话")
            .clone();
        harness.browsers[&id_record.tool].set_fingerprint(&id_record.canonical_ref, Value::Null);
        let error = harness
            .index
            .resolve(&id_record.tool, &id_record.opaque_ref, false)
            .unwrap_err();
        assert_eq!(error.params()["reason"], Value::from("session_missing"));
    }

    #[test]
    fn refresh_tool_only_evicts_within_its_scope() {
        let harness = harness();
        let records = harness.index.refresh().expect("首扫成功");
        let tool = records[0].tool.clone();
        let others = records.iter().filter(|record| record.tool != tool).count();
        // 清空该工具的扫描行后单工具重扫：只淘汰它自己的会话。
        harness.browsers[&tool].rows.lock().unwrap().clear();
        harness.index.refresh_tool(&tool).expect("单工具重扫");

        let (_, snapshot, generation) = harness.index.snapshot_with_status().expect("已 bootstrap");
        assert_eq!(snapshot.len(), others);
        assert_eq!(generation, 1);
        assert!(snapshot.iter().all(|record| record.tool != tool));
    }

    #[test]
    fn refresh_selected_only_scans_requested_tools_after_bootstrap() {
        let harness = harness();
        harness.index.refresh().expect("首扫成功");
        let before: BTreeMap<String, usize> = harness
            .browsers
            .iter()
            .map(|(name, browser)| {
                (
                    name.clone(),
                    browser.scans.load(std::sync::atomic::Ordering::SeqCst),
                )
            })
            .collect();
        let selected = vec!["opencode".to_string()];

        harness
            .index
            .refresh_selected(&selected)
            .expect("定向刷新成功");

        for (name, browser) in &harness.browsers {
            let delta = browser.scans.load(std::sync::atomic::Ordering::SeqCst) - before[name];
            assert_eq!(delta, usize::from(name == "opencode"), "{name}");
        }
    }

    #[test]
    fn message_locators_are_reused_and_invalidated_with_the_revision() {
        let harness = harness();
        let records = harness.index.refresh().expect("首扫成功");
        let record = records[0].clone();
        let first = harness
            .index
            .issue_message_locator(&record, "uuid-1", "user", true)
            .expect("签发 locator");
        assert!(first.starts_with("fml_"));
        // 同一 (ref, 原生定位, role) 复用同一个 fml_。
        let again = harness
            .index
            .issue_message_locator(&record, "uuid-1", "user", true)
            .expect("复用 locator");
        assert_eq!(first, again);
        assert!(harness
            .index
            .resolve_message_locator(&record, &first)
            .is_ok());

        // revision 变化 → LocatorStaleError。
        let mut stale = record.clone();
        stale.revision = "0".repeat(64);
        let error = harness
            .index
            .resolve_message_locator(&stale, &first)
            .unwrap_err();
        assert_eq!(error.code, "session.locator_stale");
        assert_eq!(error.params()["field"], Value::from("locator"));

        // 前缀不对直接判非法引用。
        let error = harness
            .index
            .resolve_message_locator(&record, "nope")
            .unwrap_err();
        assert_eq!(error.code, "agent.reference_invalid");
        // 超长原生定位符拒绝签发。
        assert!(harness
            .index
            .issue_message_locator(&record, &"x".repeat(513), "user", true)
            .is_err());
    }

    #[test]
    fn identity_shapes_match_the_documented_json_forms() {
        let harness = harness();
        let records = harness.index.refresh().expect("首扫成功");
        for record in &records {
            match record.storage_kind {
                StorageKind::File => {
                    let parts = record
                        .source_identity
                        .as_array()
                        .expect("file 身份是二元组");
                    assert_eq!(parts.len(), 2);
                    let stat = parts[0].as_array().expect("stat 五元组");
                    assert_eq!(stat.len(), 5);
                    assert_eq!(stat[4].as_str().unwrap().len(), 64);
                    assert_eq!(parts[1], Value::from("fp"));
                }
                StorageKind::Directory => {
                    let members = record.source_identity.as_array().expect("目录身份是成员表");
                    assert!(!members.is_empty());
                    let mut names: Vec<&str> = Vec::new();
                    for member in members {
                        let pair = member.as_array().expect("成员是 (rel, stat)");
                        assert_eq!(pair.len(), 2);
                        names.push(pair[0].as_str().expect("相对路径"));
                        assert_eq!(pair[1].as_array().expect("stat 五元组").len(), 5);
                    }
                    let mut sorted = names.clone();
                    sorted.sort_unstable();
                    assert_eq!(names, sorted, "目录成员必须按相对路径排序");
                }
                StorageKind::Id => {
                    assert_eq!(record.source_identity, Value::from("fp"));
                }
            }
        }
    }

    /// §2.5：全量刷新单飞合并——后到者等先行者并复用其结果，adapter 只被扫一遍。
    #[test]
    fn concurrent_refreshes_collapse_into_a_single_flight() {
        let harness = harness();
        for browser in harness.browsers.values() {
            browser
                .slow
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        let index = harness.index.clone();
        let handles: Vec<_> = (0..6)
            .map(|_| {
                let index = index.clone();
                std::thread::spawn(move || {
                    index
                        .refresh()
                        .expect("并发刷新成功")
                        .iter()
                        .map(|record| (record.opaque_ref.clone(), record.revision.clone()))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let results: Vec<Vec<(String, String)>> = handles
            .into_iter()
            .map(|handle| handle.join().expect("线程无 panic"))
            .collect();

        // 六个调用者拿到同一份结果。
        for result in &results[1..] {
            assert_eq!(result, &results[0]);
        }
        // 每个 adapter 至多被扫一遍：合并成功。后到者若错过飞行会再起一轮，
        // 因此只断言上界不是恒等（调度上仍允许串行发生）。
        for (agent, browser) in &harness.browsers {
            let scans = browser.scans.load(std::sync::atomic::Ordering::SeqCst);
            assert!(scans >= 1, "{agent} 至少扫一遍");
            assert!(scans < 6, "{agent} 扫了 {scans} 遍，单飞没有合并");
        }
        // 首扫不推增量，哪怕是并发进来的。
        assert!(harness.deltas.lock().unwrap().is_empty());
    }

    /// §2.5/§29：会话正被 CLI 持续追加时，这一轮拍不出稳定快照也**不能**把它
    /// 判成已删除——`_IdentityRaceError` 路径沿用上一轮记录。
    #[test]
    fn a_session_being_appended_is_never_evicted() {
        let harness = harness();
        let records = harness.index.refresh().expect("首扫成功");
        let target = records
            .iter()
            .find(|record| record.storage_kind == StorageKind::File)
            .expect("有文件型会话")
            .clone();

        let path = target.canonical_ref.clone();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let writer_stop = stop.clone();
        let writer = std::thread::spawn(move || {
            use std::io::Write as _;
            // 上限只是防止刷新意外变慢时把临时文件写爆，与断言无关。
            let mut round = 0u64;
            while round < 20_000 && !writer_stop.load(std::sync::atomic::Ordering::SeqCst) {
                if let Ok(mut file) = std::fs::OpenOptions::new().append(true).open(&path) {
                    let _ = writeln!(file, "追加第 {round} 行");
                    let _ = file.flush();
                }
                round += 1;
            }
        });
        // 追加期间连扫数轮：无论是否真的撞上竞态，这个会话都必须始终在索引里。
        for _ in 0..5 {
            let round = harness.index.refresh().expect("追加期间刷新成功");
            assert!(
                round
                    .iter()
                    .any(|record| record.opaque_ref == target.opaque_ref),
                "活跃会话被误判为已删除"
            );
        }
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        writer.join().expect("写线程无 panic");

        // 安静下来后收敛：ref 不变，revision 跟上最新内容。
        let settled = harness.index.refresh().expect("收敛刷新");
        let after = settled
            .iter()
            .find(|record| record.opaque_ref == target.opaque_ref)
            .expect("仍在索引里");
        assert_eq!(after.canonical_ref, target.canonical_ref);
        assert_ne!(after.revision, target.revision);
        // 一次 removal 都不该推。
        for delta in harness.deltas.lock().unwrap().iter() {
            assert!(
                delta["removals"]
                    .as_array()
                    .expect("removals 是数组")
                    .is_empty(),
                "追加期间推了 removal：{delta}"
            );
        }
    }

    #[test]
    fn scan_snapshot_is_sorted_and_carries_the_generation() {
        let harness = harness();
        let payload = super::super::scan::scan(&harness.index, None).expect("首次 scan");
        let sessions = payload["sessions"].as_array().expect("sessions 是数组");
        assert_eq!(sessions.len(), 13);
        assert_eq!(payload["generation"], Value::from(0));
        let updated: Vec<i64> = sessions
            .iter()
            .map(|session| session["updated"].as_i64().unwrap_or(0))
            .collect();
        let mut sorted = updated.clone();
        sorted.sort_unstable_by(|left, right| right.cmp(left));
        assert_eq!(updated, sorted);
        // 每个工具都有扫描状态。
        assert_eq!(payload["tools"].as_object().unwrap().len(), 5);
        for status in payload["tools"].as_object().unwrap().values() {
            assert_eq!(status["ok"], Value::Bool(true));
        }
    }
}
