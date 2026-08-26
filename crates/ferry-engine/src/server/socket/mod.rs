//! 本地 socket 传输：`ferry-ipc/1` 的第二条通道。
//!
//! 与 stdio 的差异只有三条，能力模块零改动：
//!
//! 1. **callers 过滤**：只分发 `CLI_METHOD_NAMES`（契约生成物）里的方法，其余
//!    按「这条通道上没有这个方法」拒绝。`runtime_sessions.*` / `agent_prompt`
//!    永不经 socket 暴露。
//! 2. **管理方法拦截**：`daemon.status` / `daemon.shutdown` 在**分发之前**由
//!    传输层处理，不进方法表——它们是传输的性质，不是引擎能力。App 模式下
//!    `daemon.shutdown` 结构化拒绝：App 的引擎不能被 CLI 杀。
//! 3. **不订阅事件**：CLI 是请求-响应式调用方，notifier 只接 stdio。代价是
//!    socket 侧拿不到 `sessions.changed` 增量，CLI 需要状态就显式查。
//!
//! 工作道与 stdio 共用同一组（见 [`crate::server::serve::Lanes`]），所以「读
//! 并行、mutation 串行」是进程级不变量，不会因为多开连接而失效。

pub mod idle;
pub mod lock;
pub mod platform;

use std::io::BufReader;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

use crate::contracts::engine_methods::{is_cli_method, is_control, is_parallel_read};
use crate::contracts::ipc::FERRY_CONTRACT_HASH;
use crate::errors::DomainError;
use crate::server::rpc::{error_envelope, result_envelope, RpcDispatcher, PROTOCOL};
use crate::server::serve::{
    log_info, log_warning, serve_connection, Lane, LanePolicy, Lanes, ServeHandler,
};

use idle::IdleTracker;
use lock::Binding;

/// 同时在线的 socket 连接上限。CLI 是一次调用一条连接，32 条足够，
/// 上限只是防跑飞。
const MAX_CONNECTIONS: usize = 32;

/// daemon 主循环的巡检间隔。
const TICK: Duration = Duration::from_millis(200);

/// 优雅退出时留给在途响应写出的时间。
const DRAIN_GRACE: Duration = Duration::from_millis(150);

/// 引擎实例的角色。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EngineMode {
    /// App sidecar：stdio 是主通道，socket 只是兼听；不 idle-exit、不可被 CLI 关。
    App,
    /// CLI 自拉起的独立 daemon：stdin 不是 RPC 通道，空闲自动退出。
    Daemon,
}

impl EngineMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::Daemon => "daemon",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "app" => Some(Self::App),
            "daemon" => Some(Self::Daemon),
            _ => None,
        }
    }
}

/// `serve` 的 socket 形态配置。
#[derive(Clone, Debug)]
pub struct SocketConfig {
    pub path: PathBuf,
    pub mode: EngineMode,
    /// 仅 daemon 模式生效。
    pub idle_exit: Option<Duration>,
}

/// `daemon.status` 里的内容索引状态提供者（组合根注入，只读、无副作用）。
pub type ContentIndexStatus = Arc<dyn Fn() -> Value + Send + Sync>;

/// 退出信号：idle 计时、`daemon.shutdown`、SIGTERM 三个来源共用它。
#[derive(Default)]
struct Shutdown {
    requested: Mutex<Option<&'static str>>,
    changed: Condvar,
}

impl Shutdown {
    fn request(&self, reason: &'static str) {
        let mut guard = self
            .requested
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if guard.is_none() {
            *guard = Some(reason);
        }
        self.changed.notify_all();
    }

    fn reason(&self) -> Option<&'static str> {
        *self
            .requested
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }
}

struct Shared {
    mode: EngineMode,
    started: Instant,
    shutdown: Arc<Shutdown>,
    idle: Arc<Mutex<IdleTracker>>,
    content_index_status: Option<ContentIndexStatus>,
}

impl Shared {
    fn status(&self) -> Value {
        let mut payload = Map::new();
        payload.insert("mode".into(), Value::from(self.mode.as_str()));
        payload.insert("pid".into(), Value::from(std::process::id()));
        payload.insert(
            "version".into(),
            Value::from(crate::context::ENGINE_VERSION),
        );
        payload.insert("package".into(), Value::from(env!("CARGO_PKG_VERSION")));
        payload.insert("contract_hash".into(), Value::from(FERRY_CONTRACT_HASH));
        payload.insert(
            "uptime_sec".into(),
            Value::from(self.started.elapsed().as_secs()),
        );
        payload.insert(
            "connections".into(),
            Value::from(
                self.idle
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .active(),
            ),
        );
        payload.insert(
            "content_index".into(),
            match &self.content_index_status {
                Some(status) => status(),
                None => Value::Null,
            },
        );
        Value::Object(payload)
    }
}

/// 运行中的 socket 服务。`Drop` 会清掉 socket 与锁文件。
pub struct SocketServer {
    shared: Arc<Shared>,
    binding: Binding,
    idle_exit: Option<Duration>,
}

impl SocketServer {
    /// 预热完成；daemon 的空闲计时从这里才开始。
    pub fn mark_warm(&self) {
        self.shared
            .idle
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .mark_warm(now_ms());
    }

    /// 交给预热线程的回调：预热一结束就开始 idle 计时。
    pub fn warm_notifier(&self) -> impl Fn() + Send + 'static {
        let idle = Arc::clone(&self.shared.idle);
        move || {
            idle.lock()
                .unwrap_or_else(|error| error.into_inner())
                .mark_warm(now_ms());
        }
    }

    pub fn request_shutdown(&self, reason: &'static str) {
        self.shared.shutdown.request(reason);
    }

    pub fn socket_path(&self) -> &std::path::Path {
        self.binding.socket()
    }

    /// daemon 主循环：等退出信号或空闲超时。
    ///
    /// accept 线程是 detach 的——退出信号到了就走清理，不去唤醒阻塞在 accept
    /// 上的线程：进程随即结束，内核会替我们收尾。
    pub fn run_until_shutdown(&self) {
        // SIGTERM 只在 daemon 模式接管：装上之后信号不再默认终止进程，
        // 而 stdio 模式的主线程阻塞在 stdin 上、根本看不到这个标志。
        if matches!(self.shared.mode, EngineMode::Daemon)
            && !platform::install_termination_handler()
        {
            log_warning("SIGTERM 处理器安装失败：退出时可能留下陈旧 socket");
        }
        loop {
            if let Some(reason) = self.shared.shutdown.reason() {
                log_info(&format!("引擎退出: {reason}"));
                break;
            }
            if platform::termination_requested() {
                log_info("引擎退出: SIGTERM");
                break;
            }
            if self.idle_exit.is_some()
                && self
                    .shared
                    .idle
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .should_exit(now_ms())
            {
                log_info("引擎退出: idle-exit");
                break;
            }
            let guard = self
                .shared
                .shutdown
                .requested
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let _unused = self
                .shared
                .shutdown
                .changed
                .wait_timeout(guard, TICK)
                .unwrap_or_else(|error| error.into_inner());
        }
        // 让 `daemon.shutdown` 的 ok 应答有机会写出去再拆 socket。
        std::thread::sleep(DRAIN_GRACE);
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// 绑定 socket 并起 accept 线程。
pub fn start(
    config: &SocketConfig,
    lanes: Arc<Lanes>,
    dispatcher: Arc<RpcDispatcher>,
    content_index_status: Option<ContentIndexStatus>,
) -> Result<SocketServer, String> {
    let (binding, listener) = lock::bind_exclusive(&config.path, &lock::lock_path(), config.mode)?;
    let shared = Arc::new(Shared {
        mode: config.mode,
        started: Instant::now(),
        shutdown: Arc::new(Shutdown::default()),
        idle: Arc::new(Mutex::new(IdleTracker::new(
            config.idle_exit.unwrap_or(Duration::MAX),
        ))),
        content_index_status,
    });
    let policy = lane_policy(Arc::clone(&shared));
    let handler: ServeHandler = Arc::new(move |request: &str| Ok(dispatcher.handle(request)));
    let connections = Arc::new(AtomicUsize::new(0));
    {
        let shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("engine-socket-accept".into())
            .spawn(move || loop {
                match listener.accept() {
                    Ok(stream) => {
                        if shared.shutdown.reason().is_some() {
                            break;
                        }
                        if connections.load(Ordering::SeqCst) >= MAX_CONNECTIONS {
                            log_warning("socket 连接数已达上限，拒绝新连接");
                            drop(stream);
                            continue;
                        }
                        // 计数与 idle 记账都挂在 guard 上：线程正常结束、
                        // panic、甚至 spawn 失败（闭包被丢弃）都会归还。
                        let guard =
                            ConnectionGuard::open(Arc::clone(&shared), Arc::clone(&connections));
                        let lanes = Arc::clone(&lanes);
                        let handler = Arc::clone(&handler);
                        let policy = Arc::clone(&policy);
                        let spawned = std::thread::Builder::new()
                            .name("engine-socket-conn".into())
                            .spawn(move || {
                                let _guard = guard;
                                if let Err(error) =
                                    handle_connection(&lanes, stream, handler, policy)
                                {
                                    log_warning(&format!("socket 连接异常结束: {error}"));
                                }
                            });
                        if spawned.is_err() {
                            log_warning("无法为 socket 连接启动线程");
                        }
                    }
                    Err(error) => {
                        if shared.shutdown.reason().is_some() {
                            break;
                        }
                        log_warning(&format!("socket accept 失败: {error}"));
                        std::thread::sleep(TICK);
                    }
                }
            })
            .map_err(|error| format!("无法启动 socket accept 线程: {error}"))?;
    }
    log_info(&format!(
        "socket 传输已就绪: {} mode={}",
        config.path.display(),
        config.mode.as_str()
    ));
    Ok(SocketServer {
        shared,
        binding,
        idle_exit: config.idle_exit,
    })
}

/// 一条连接的记账凭证：建时占位，析构时归还。
struct ConnectionGuard {
    shared: Arc<Shared>,
    connections: Arc<AtomicUsize>,
}

impl ConnectionGuard {
    fn open(shared: Arc<Shared>, connections: Arc<AtomicUsize>) -> Self {
        connections.fetch_add(1, Ordering::SeqCst);
        shared
            .idle
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .connection_opened();
        Self {
            shared,
            connections,
        }
    }
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.connections.fetch_sub(1, Ordering::SeqCst);
        self.shared
            .idle
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .connection_closed(now_ms());
    }
}

fn handle_connection(
    lanes: &Arc<Lanes>,
    stream: platform::SocketStream,
    handler: ServeHandler,
    policy: LanePolicy,
) -> Result<(), String> {
    let reader = stream
        .try_clone()
        .map_err(|error| format!("socket 连接不可复制: {error}"))?;
    serve_connection(
        lanes,
        BufReader::new(reader),
        Box::new(stream),
        handler,
        policy,
    )
}

/// socket 通道的分道策略：管理方法拦截 + callers 过滤 + 契约分道。
fn lane_policy(shared: Arc<Shared>) -> LanePolicy {
    Arc::new(move |request: &str| {
        let Ok(value) = serde_json::from_str::<Value>(request) else {
            // 交给 RpcDispatcher 报 `rpc.invalid_json`，错误形态只有一处定义。
            return Lane::Serial;
        };
        if value.get("protocol") != Some(&Value::from(PROTOCOL)) {
            return Lane::Serial;
        }
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty() && id.chars().count() <= 128)
            .unwrap_or("unknown")
            .to_string();
        let Some(method) = value.get("method").and_then(Value::as_str) else {
            return Lane::Serial;
        };
        match method {
            "daemon.status" => Lane::Immediate(result_envelope(shared.status(), &id)),
            "daemon.shutdown" => Lane::Immediate(shutdown_response(&shared, &id)),
            other if !is_cli_method(other) => Lane::Immediate(error_envelope(
                &DomainError::method_not_exposed(other, "cli"),
                &id,
            )),
            other if is_control(other) => Lane::Control,
            other if is_parallel_read(other) => Lane::Parallel,
            _ => Lane::Serial,
        }
    })
}

fn shutdown_response(shared: &Shared, id: &str) -> Value {
    match shared.mode {
        EngineMode::Daemon => {
            shared.shutdown.request("daemon.shutdown");
            let mut payload = Map::new();
            payload.insert("stopping".into(), Value::Bool(true));
            payload.insert("pid".into(), Value::from(std::process::id()));
            result_envelope(Value::Object(payload), id)
        }
        // App 的引擎跟着 App 的生命周期走；CLI 只能选择不用它。
        EngineMode::App => error_envelope(
            &DomainError::transport_refused(
                "app_mode",
                "App 共享的引擎不接受 daemon.shutdown",
                "退出 Ferry App 即可释放引擎；或在设置页关闭「允许 CLI 共享 App 引擎」",
            ),
            id,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "windows")]
    use std::io::{BufRead, BufReader, Write};

    fn shared(mode: EngineMode) -> Arc<Shared> {
        Arc::new(Shared {
            mode,
            started: Instant::now(),
            shutdown: Arc::new(Shutdown::default()),
            idle: Arc::new(Mutex::new(IdleTracker::new(Duration::from_secs(600)))),
            content_index_status: Some(Arc::new(|| Value::from("stub"))),
        })
    }

    fn request(method: &str) -> String {
        format!(r#"{{"protocol":"{PROTOCOL}","id":"x","method":"{method}","params":{{}}}}"#)
    }

    fn immediate(policy: &LanePolicy, method: &str) -> Option<Value> {
        match policy(&request(method)) {
            Lane::Immediate(value) => Some(value),
            _ => None,
        }
    }

    #[test]
    fn non_cli_methods_are_refused_before_dispatch() {
        let policy = lane_policy(shared(EngineMode::Daemon));
        for method in [
            "show",
            "session_search",
            "runtime_sessions.load_all",
            "agent_prompt",
        ] {
            let response = immediate(&policy, method).expect("必须被传输层拦下");
            assert_eq!(response["ok"], Value::Bool(false), "{method}");
            assert_eq!(response["error"]["code"], Value::from("rpc.unknown_method"));
            assert_eq!(response["error"]["params"]["caller"], Value::from("cli"));
        }
        // callers 含 cli 的方法照旧进工作道。
        for method in ["content_search", "session_read", "usage_stats"] {
            assert!(
                matches!(policy(&request(method)), Lane::Parallel),
                "{method}"
            );
        }
        assert!(matches!(policy(&request("health")), Lane::Control));
        assert!(matches!(policy(&request("operation.plan")), Lane::Serial));
    }

    #[test]
    fn daemon_status_is_answered_by_the_transport() {
        let policy = lane_policy(shared(EngineMode::Daemon));
        let response = immediate(&policy, "daemon.status").expect("传输层直答");
        assert_eq!(response["ok"], Value::Bool(true));
        assert_eq!(response["result"]["mode"], Value::from("daemon"));
        assert_eq!(
            response["result"]["contract_hash"],
            Value::from(FERRY_CONTRACT_HASH)
        );
        assert_eq!(response["result"]["content_index"], Value::from("stub"));
    }

    #[test]
    fn shutdown_is_honoured_for_daemon_and_refused_for_app() {
        let daemon = shared(EngineMode::Daemon);
        let policy = lane_policy(Arc::clone(&daemon));
        let response = immediate(&policy, "daemon.shutdown").expect("传输层直答");
        assert_eq!(response["ok"], Value::Bool(true));
        assert_eq!(daemon.shutdown.reason(), Some("daemon.shutdown"));

        let app = shared(EngineMode::App);
        let policy = lane_policy(Arc::clone(&app));
        let response = immediate(&policy, "daemon.shutdown").expect("传输层直答");
        assert_eq!(response["ok"], Value::Bool(false));
        assert_eq!(
            response["error"]["code"],
            Value::from("rpc.invalid_request")
        );
        assert_eq!(
            response["error"]["params"]["reason"],
            Value::from("app_mode")
        );
        assert_eq!(app.shutdown.reason(), None, "App 的引擎不能被 CLI 杀");
    }

    /// 协议不匹配的帧不走拦截：错误形态由 RpcDispatcher 统一给。
    #[test]
    fn frames_with_a_wrong_protocol_fall_through_to_the_dispatcher() {
        let policy = lane_policy(shared(EngineMode::Daemon));
        let raw = r#"{"protocol":"nope/9","id":"x","method":"daemon.shutdown","params":{}}"#;
        assert!(matches!(policy(raw), Lane::Serial));
        assert!(matches!(policy("not json"), Lane::Serial));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn worker_response_is_written_before_the_next_pipe_read() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("engine.sock");
        let listener = platform::bind(&marker).unwrap();
        let lanes = Lanes::new();
        let server_lanes = Arc::clone(&lanes);
        let server = std::thread::spawn(move || {
            let stream = listener.accept().unwrap();
            let reader = stream.try_clone().unwrap();
            let handler: ServeHandler = Arc::new(|request| {
                let value: Value = serde_json::from_str(request).unwrap();
                Ok(serde_json::json!({"id": value["id"], "ok": true}))
            });
            let policy: LanePolicy = Arc::new(|_| Lane::Control);
            serve_connection(
                &server_lanes,
                BufReader::new(reader),
                Box::new(stream),
                handler,
                policy,
            )
            .unwrap();
        });

        let mut stream = platform::connect(&marker).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        stream
            .write_all(b"{\"id\":\"one\"}\n")
            .and_then(|()| stream.flush())
            .unwrap();
        let mut response = String::new();
        reader.read_line(&mut response).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&response).unwrap()["id"],
            "one"
        );
        drop(reader);
        drop(stream);
        server.join().unwrap();
        lanes.shutdown();
    }
}
