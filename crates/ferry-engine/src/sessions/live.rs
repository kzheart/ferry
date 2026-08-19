//! 常驻活索引：文件系统轮询探测 + 增量重扫 + 周期全量对账。
//!
//! 引擎不再等 UI 拉取才知道世界变了：后台线程每个周期对各 adapter 的会话存储
//! 做一轮廉价的 stat 扫描，源头一变就只重扫那个工具，delta 经
//! `AgentSessionIndex::set_on_delta` 推给前端。周期性的全量对账兜住轮询窗口内
//! 可能漏掉的变化（睡眠恢复、外置卷、探测失败）。
//!
//! adapter 可选提供 `browser.watch_stamp()` 返回廉价变更令牌；未提供时默认对
//! `manifest.source_path` 做全树 scandir 扫描。新增 agent 零成本获得实时能力。

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::server::serve::log_error;
use crate::system::paths::{expanduser, realpath_strict};

use super::index::AgentSessionIndex;

pub const POLL_INTERVAL: Duration = Duration::from_millis(2_500);
pub const RECONCILE_INTERVAL: Duration = Duration::from_secs(300);
/// 手动刷新（nudge）触发的全量对账最小间隔：防止 UI 连点造成扫描风暴。
pub const NUDGE_MIN_GAP: Duration = Duration::from_secs(5);
/// 高频写入源的令牌每轮都在变，逐轮全库重扫是纯浪费；变更落定（连续两轮令牌
/// 相同）才重扫，持续变动按此上限兜底。
pub const MAX_PENDING: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug)]
pub struct LiveConfig {
    pub poll_interval: Duration,
    pub reconcile_interval: Duration,
    pub nudge_min_gap: Duration,
    pub max_pending: Duration,
}

impl Default for LiveConfig {
    fn default() -> Self {
        Self {
            poll_interval: POLL_INTERVAL,
            reconcile_interval: RECONCILE_INTERVAL,
            nudge_min_gap: NUDGE_MIN_GAP,
            max_pending: MAX_PENDING,
        }
    }
}

/// 目录树的变更令牌：全部文件的 `(路径, mtime_ns, size)` 排序后逐行哈希。
///
/// 逐字对齐 `live.py:31-62`：行格式是 `"{path}\0{mtime_ns}\0{size}"`，
/// 每行后追加一个 `\n` 再喂给 sha256。
pub fn tree_stamp(root: &str) -> Option<String> {
    let base = realpath_strict(&expanduser(root)).ok()?;
    let mut entries: Vec<String> = Vec::new();
    let mut stack: Vec<PathBuf> = vec![base];
    while let Some(directory) = stack.pop() {
        let Ok(iterator) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in iterator.flatten() {
            let path = entry.path();
            // follow_symlinks=False：符号链接目录不递归，按普通条目 lstat。
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if metadata.is_dir() {
                stack.push(path);
                continue;
            }
            let stat = crate::jsonutil::FileStat::from_metadata(&metadata);
            entries.push(format!(
                "{}\u{0}{}\u{0}{}",
                path.to_string_lossy(),
                stat.mtime_ns,
                stat.size
            ));
        }
    }
    entries.sort_unstable();
    let mut digest = Sha256::new();
    for line in &entries {
        digest.update(line.as_bytes());
        digest.update(b"\n");
    }
    let mut hex = String::with_capacity(64);
    for byte in digest.finalize() {
        let _ = write!(hex, "{byte:02x}");
    }
    Some(hex)
}

/// 轮询侧的可变状态；只被工作线程持有，单元测试可直接驱动。
#[derive(Default)]
pub struct ProbeState {
    tokens: HashMap<String, Value>,
    synced: HashMap<String, Value>,
    pending_since: HashMap<String, Instant>,
}

impl ProbeState {
    /// 返回本轮需要增量重扫的工具名。
    pub fn changed_tools(
        &mut self,
        index: &AgentSessionIndex,
        max_pending: Duration,
        now: Instant,
    ) -> Vec<String> {
        let mut changed = Vec::new();
        for name in index.ports().adapters() {
            let Some(token) = probe(index, &name) else {
                continue;
            };
            let previous = self.tokens.insert(name.clone(), token.clone());
            // 首次观测只记基线：快照刚由全量扫描建立，无需重扫。
            let Some(previous) = previous else {
                self.synced.insert(name, token);
                continue;
            };
            if self.synced.get(&name) == Some(&token) {
                self.pending_since.remove(&name);
                continue;
            }
            let started = *self.pending_since.entry(name.clone()).or_insert(now);
            // 变更落定（连续两轮令牌相同）才重扫；源头持续被写入时，最多欠账
            // max_pending 就强制重扫一次兜底。
            if token != previous && now.duration_since(started) < max_pending {
                continue;
            }
            changed.push(name.clone());
            self.synced.insert(name.clone(), token);
            self.pending_since.remove(&name);
        }
        changed
    }

    /// 对账已覆盖到此刻的世界：把观测令牌记为已同步。
    pub fn mark_reconciled(&mut self) {
        for (name, token) in &self.tokens {
            self.synced.insert(name.clone(), token.clone());
        }
        self.pending_since.clear();
    }
}

fn probe(index: &AgentSessionIndex, name: &str) -> Option<Value> {
    let adapter = index.ports().adapter(name).ok()?;
    let browser = adapter.browser.as_deref()?;
    if let Some(stamp) = browser.watch_stamp() {
        // 探测失败按不可知处理（Python `live.py:179-180` 打 log.exception：
        // 轮询悄悄不干活时，这行日志是唯一线索）。
        return match stamp {
            Ok(value) => Some(Value::Array(vec![Value::from("adapter"), value])),
            Err(error) => {
                log_error(&format!(
                    "watch_stamp 探测失败: {name}: {}",
                    error.message()
                ));
                None
            }
        };
    }
    let source = adapter.manifest.source_path.as_str();
    if source.is_empty() {
        return None;
    }
    tree_stamp(source).map(Value::from)
}

#[derive(Default)]
struct WakeState {
    woken: bool,
    stop: bool,
    nudged: bool,
}

struct Shared {
    state: Mutex<WakeState>,
    signal: Condvar,
}

impl Shared {
    fn wait(&self, timeout: Duration) {
        let guard = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (mut guard, _) = self
            .signal
            .wait_timeout_while(guard, timeout, |state| !state.woken && !state.stop)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.woken = false;
    }

    fn set(&self, mutate: impl FnOnce(&mut WakeState)) {
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        mutate(&mut guard);
        guard.woken = true;
        self.signal.notify_all();
    }

    fn stopping(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .stop
    }

    fn take_nudge(&self) -> bool {
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::mem::take(&mut guard.nudged)
    }
}

pub struct LiveIndexService {
    index: Arc<AgentSessionIndex>,
    config: LiveConfig,
    shared: Arc<Shared>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl LiveIndexService {
    pub fn new(index: Arc<AgentSessionIndex>) -> Self {
        Self::with_config(index, LiveConfig::default())
    }

    pub fn with_config(index: Arc<AgentSessionIndex>, config: LiveConfig) -> Self {
        Self {
            index,
            config,
            shared: Arc::new(Shared {
                state: Mutex::new(WakeState::default()),
                signal: Condvar::new(),
            }),
            thread: Mutex::new(None),
        }
    }

    pub fn start(&self) {
        let mut slot = self
            .thread
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if slot.is_some() {
            return;
        }
        let index = self.index.clone();
        let shared = self.shared.clone();
        let config = self.config;
        *slot = std::thread::Builder::new()
            .name("live-index".into())
            .spawn(move || run(index, shared, config))
            .ok();
    }

    pub fn stop(&self) {
        self.shared.set(|state| state.stop = true);
    }

    /// UI 手动刷新的逃生口：立即轮询一轮并（限频地）全量对账。
    pub fn nudge(&self) {
        self.shared.set(|state| state.nudged = true);
    }

    /// 等待工作线程退出（测试与优雅关停用）。
    pub fn join(&self) {
        let handle = self
            .thread
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(handle) = handle {
            let _ = handle.join();
        }
    }
}

fn run(index: Arc<AgentSessionIndex>, shared: Arc<Shared>, config: LiveConfig) {
    let mut probes = ProbeState::default();
    let mut last_reconcile = Instant::now();
    while !shared.stopping() {
        shared.wait(config.poll_interval);
        if shared.stopping() {
            return;
        }
        let nudged = shared.take_nudge();
        // 首次全量扫描（启动预热）完成前没有可增量的基线。
        if index.snapshot_with_status().is_none() {
            continue;
        }
        for name in probes.changed_tools(&index, config.max_pending, Instant::now()) {
            if shared.stopping() {
                return;
            }
            // 单工具失败不影响轮询。
            if let Err(error) = index.refresh_tool(&name) {
                log_error(&format!("增量重扫失败: {name}: {}", error.message()));
            }
        }
        let now = Instant::now();
        let elapsed = now.duration_since(last_reconcile);
        if elapsed >= config.reconcile_interval || (nudged && elapsed >= config.nudge_min_gap) {
            // 对账失败等下一轮。
            match index.refresh_with_status() {
                // 对账已覆盖到此刻的世界：把观测令牌记为已同步，避免紧接着的
                // 轮询对同一批变化再重扫一遍。
                Ok(_) => probes.mark_reconciled(),
                Err(error) => log_error(&format!("周期对账失败: {}", error.message())),
            }
            last_reconcile = Instant::now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intervals_match_the_python_constants() {
        assert_eq!(POLL_INTERVAL.as_secs_f64(), 2.5);
        assert_eq!(RECONCILE_INTERVAL.as_secs_f64(), 300.0);
        assert_eq!(NUDGE_MIN_GAP.as_secs_f64(), 5.0);
        assert_eq!(MAX_PENDING.as_secs_f64(), 15.0);
    }

    #[test]
    fn tree_stamp_tracks_content_and_ignores_ordering() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_string_lossy().into_owned();
        std::fs::create_dir(temp.path().join("nested")).unwrap();
        std::fs::write(temp.path().join("nested/a.jsonl"), b"a").unwrap();
        std::fs::write(temp.path().join("b.jsonl"), b"bb").unwrap();
        let first = tree_stamp(&root).unwrap();
        assert_eq!(first.len(), 64);
        assert_eq!(tree_stamp(&root).unwrap(), first);
        // 大小变化 → 令牌变化。
        std::fs::write(temp.path().join("b.jsonl"), b"bbb").unwrap();
        assert_ne!(tree_stamp(&root).unwrap(), first);
        assert_eq!(tree_stamp("/definitely/not/here"), None);
    }

    /// 令牌行的字节形状是稳定面：形状一变，所有已缓存的令牌都会被判定成
    /// “世界变了”，触发一次无谓的全量重扫。
    #[test]
    fn tree_stamp_line_format_is_nul_separated() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("a.bin");
        std::fs::write(&file, b"xyz").unwrap();
        let base = realpath_strict(temp.path()).unwrap();
        let stat =
            crate::jsonutil::FileStat::from_metadata(&std::fs::symlink_metadata(&file).unwrap());
        let line = format!(
            "{}\u{0}{}\u{0}{}",
            base.join("a.bin").to_string_lossy(),
            stat.mtime_ns,
            stat.size
        );
        let mut digest = Sha256::new();
        digest.update(line.as_bytes());
        digest.update(b"\n");
        let mut expected = String::new();
        for byte in digest.finalize() {
            let _ = write!(expected, "{byte:02x}");
        }
        assert_eq!(
            tree_stamp(&temp.path().to_string_lossy()).unwrap(),
            expected
        );
    }
}
