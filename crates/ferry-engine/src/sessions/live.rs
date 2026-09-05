//! 常驻活索引：原生文件事件 + 数据库轻量探针 + 按工具增量重扫。
//!
//! 文件型 adapter 由 `notify::RecommendedWatcher` 递归监听，不在空闲时遍历会话树；
//! 提供 `watch_stamp()` 的数据库型 adapter 仍按固定间隔轮询廉价令牌。文件事件与
//! 令牌变化都按工具去抖，持续写入最多欠账 [`MAX_PENDING`]。全量对账只用于显式
//! nudge，或 watcher 安装失败、溢出和运行期错误后的兜底。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::Value;

use crate::server::serve::log_error;
use crate::system::paths::expanduser;

use super::index::AgentSessionIndex;

pub const POLL_INTERVAL: Duration = Duration::from_millis(2_500);
pub const RECONCILE_INTERVAL: Duration = Duration::from_secs(300);
/// 手动刷新（nudge）触发的全量对账最小间隔：防止 UI 连点造成扫描风暴。
pub const NUDGE_MIN_GAP: Duration = Duration::from_secs(5);
/// 高频写入源持续变化时，最多欠账这么久就强制增量重扫一次。
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

/// 廉价令牌轮询状态；只包含实现了 `watch_stamp()` 的 adapter。
#[derive(Default)]
struct ProbeState {
    tokens: HashMap<String, Value>,
    synced: HashMap<String, Value>,
    pending_since: HashMap<String, Instant>,
}

impl ProbeState {
    fn changed_tools(
        &mut self,
        index: &AgentSessionIndex,
        names: &[String],
        max_pending: Duration,
        now: Instant,
    ) -> Vec<String> {
        let mut changed = Vec::new();
        for name in names {
            let Some(token) = probe_stamp(index, name) else {
                continue;
            };
            let previous = self.tokens.insert(name.clone(), token.clone());
            let Some(previous) = previous else {
                self.synced.insert(name.clone(), token);
                continue;
            };
            if self.synced.get(name) == Some(&token) {
                self.pending_since.remove(name);
                continue;
            }
            let started = *self.pending_since.entry(name.clone()).or_insert(now);
            if token != previous && now.duration_since(started) < max_pending {
                continue;
            }
            changed.push(name.clone());
            self.synced.insert(name.clone(), token);
            self.pending_since.remove(name);
        }
        changed
    }

    fn mark_reconciled(&mut self) {
        for (name, token) in &self.tokens {
            self.synced.insert(name.clone(), token.clone());
        }
        self.pending_since.clear();
    }
}

fn probe_stamp(index: &AgentSessionIndex, name: &str) -> Option<Value> {
    let adapter = index.ports().adapter(name).ok()?;
    let browser = adapter.browser.as_deref()?;
    let stamp = browser.watch_stamp()?;
    match stamp {
        Ok(value) => Some(Value::Array(vec![Value::from("adapter"), value])),
        Err(error) => {
            log_error(&format!(
                "watch_stamp 探测失败: {name}: {}",
                error.message()
            ));
            None
        }
    }
}

#[derive(Clone, Copy)]
struct EventWindow {
    first: Instant,
    last: Instant,
}

#[derive(Default)]
struct WakeState {
    woken: bool,
    stop: bool,
    nudged: bool,
    ready: bool,
    watch_fault: bool,
    file_events: HashMap<String, EventWindow>,
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

    fn mark_ready(&self) {
        self.set(|state| state.ready = true);
    }

    fn note_file_event(&self, name: &str) {
        let now = Instant::now();
        self.set(|state| match state.file_events.get_mut(name) {
            Some(window) => window.last = now,
            None => {
                state.file_events.insert(
                    name.to_string(),
                    EventWindow {
                        first: now,
                        last: now,
                    },
                );
            }
        });
    }

    fn note_watch_fault(&self) {
        self.set(|state| state.watch_fault = true);
    }

    fn take_watch_fault(&self) -> bool {
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::mem::take(&mut guard.watch_fault)
    }

    fn ready_file_events(
        &self,
        now: Instant,
        debounce: Duration,
        max_pending: Duration,
    ) -> Vec<String> {
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let ready: Vec<String> = guard
            .file_events
            .iter()
            .filter(|(_, window)| {
                now.duration_since(window.last) >= debounce
                    || now.duration_since(window.first) >= max_pending
            })
            .map(|(name, _)| name.clone())
            .collect();
        for name in &ready {
            guard.file_events.remove(name);
        }
        ready
    }

    fn next_file_wait(
        &self,
        now: Instant,
        debounce: Duration,
        max_pending: Duration,
    ) -> Option<Duration> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .file_events
            .values()
            .map(|window| {
                let quiet = window.last + debounce;
                let forced = window.first + max_pending;
                quiet.min(forced).saturating_duration_since(now)
            })
            .min()
    }

    fn clear_file_events_through(&self, cutoff: Instant) {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .file_events
            .retain(|_, window| window.last > cutoff);
    }

    #[cfg(test)]
    fn wait_until_ready(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while !guard.ready {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let (next, _) = self
                .signal
                .wait_timeout(guard, deadline.saturating_duration_since(now))
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard = next;
        }
        true
    }
}

#[derive(Clone)]
struct WatchTarget {
    name: String,
    path: PathBuf,
    installed: bool,
    install_failed: bool,
}

struct FileWatchers {
    watcher: Option<RecommendedWatcher>,
    backend_failed: bool,
    targets: Vec<WatchTarget>,
    routes: Arc<Vec<(String, PathBuf)>>,
    shared: Arc<Shared>,
}

impl FileWatchers {
    fn new(targets: Vec<WatchTarget>, shared: Arc<Shared>) -> Self {
        let routes = Arc::new(
            targets
                .iter()
                .map(|target| (target.name.clone(), target.path.clone()))
                .collect(),
        );
        let watcher = if targets.is_empty() {
            None
        } else {
            create_watcher(Arc::clone(&routes), Arc::clone(&shared))
        };
        let backend_failed = watcher.is_none() && !targets.is_empty();
        let mut this = Self {
            watcher,
            backend_failed,
            targets,
            routes,
            shared,
        };
        this.install_available(false, true);
        this
    }

    fn install_available(&mut self, changed: bool, retry_failed: bool) {
        let Some(watcher) = self.watcher.as_mut() else {
            return;
        };
        for target in &mut self.targets {
            if target.installed || (target.install_failed && !retry_failed) || !target.path.is_dir()
            {
                continue;
            }
            match watcher.watch(&target.path, RecursiveMode::Recursive) {
                Ok(()) => {
                    target.installed = true;
                    target.install_failed = false;
                    if changed {
                        self.shared.note_file_event(&target.name);
                    }
                }
                Err(error) => {
                    target.install_failed = true;
                    log_error(&format!(
                        "文件 watcher 安装失败: {} {}: {error}",
                        target.name,
                        target.path.display()
                    ));
                    self.shared.note_watch_fault();
                }
            }
        }
    }

    fn maintain(&mut self) {
        if let Some(watcher) = self.watcher.as_mut() {
            for target in &mut self.targets {
                if target.installed && !target.path.is_dir() {
                    let _ = watcher.unwatch(&target.path);
                    target.installed = false;
                    target.install_failed = false;
                    self.shared.note_file_event(&target.name);
                }
            }
        }
        // 缺失目录只做一次 is_dir；真正安装失败的目标等 300s 兜底时再重试，
        // 避免每 2.5s 重复系统调用和错误日志。
        self.install_available(true, false);
    }

    fn retry_backend(&mut self, recreate: bool) {
        if recreate {
            self.watcher = None;
            for target in &mut self.targets {
                target.installed = false;
                target.install_failed = false;
            }
        }
        if self.watcher.is_none() && !self.targets.is_empty() {
            self.watcher = create_watcher(Arc::clone(&self.routes), Arc::clone(&self.shared));
            self.backend_failed = self.watcher.is_none();
        }
        self.install_available(false, true);
    }

    fn failed(&self) -> bool {
        self.backend_failed || self.targets.iter().any(|target| target.install_failed)
    }
}

fn create_watcher(
    routes: Arc<Vec<(String, PathBuf)>>,
    shared: Arc<Shared>,
) -> Option<RecommendedWatcher> {
    notify::recommended_watcher(move |result: notify::Result<Event>| match result {
        Ok(event) if event.need_rescan() => shared.note_watch_fault(),
        Ok(event) if !event.kind.is_access() => {
            for (name, root) in routes.iter() {
                if event.paths.is_empty()
                    || event
                        .paths
                        .iter()
                        .any(|path| path.starts_with(root) || root.starts_with(path))
                {
                    shared.note_file_event(name);
                }
            }
        }
        Ok(_) => {}
        Err(error) => {
            log_error(&format!("文件 watcher 运行失败: {error}"));
            shared.note_watch_fault();
        }
    })
    .map_err(|error| log_error(&format!("文件 watcher 创建失败: {error}")))
    .ok()
}

fn adapter_modes(index: &AgentSessionIndex) -> (Vec<String>, Vec<WatchTarget>) {
    let mut polled = Vec::new();
    let mut watched = Vec::new();
    for name in index.ports().adapters() {
        let Ok(adapter) = index.ports().adapter(&name) else {
            continue;
        };
        let Some(browser) = adapter.browser.as_deref() else {
            continue;
        };
        if browser.watch_stamp().is_some() {
            polled.push(name);
            continue;
        }
        if !adapter.manifest.source_path.is_empty() {
            let expanded = expanduser(&adapter.manifest.source_path);
            watched.push(WatchTarget {
                name,
                path: std::fs::canonicalize(&expanded).unwrap_or(expanded),
                installed: false,
                install_failed: false,
            });
        }
    }
    (polled, watched)
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
        self.join();
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

impl Drop for LiveIndexService {
    fn drop(&mut self) {
        self.shared.set(|state| state.stop = true);
        if let Some(handle) = self
            .thread
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            let _ = handle.join();
        }
    }
}

fn refresh_all(index: &AgentSessionIndex, probes: &mut ProbeState, shared: &Shared) -> bool {
    let started = Instant::now();
    match index.refresh_with_status() {
        Ok(_) => {
            probes.mark_reconciled();
            shared.clear_file_events_through(started);
            true
        }
        Err(error) => {
            log_error(&format!("全量对账失败: {}", error.message()));
            false
        }
    }
}

fn refresh_tools(index: &AgentSessionIndex, names: impl IntoIterator<Item = String>) {
    for name in names {
        if let Err(error) = index.refresh_tool(&name) {
            log_error(&format!("增量重扫失败: {name}: {}", error.message()));
        }
    }
}

fn run(index: Arc<AgentSessionIndex>, shared: Arc<Shared>, config: LiveConfig) {
    let (polled_names, watch_targets) = adapter_modes(&index);
    let mut watchers = FileWatchers::new(watch_targets, Arc::clone(&shared));
    let mut probes = ProbeState::default();
    let started = Instant::now();
    let mut next_poll = started;
    let mut last_full_attempt = started;
    let mut nudge_pending = false;
    let mut watch_fault_pending = false;
    shared.mark_ready();

    while !shared.stopping() {
        let now = Instant::now();
        let poll_wait = next_poll.saturating_duration_since(now);
        let event_wait = shared
            .next_file_wait(now, config.poll_interval, config.max_pending)
            .unwrap_or(poll_wait);
        shared.wait(poll_wait.min(event_wait));
        if shared.stopping() {
            return;
        }

        let now = Instant::now();
        nudge_pending |= shared.take_nudge();
        watch_fault_pending |= shared.take_watch_fault();
        let snapshot_ready = index.snapshot_with_status().is_some();
        let mut polled_changes = Vec::new();
        if now >= next_poll {
            watchers.maintain();
            if snapshot_ready {
                polled_changes =
                    probes.changed_tools(&index, &polled_names, config.max_pending, now);
            }
            next_poll = now + config.poll_interval;
        }
        if !snapshot_ready {
            continue;
        }

        let full_due_to_nudge =
            nudge_pending && now.duration_since(last_full_attempt) >= config.nudge_min_gap;
        let full_due_to_fallback = (watch_fault_pending || watchers.failed())
            && now.duration_since(last_full_attempt) >= config.reconcile_interval;
        let mut refreshed_all = false;
        if full_due_to_nudge || full_due_to_fallback {
            if full_due_to_fallback {
                watchers.retry_backend(watch_fault_pending);
            }
            refreshed_all = refresh_all(&index, &mut probes, &shared);
            last_full_attempt = Instant::now();
            if refreshed_all {
                nudge_pending = false;
                watch_fault_pending = false;
                polled_changes.clear();
            }
        }

        if !refreshed_all {
            refresh_tools(&index, polled_changes);
        }
        let file_changes =
            shared.ready_file_events(Instant::now(), config.poll_interval, config.max_pending);
        refresh_tools(&index, file_changes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::contracts::{
        AgentAdapter, AgentManifest, Fingerprint, NativeSessionReference, ScanCache, ScanRow,
        SessionBrowser,
    };
    use crate::errors::{DomainError, DomainResult};
    use crate::model::Session;
    use crate::sessions::index::SessionPorts;
    use std::collections::BTreeMap;
    use std::fs::OpenOptions;
    use std::io::Write as _;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct NullScanCache;

    impl ScanCache for NullScanCache {
        fn get(&self, _path: &Path, _stat: &crate::jsonutil::FileStat) -> Option<Option<ScanRow>> {
            None
        }

        fn put(&self, _path: &Path, _stat: &crate::jsonutil::FileStat, _meta: Option<ScanRow>) {}

        fn get_digest(&self, _path: &Path, _stat: &crate::jsonutil::FileStat) -> Option<String> {
            None
        }

        fn put_digest(&self, _path: &Path, _stat: &crate::jsonutil::FileStat, _digest: &str) {}

        fn flush(&self) {}
    }

    #[test]
    fn file_events_are_debounced_per_tool_and_forced_at_the_cap() {
        let shared = Shared {
            state: Mutex::new(WakeState::default()),
            signal: Condvar::new(),
        };
        shared.note_file_event("alpha");
        std::thread::sleep(Duration::from_millis(5));
        shared.note_file_event("alpha");
        shared.note_file_event("beta");
        assert!(shared
            .ready_file_events(
                Instant::now(),
                Duration::from_secs(1),
                Duration::from_secs(2)
            )
            .is_empty());
        let ready = shared.ready_file_events(
            Instant::now() + Duration::from_secs(3),
            Duration::from_secs(1),
            Duration::from_secs(2),
        );
        assert_eq!(ready.len(), 2);
        assert!(ready.contains(&"alpha".to_string()));
        assert!(ready.contains(&"beta".to_string()));
    }

    #[test]
    fn missing_watch_root_installs_when_it_appears() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("later");
        let shared = Arc::new(Shared {
            state: Mutex::new(WakeState::default()),
            signal: Condvar::new(),
        });
        let mut watchers = FileWatchers::new(
            vec![WatchTarget {
                name: "alpha".into(),
                path: missing.clone(),
                installed: false,
                install_failed: false,
            }],
            Arc::clone(&shared),
        );
        assert!(!watchers.failed());
        std::fs::create_dir(&missing).unwrap();
        watchers.maintain();
        assert!(!watchers.failed());
        let ready = shared.ready_file_events(
            Instant::now() + Duration::from_secs(1),
            Duration::from_millis(10),
            Duration::from_secs(1),
        );
        assert_eq!(ready, vec!["alpha".to_string()]);
    }

    struct FakeBrowser {
        scans: AtomicUsize,
    }

    impl SessionBrowser for FakeBrowser {
        fn scan(&self, _cache: &dyn ScanCache) -> DomainResult<Vec<ScanRow>> {
            self.scans.fetch_add(1, Ordering::SeqCst);
            Ok(Vec::new())
        }

        fn read(&self, reference: &str) -> DomainResult<Session> {
            Ok(Session::new("fake", reference, ""))
        }

        fn resolve_ref(&self, reference: &str) -> DomainResult<String> {
            Ok(reference.to_string())
        }

        fn fingerprint(&self, _reference: &str) -> DomainResult<Fingerprint> {
            Ok(Value::Null)
        }

        fn agent_fingerprint(&self, reference: &str) -> DomainResult<Fingerprint> {
            self.fingerprint(reference)
        }

        fn canonicalize(&self, _row: &ScanRow) -> Option<NativeSessionReference> {
            None
        }

        fn validate_read_scope(&self, _reference: &NativeSessionReference) -> DomainResult<()> {
            Ok(())
        }
    }

    struct FakePorts {
        adapters: BTreeMap<String, AgentAdapter>,
        order: Vec<String>,
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
            Arc::new(NullScanCache)
        }
    }

    struct Harness {
        _temp: tempfile::TempDir,
        roots: BTreeMap<String, PathBuf>,
        browsers: BTreeMap<String, Arc<FakeBrowser>>,
        index: Arc<AgentSessionIndex>,
    }

    impl Harness {
        fn new(names: &[&str]) -> Self {
            let temp = tempfile::tempdir().unwrap();
            let mut adapters = BTreeMap::new();
            let mut roots = BTreeMap::new();
            let mut browsers = BTreeMap::new();
            let mut order = Vec::new();
            for name in names {
                let root = temp.path().join(name);
                std::fs::create_dir_all(&root).unwrap();
                let browser = Arc::new(FakeBrowser {
                    scans: AtomicUsize::new(0),
                });
                let manifest = AgentManifest {
                    id: (*name).to_string(),
                    display_name: (*name).to_string(),
                    icon: (*name).to_string(),
                    source_path: root.to_string_lossy().into_owned(),
                    capabilities: vec!["browse".into()],
                    edit_operations: Vec::new(),
                    executables: Vec::new(),
                    fallback_bin_dirs: Vec::new(),
                };
                let adapter = AgentAdapter::builder()
                    .browser(browser.clone() as Arc<dyn SessionBrowser>)
                    .build(manifest)
                    .unwrap();
                order.push((*name).to_string());
                roots.insert((*name).to_string(), root);
                browsers.insert((*name).to_string(), browser);
                adapters.insert((*name).to_string(), adapter);
            }
            let index = Arc::new(AgentSessionIndex::new(Arc::new(FakePorts {
                adapters,
                order,
            })));
            index.refresh().unwrap();
            for browser in browsers.values() {
                browser.scans.store(0, Ordering::SeqCst);
            }
            Self {
                _temp: temp,
                roots,
                browsers,
                index,
            }
        }
    }

    fn wait_for(timeout: Duration, predicate: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if predicate() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        predicate()
    }

    #[test]
    fn recommended_watcher_is_silent_and_merges_file_mutations_by_tool() {
        let harness = Harness::new(&["alpha", "beta"]);
        let alpha_root = &harness.roots["alpha"];
        // 缺失目录不是 watcher 故障；即使跨过 reconcile_interval 也不能全扫。
        std::fs::remove_dir(&harness.roots["beta"]).unwrap();
        let original = alpha_root.join("original.jsonl");
        std::fs::write(&original, b"one\n").unwrap();
        let config = LiveConfig {
            poll_interval: Duration::from_millis(500),
            reconcile_interval: Duration::from_millis(700),
            nudge_min_gap: Duration::from_millis(100),
            max_pending: Duration::from_secs(2),
        };
        let service = LiveIndexService::with_config(Arc::clone(&harness.index), config);
        service.start();
        assert!(service.shared.wait_until_ready(Duration::from_secs(3)));

        // 健康 watcher 即便跨过 reconcile_interval 也不得触发全量扫描。
        // macOS FSEvents 可能在 watcher 安装后补发注册前的目录事件，因此 alpha
        // 可以有一次启动期增量扫描；缺失的 beta 仍能证明没有发生全量扫描。
        std::thread::sleep(Duration::from_millis(900));
        let alpha_baseline = harness.browsers["alpha"].scans.load(Ordering::SeqCst);
        assert_eq!(harness.browsers["beta"].scans.load(Ordering::SeqCst), 0);

        OpenOptions::new()
            .append(true)
            .open(&original)
            .unwrap()
            .write_all(b"two\n")
            .unwrap();
        let created = alpha_root.join("created.jsonl");
        std::fs::write(&created, b"created\n").unwrap();
        let renamed = alpha_root.join("renamed.jsonl");
        std::fs::rename(&created, &renamed).unwrap();
        std::fs::remove_file(&renamed).unwrap();

        assert!(wait_for(Duration::from_secs(8), || {
            harness.browsers["alpha"].scans.load(Ordering::SeqCst) > alpha_baseline
        }));
        std::thread::sleep(Duration::from_millis(700));
        assert_eq!(
            harness.browsers["alpha"].scans.load(Ordering::SeqCst),
            alpha_baseline + 1
        );
        assert_eq!(harness.browsers["beta"].scans.load(Ordering::SeqCst), 0);

        service.stop();
        std::fs::write(alpha_root.join("after-stop.jsonl"), b"ignored\n").unwrap();
        std::thread::sleep(Duration::from_millis(700));
        assert_eq!(
            harness.browsers["alpha"].scans.load(Ordering::SeqCst),
            alpha_baseline + 1
        );
        assert!(Path::new(alpha_root).is_dir());
    }
}
