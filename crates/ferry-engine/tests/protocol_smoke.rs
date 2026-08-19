//! 黑盒协议 smoke：直接 spawn `ferry-engine` 二进制，只看 stdout 上的字节。
//!
//! 形态对齐 `.github/workflows/ci.yml` 的 frozen-sidecar smoke（health / version /
//! env 三条），再补上 serve 模式的三条硬约束（方案 §2.1 第 3 / 7 条）：
//! 并发只读池可乱序、串行池严格保序、事件帧无 `id`。
//!
//! 这里刻意**不 link lib**去读内部状态：宿主看到的就是这些字节，测的也只能是
//! 这些字节。唯一的例外是把 `contracts/agents.json` 当断言基准（与 CI 同源）。

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
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

fn contract_agent_ids() -> BTreeSet<String> {
    let path = repo_root().join("contracts").join("agents.json");
    let text = std::fs::read_to_string(path).expect("contracts/agents.json 可读");
    let value: Value = serde_json::from_str(&text).expect("contracts/agents.json 是 JSON");
    value["agents"]
        .as_array()
        .expect("agents 是数组")
        .iter()
        .map(|agent| agent["id"].as_str().expect("id 是字符串").to_string())
        .collect()
}

fn request(method: &str, id: &str, params: Value) -> String {
    json!({"protocol": protocol(), "id": id, "method": method, "params": params}).to_string()
}

/// 干净沙箱：HOME 与全部 Ferry 目录都指向临时目录，绝不碰运行者的真实数据。
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

    /// 把一个 claude fixture 物化成真实存储布局，让 scan / 活索引有东西可看。
    fn seed_claude_session(&self) -> PathBuf {
        let fixtures = repo_root()
            .join("tests")
            .join("fixtures")
            .join("agent_formats")
            .join("claude");
        let case = std::fs::read_dir(&fixtures)
            .expect("claude fixture 目录可读")
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.join("session.jsonl").is_file())
            .min()
            .expect("至少有一个 claude fixture");
        let name = case.file_name().expect("case 有目录名").to_owned();
        let target = self
            .home()
            .join(".claude")
            .join("projects")
            .join(&name)
            .join("smoke.jsonl");
        std::fs::create_dir_all(target.parent().expect("有父目录")).expect("会话目录可创建");
        std::fs::copy(case.join("session.jsonl"), &target).expect("fixture 可拷贝");
        target
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
            .env("FERRY_OPENCODE_DB", home.join("opencode/storage.db"))
            .env("GROK_HOME", home.join(".grok"))
            .env("PI_CODING_AGENT_SESSION_DIR", home.join("pi-sessions"))
            .env_remove("CODEX_HOME")
            .env_remove("PI_CODING_AGENT_DIR")
            .env_remove("FERRY_DEBUG");
        command
    }

    /// 一次性 `rpc`：返回 stdout 首行解析出的应答。
    fn rpc(&self, method: &str, params: Value) -> Value {
        let output = self
            .command()
            .arg("rpc")
            .arg(request(method, &format!("smoke-{method}"), params))
            .stdin(Stdio::null())
            .output()
            .expect("ferry-engine 可执行");
        assert!(
            output.status.success(),
            "{method} 退出码非零: {:?}\nstderr={}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).expect("stdout 是 UTF-8");
        let line = stdout
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or_else(|| {
                panic!(
                    "{method} 无输出\nstderr={}",
                    String::from_utf8_lossy(&output.stderr)
                )
            });
        serde_json::from_str(line).expect("应答是 JSON")
    }
}

/// 造一批「慢 CLI」垫片，让 `env` 的探针可控地耗时（用于并发乱序断言）。
#[cfg(unix)]
fn slow_cli_dir(sandbox: &Sandbox) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let dir = sandbox.home().join("slow-bin");
    std::fs::create_dir_all(&dir).expect("垫片目录可创建");
    for name in contract_agent_ids() {
        let script = dir.join(&name);
        std::fs::write(&script, "#!/bin/sh\nsleep 0.4\necho 1.0.0\n").expect("垫片可写");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .expect("垫片可执行");
    }
    dir
}

/// serve 会话：写请求、按 id 收应答，同时把事件帧单独攒起来。
struct Serve {
    child: Child,
    reader: BufReader<std::process::ChildStdout>,
    events: Vec<Value>,
}

impl Serve {
    fn start(sandbox: &Sandbox) -> Self {
        let mut child = sandbox
            .command()
            .arg("serve")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("serve 可启动");
        let stdout = child.stdout.take().expect("serve 有 stdout");
        Self {
            child,
            reader: BufReader::new(stdout),
            events: Vec::new(),
        }
    }

    fn send(&mut self, method: &str, id: &str, params: Value) {
        let line = request(method, id, params);
        let stdin = self.child.stdin.as_mut().expect("serve 有 stdin");
        writeln!(stdin, "{line}").expect("请求可写入");
        stdin.flush().expect("请求可冲刷");
    }

    /// 读下一个**应答**（有 `id`），事件帧顺手收进 `events`。
    fn next_response(&mut self) -> Value {
        loop {
            let mut line = String::new();
            let read = self.reader.read_line(&mut line).expect("stdout 可读");
            assert!(read > 0, "serve 提前关闭 stdout");
            if line.trim().is_empty() {
                continue;
            }
            let frame: Value = serde_json::from_str(&line).expect("每一行都必须是 JSON");
            if frame.get("id").is_some() {
                return frame;
            }
            // 事件帧：无 id、有 type（§2.1 第 3 条）。
            self.events.push(frame);
        }
    }

    /// 收集帧直到出现事件帧或超时。
    fn wait_for_event(&mut self, budget: Duration) -> Option<Value> {
        if let Some(event) = self.events.first() {
            return Some(event.clone());
        }
        let deadline = Instant::now() + budget;
        while Instant::now() < deadline {
            let mut line = String::new();
            match self.reader.read_line(&mut line) {
                Ok(0) | Err(_) => return None,
                Ok(_) => {}
            }
            if line.trim().is_empty() {
                continue;
            }
            let frame: Value = serde_json::from_str(&line).expect("每一行都必须是 JSON");
            if frame.get("id").is_none() {
                return Some(frame);
            }
        }
        None
    }

    fn finish(mut self) {
        drop(self.child.stdin.take());
        let _ = self.child.wait();
    }
}

// ---------------------------------------------------------------------------
// 一次性 rpc：CI frozen-sidecar smoke 的等价物
// ---------------------------------------------------------------------------

#[test]
fn handshake_reports_ready_with_the_contract_hash() {
    let sandbox = Sandbox::new();
    let response = sandbox.rpc("health", json!({}));
    assert_eq!(response["protocol"], json!(protocol()));
    assert_eq!(response["ok"], json!(true));
    assert_eq!(response["id"], json!("smoke-health"));
    let result = &response["result"];
    assert_eq!(result["status"], json!("ready"));
    assert_eq!(result["service"], json!("engine"));
    let hash = result["contract_hash"]
        .as_str()
        .expect("contract_hash 是字符串");
    assert!(hash.starts_with("sha256:"), "contract_hash={hash}");
    // 与生成契约同源：宿主用它判定 sidecar 与前端是否同一份契约。
    assert_eq!(hash, ferry_engine::contracts::ipc::FERRY_CONTRACT_HASH);
}

#[test]
fn version_returns_a_non_empty_string() {
    let sandbox = Sandbox::new();
    let response = sandbox.rpc("version", json!({}));
    assert_eq!(response["ok"], json!(true));
    let version = response["result"]["version"]
        .as_str()
        .expect("version 是字符串");
    assert!(!version.is_empty());
}

#[test]
fn env_reports_exactly_the_contract_agent_ids() {
    let sandbox = Sandbox::new();
    let response = sandbox.rpc("env", json!({}));
    assert_eq!(response["ok"], json!(true));
    let reported: BTreeSet<String> = response["result"]
        .as_object()
        .expect("env 结果是 object")
        .keys()
        .cloned()
        .collect();
    assert_eq!(reported, contract_agent_ids());
    for (tool, info) in response["result"].as_object().expect("是 object") {
        assert!(info["installed"].is_boolean(), "{tool} 的 installed 非布尔");
        assert!(info["broken"].is_boolean(), "{tool} 的 broken 非布尔");
        assert!(
            info["path"].is_string() || info["path"].is_null(),
            "{tool} 的 path 形状不对"
        );
    }
}

#[test]
fn envelope_errors_still_carry_a_string_id() {
    let sandbox = Sandbox::new();
    // 未知方法：错误信封同样必须带 string 型 id，否则宿主判定连接死亡。
    let response = sandbox.rpc("nope", json!({}));
    assert_eq!(response["ok"], json!(false));
    assert!(response["id"].is_string());
    assert_eq!(response["error"]["code"], json!("rpc.unknown_method"));
    assert!(response["error"]["params"]["message"].is_string());
}

// ---------------------------------------------------------------------------
// serve：并发乱序 / 串行保序 / 事件帧
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn parallel_read_requests_can_finish_out_of_input_order() {
    let sandbox = Sandbox::new();
    let bin = slow_cli_dir(&sandbox);
    let mut serve = {
        let mut child = sandbox
            .command()
            // 只留垫片目录：env 的 5 次 `--version` 各睡 0.4 秒。
            .env("PATH", &bin)
            .arg("serve")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("serve 可启动");
        let stdout = child.stdout.take().expect("serve 有 stdout");
        Serve {
            child,
            reader: BufReader::new(stdout),
            events: Vec::new(),
        }
    };

    // 两条都在 parallel-read 池里：慢的先进，快的必须先出。
    serve.send("env", "slow", json!({}));
    serve.send("version", "fast", json!({}));
    let first = serve.next_response();
    let second = serve.next_response();
    assert_eq!(first["id"], json!("fast"), "并发只读池没有乱序完成");
    assert_eq!(second["id"], json!("slow"));
    assert_eq!(first["ok"], json!(true));
    assert_eq!(second["ok"], json!(true));
    serve.finish();
}

#[test]
fn serial_requests_keep_their_input_order() {
    let sandbox = Sandbox::new();
    sandbox.seed_claude_session();
    let mut serve = Serve::start(&sandbox);
    // scan 是串行池里最慢的一条（全量扫库），后面跟一条几乎瞬时的串行请求。
    serve.send("scan", "s1", json!({}));
    serve.send("runtime_sessions.load_all", "s2", json!({}));
    serve.send("session_meta_list", "s3", json!({}));
    // s3 走 parallel-read 池，可以随时完成（甚至先于 s2）；串行保序只约束
    // s1 相对 s2 的顺序，所以按到达序收齐三条再断言相对位置。
    let mut arrival: Vec<Value> = Vec::new();
    for _ in 0..3 {
        let frame = serve.next_response();
        assert_eq!(frame["ok"], json!(true), "请求失败: {frame}");
        arrival.push(frame["id"].clone());
    }
    let position = |id: &str| {
        arrival
            .iter()
            .position(|item| item == &json!(id))
            .unwrap_or_else(|| panic!("{id} 没有应答: {arrival:?}"))
    };
    assert!(
        position("s1") < position("s2"),
        "串行池没有保序: {arrival:?}"
    );
    serve.finish();
}

#[test]
fn session_changes_are_pushed_as_id_less_event_frames() {
    let sandbox = Sandbox::new();
    let session = sandbox.seed_claude_session();
    let mut serve = Serve::start(&sandbox);
    // 先建立基线快照：活索引首轮只记基线，不推增量。
    let scan = serve.next_response_for("scan", "baseline");
    assert_eq!(scan["ok"], json!(true));

    // 等活索引的首轮探测（poll 2.5s）把当前状态记为基线令牌之后再改文件：
    // 首轮之前的变更会被折进基线，那样就只能
    // 靠 300s 的全量对账兜底，测试会等满五分钟。
    std::thread::sleep(Duration::from_secs(4));

    // 追加一条记录，等活索引轮询发现（2.5s 轮询 + 两轮防抖）。
    let mut content = std::fs::read_to_string(&session).expect("会话可读");
    content.push_str(
        &json!({
            "type": "user",
            "uuid": "smoke-appended",
            "parentUuid": Value::Null,
            "timestamp": "2026-07-25T00:00:01.000Z",
            "message": {"role": "user", "content": "smoke"},
        })
        .to_string(),
    );
    content.push('\n');
    std::fs::write(&session, content).expect("会话可追写");

    let event = serve
        .wait_for_event(Duration::from_secs(60))
        .expect("活索引应在轮询周期内推出 sessions.changed");
    assert!(event.get("id").is_none(), "事件帧不得带 id: {event}");
    assert_eq!(event["type"], json!("sessions.changed"));
    assert_eq!(event["protocol"], json!(protocol()));
    assert!(event["payload"].is_object(), "事件负载是 object");
    assert!(
        event["payload"]["generation"].is_i64() || event["payload"]["generation"].is_u64(),
        "增量必须带 generation: {event}"
    );
    serve.finish();
}

impl Serve {
    /// 发一条请求并等它自己的应答（中途的事件帧照旧收进 `events`）。
    fn next_response_for(&mut self, method: &str, id: &str) -> Value {
        self.send(method, id, json!({}));
        loop {
            let frame = self.next_response();
            if frame["id"] == json!(id) {
                return frame;
            }
        }
    }
}
