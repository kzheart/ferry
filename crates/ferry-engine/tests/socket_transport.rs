//! 本地 socket 传输的黑盒端到端：起真实进程、连真实 socket、只看字节与文件。
//!
//! 覆盖四件事：
//! 1. socket 上能走通 `ferry-ipc/1`（`health` / `content_search`）；
//! 2. callers 矩阵在传输层生效——`show` 这类不含 cli 的方法被拒；
//! 3. `daemon.shutdown` 在 daemon 模式优雅退出、在 app 模式被结构化拒绝；
//! 4. 薄客户端能自拉起 daemon、`daemon status/stop` 能管住它。
//!
//! unix-only：Windows 走命名管道，是 P2 的事（平台边界见
//! `server/socket/platform`）。

#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

const BINARY: &str = env!("CARGO_BIN_EXE_ferry-engine");

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("仓库根可规范化")
}

fn protocol() -> String {
    let path = repo_root().join("contracts").join("ipc.json");
    let text = std::fs::read_to_string(path).expect("contracts/ipc.json 可读");
    serde_json::from_str::<Value>(&text).expect("contracts/ipc.json 是 JSON")["protocol"]
        .as_str()
        .expect("protocol 是字符串")
        .to_string()
}

/// 干净沙箱：HOME 与全部 Ferry 目录都指向临时目录，socket 路径经环境变量注入。
struct Sandbox {
    root: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("临时沙箱可创建");
        std::fs::create_dir_all(root.path().join(".ferry")).expect("状态目录可创建");
        Self { root }
    }

    fn home(&self) -> &Path {
        self.root.path()
    }

    /// unix socket 的路径长度上限很低（macOS 104 字节），名字要短。
    fn socket(&self) -> PathBuf {
        self.home().join(".ferry").join("e.sock")
    }

    fn lock(&self) -> PathBuf {
        self.home().join(".ferry").join("engine.lock")
    }

    fn command(&self) -> Command {
        let home = self.home();
        let mut command = Command::new(BINARY);
        command
            .env("HOME", home)
            .env("USERPROFILE", home)
            .env("XDG_DATA_HOME", home.join(".local/share"))
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("FERRY_DATA_DIR", home.join(".ferry"))
            .env("FERRY_BACKUP_DIR", home.join(".ferry/backups"))
            .env("FERRY_ENGINE_SOCKET", self.socket())
            .env("FERRY_OPENCODE_DB", home.join("opencode/storage.db"))
            .env("GROK_HOME", home.join(".grok"))
            .env("PI_CODING_AGENT_SESSION_DIR", home.join("pi-sessions"))
            .env_remove("CODEX_HOME")
            .env_remove("PI_CODING_AGENT_DIR")
            .env_remove("FERRY_DEBUG");
        command
    }
}

/// 起一个 serve 进程，等它把 socket 挂出来。
struct Engine {
    child: Child,
}

impl Engine {
    fn start(sandbox: &Sandbox, args: &[&str]) -> Self {
        let mut command = sandbox.command();
        command.arg("serve");
        for argument in args {
            command.arg(argument);
        }
        let child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("serve 可启动");
        wait_for(&sandbox.socket(), true);
        Self { child }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn wait_for(path: &Path, present: bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while path.exists() != present {
        assert!(
            Instant::now() < deadline,
            "等待 {} {} 超时",
            path.display(),
            if present { "出现" } else { "消失" }
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// 一条 socket 会话：写一行请求，读一行应答。
struct Session {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Session {
    fn open(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).expect("socket 可连接");
        stream
            .set_read_timeout(Some(Duration::from_secs(60)))
            .expect("可设读超时");
        let reader = BufReader::new(stream.try_clone().expect("连接可复制"));
        Self { stream, reader }
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        let request = json!({
            "protocol": protocol(),
            "id": format!("socket-{method}"),
            "method": method,
            "params": params,
        });
        writeln!(self.stream, "{request}").expect("请求可写入");
        self.stream.flush().expect("请求可冲刷");
        let mut line = String::new();
        let read = self.reader.read_line(&mut line).expect("应答可读");
        assert!(read > 0, "{method} 没有应答");
        serde_json::from_str(line.trim()).expect("应答是 JSON")
    }
}

#[test]
fn daemon_socket_serves_cli_methods_and_refuses_the_rest() {
    let sandbox = Sandbox::new();
    let engine = Engine::start(&sandbox, &["--mode", "daemon", "--idle-exit", "3600"]);
    let mut session = Session::open(&sandbox.socket());

    let health = session.call("health", json!({}));
    assert_eq!(health["ok"], Value::Bool(true), "{health}");
    assert_eq!(health["result"]["status"], Value::from("ready"));
    assert_eq!(health["result"]["service"], Value::from("engine"));

    // 能力方法照常走通（沙箱里没会话，返回空结果即可）。
    let search = session.call("content_search", json!({"query": "ferry", "limit": 1}));
    assert_eq!(search["ok"], Value::Bool(true), "{search}");
    assert!(search["result"]["sessions"].is_array());

    // callers 矩阵：不含 cli 的方法在分发之前就被拒。
    for method in ["show", "session_search", "runtime_sessions.load_all"] {
        let refused = session.call(method, json!({}));
        assert_eq!(refused["ok"], Value::Bool(false), "{method}: {refused}");
        assert_eq!(
            refused["error"]["code"],
            Value::from("rpc.unknown_method"),
            "{method}"
        );
        assert_eq!(refused["error"]["params"]["caller"], Value::from("cli"));
    }

    // 传输层管理方法。
    let status = session.call("daemon.status", json!({}));
    assert_eq!(status["ok"], Value::Bool(true), "{status}");
    assert_eq!(status["result"]["mode"], Value::from("daemon"));
    assert!(status["result"]["contract_hash"].is_string());
    assert!(status["result"]["content_index"].is_object());

    // 锁文件：pid 指向真身，权限只对本人开放。
    let lock: Value =
        serde_json::from_str(&std::fs::read_to_string(sandbox.lock()).expect("锁文件可读"))
            .expect("锁文件是 JSON");
    assert_eq!(lock["pid"], Value::from(engine.child.id()));
    assert_eq!(lock["mode"], Value::from("daemon"));
    assert_eq!(mode_bits(&sandbox.socket()), 0o600, "socket 必须是 0600");
    assert_eq!(mode_bits(&sandbox.lock()), 0o600, "锁文件必须是 0600");

    // 优雅退出：先回 ok，再清 socket 与锁。
    let stopping = session.call("daemon.shutdown", json!({}));
    assert_eq!(stopping["ok"], Value::Bool(true), "{stopping}");
    assert_eq!(stopping["result"]["stopping"], Value::Bool(true));
    wait_for(&sandbox.socket(), false);
    wait_for(&sandbox.lock(), false);
}

#[test]
fn app_mode_engine_cannot_be_stopped_by_the_cli() {
    let sandbox = Sandbox::new();
    // stdio + socket 兼听：App sidecar 的形态。
    let _engine = Engine::start(&sandbox, &["--socket"]);
    let mut session = Session::open(&sandbox.socket());

    let health = session.call("health", json!({}));
    assert_eq!(health["ok"], Value::Bool(true), "{health}");

    let refused = session.call("daemon.shutdown", json!({}));
    assert_eq!(refused["ok"], Value::Bool(false), "{refused}");
    assert_eq!(refused["error"]["code"], Value::from("rpc.invalid_request"));
    assert_eq!(
        refused["error"]["params"]["reason"],
        Value::from("app_mode")
    );

    // 拒绝之后引擎照常服务，socket 也还在。
    let status = session.call("daemon.status", json!({}));
    assert_eq!(status["result"]["mode"], Value::from("app"));
    assert!(sandbox.socket().exists());
}

#[test]
fn the_thin_client_starts_a_daemon_and_can_stop_it() {
    let sandbox = Sandbox::new();

    // 没有引擎时，daemon stop 直接说「未运行」，不自拉起。
    let idle = sandbox
        .command()
        .args(["daemon", "stop"])
        .output()
        .expect("可执行");
    assert_eq!(idle.status.code(), Some(2), "连接失败退出码是 2");
    let payload: Value = serde_json::from_slice(&idle.stderr).expect("错误信封走 stderr 且是 JSON");
    assert_eq!(payload["code"], Value::from("engine.unavailable"));
    assert_eq!(payload["params"]["reason"], Value::from("not_running"));
    assert!(!sandbox.socket().exists(), "报告现状不该拉起 daemon");

    // 任意一条能力命令都会按需拉起 daemon。
    let health = sandbox.command().arg("health").output().expect("可执行");
    assert_eq!(
        health.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&health.stderr)
    );
    let result: Value = serde_json::from_slice(&health.stdout).expect("stdout 是 JSON");
    assert_eq!(result["status"], Value::from("ready"));
    assert!(sandbox.socket().exists(), "daemon 已被拉起");

    // 拉起来的是 daemon 模式，能被 daemon stop 收走。
    let status = sandbox
        .command()
        .args(["daemon", "status"])
        .output()
        .expect("可执行");
    assert_eq!(status.status.code(), Some(0));
    let status: Value = serde_json::from_slice(&status.stdout).expect("stdout 是 JSON");
    assert_eq!(status["mode"], Value::from("daemon"));

    let stop = sandbox
        .command()
        .args(["daemon", "stop"])
        .output()
        .expect("可执行");
    assert_eq!(stop.status.code(), Some(0));
    wait_for(&sandbox.socket(), false);
    wait_for(&sandbox.lock(), false);
}

/// SIGTERM 的优雅路径：不留陈旧 socket 与锁。
#[test]
fn sigterm_cleans_up_the_socket_and_the_lock() {
    let sandbox = Sandbox::new();
    let engine = Engine::start(&sandbox, &["--mode", "daemon", "--idle-exit", "3600"]);
    assert!(sandbox.lock().exists());
    let killed = Command::new("kill")
        .args(["-TERM", &engine.child.id().to_string()])
        .status()
        .expect("kill 可执行");
    assert!(killed.success());
    wait_for(&sandbox.socket(), false);
    wait_for(&sandbox.lock(), false);
}

/// idle-exit 的整条接线：预热完成 → 最后一条连接关闭 → 计时退出。
#[test]
fn an_idle_daemon_exits_on_its_own() {
    let sandbox = Sandbox::new();
    let _engine = Engine::start(&sandbox, &["--mode", "daemon", "--idle-exit", "1"]);
    {
        let mut session = Session::open(&sandbox.socket());
        assert_eq!(session.call("health", json!({}))["ok"], Value::Bool(true));
    }
    wait_for(&sandbox.socket(), false);
    wait_for(&sandbox.lock(), false);
}

/// 用法错误不该穿到引擎，也不该被当成成功。
#[test]
fn client_usage_errors_are_reported_locally() {
    let sandbox = Sandbox::new();
    let failure = sandbox
        .command()
        .args(["read", "claude"])
        .output()
        .expect("可执行");
    assert_eq!(failure.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&failure.stderr).contains("缺少参数 <ref>"),
        "stderr={}",
        String::from_utf8_lossy(&failure.stderr)
    );
    assert!(!sandbox.socket().exists(), "参数没凑齐就不该连引擎");
}

fn mode_bits(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .expect("可取元数据")
        .permissions()
        .mode()
        & 0o777
}
