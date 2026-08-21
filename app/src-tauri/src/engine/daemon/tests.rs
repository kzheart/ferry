use super::*;

/// 帧的形状是与引擎传输层的契约:协议、id、method、空 params 一个都不能少。
#[test]
fn control_frames_carry_the_protocol_and_a_host_owned_id() {
    let frame = control_frame("daemon.shutdown");
    let value: Value = serde_json::from_str(&frame).expect("帧是 JSON");
    assert_eq!(value["protocol"], Value::from(FERRY_IPC_PROTOCOL));
    assert_eq!(value["id"], Value::from("host_daemon_shutdown"));
    assert_eq!(value["method"], Value::from("daemon.shutdown"));
    assert_eq!(value["params"], serde_json::json!({}));
    // 单行 JSONL:帧里不能出现换行,否则会被对面拆成两条请求。
    assert!(!frame.contains('\n'));
    assert_eq!(
        serde_json::from_str::<Value>(&control_frame("daemon.status")).expect("帧是 JSON")["id"],
        Value::from("host_daemon_status")
    );
}

#[test]
fn responses_must_match_the_protocol_and_the_request_id() {
    let ok = serde_json::json!({
        "protocol": FERRY_IPC_PROTOCOL,
        "id": "host_daemon_status",
        "ok": true,
        "result": {"mode": "daemon"},
    })
    .to_string();
    let parsed = parse_control_response(&ok, "daemon.status").expect("应答可解析");
    assert!(envelope_ok(&parsed));
    assert_eq!(parsed.pointer("/result/mode"), Some(&Value::from("daemon")));

    // 别人的应答、别的协议、不是 JSON,一律拒收。
    assert!(parse_control_response(&ok, "daemon.shutdown").is_err());
    assert!(parse_control_response(
        r#"{"protocol":"nope/9","id":"host_daemon_status"}"#,
        "daemon.status"
    )
    .is_err());
    assert!(parse_control_response("not json", "daemon.status").is_err());
}

/// 拒绝信封里的人话在 `error.params.message`,拿不到退回 code。
#[test]
fn refusals_surface_the_engine_explanation() {
    let refused = serde_json::json!({
        "ok": false,
        "error": {
            "code": "rpc.invalid_request",
            "params": {"reason": "app_mode", "message": "App 共享的引擎不接受 daemon.shutdown"},
        },
    });
    assert!(!envelope_ok(&refused));
    assert_eq!(
        envelope_message(&refused),
        "App 共享的引擎不接受 daemon.shutdown"
    );
    assert_eq!(
        envelope_message(
            &serde_json::json!({"ok": false, "error": {"code": "engine.unavailable"}})
        ),
        "engine.unavailable"
    );
    assert_eq!(
        envelope_message(&serde_json::json!({"ok": false})),
        "引擎拒绝了这次请求"
    );
}

#[cfg(unix)]
mod unix_socket {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;

    /// unix socket 的路径长度上限很低(macOS 104 字节),临时目录的名字要短。
    fn scratch_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "frr-{label}-{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("临时目录可创建");
        dir
    }

    /// 临时目录里的假引擎:只接一条连接,读一行、回一行。
    struct FakeEngine {
        dir: PathBuf,
        socket: PathBuf,
        handle: Option<std::thread::JoinHandle<Vec<String>>>,
    }

    impl FakeEngine {
        /// `release` 决定回完应答是否拆掉 socket 文件(真引擎的优雅退出会拆)。
        /// 调用方必须真的连上来,否则 [`FakeEngine::requests`] 会一直等在 accept 上。
        fn start(label: &str, response: Value, release: bool) -> Self {
            let dir = scratch_dir(label);
            let socket = dir.join("e.sock");
            let listener = UnixListener::bind(&socket).expect("假引擎可监听");
            let socket_for_thread = socket.clone();
            let handle = std::thread::spawn(move || {
                let mut seen = Vec::new();
                if let Ok((stream, _address)) = listener.accept() {
                    let mut reader = BufReader::new(stream.try_clone().expect("连接可复制"));
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) > 0 {
                        seen.push(line.trim_end().to_owned());
                        let mut stream = stream;
                        let _ = writeln!(stream, "{response}");
                        let _ = stream.flush();
                    }
                }
                if release {
                    let _ = std::fs::remove_file(&socket_for_thread);
                }
                seen
            });
            Self {
                dir,
                socket,
                handle: Some(handle),
            }
        }

        fn requests(&mut self) -> Vec<String> {
            self.handle
                .take()
                .map(|handle| handle.join().expect("假引擎线程正常结束"))
                .unwrap_or_default()
        }
    }

    impl Drop for FakeEngine {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn ok_shutdown() -> Value {
        serde_json::json!({
            "protocol": FERRY_IPC_PROTOCOL,
            "id": "host_daemon_shutdown",
            "ok": true,
            "result": {"stopping": true, "pid": 4242},
        })
    }

    #[test]
    fn evicting_sends_one_shutdown_frame_and_waits_for_the_socket_to_go() {
        let mut engine = FakeEngine::start("evict", ok_shutdown(), true);
        evict(&engine.socket);
        let requests = engine.requests();
        assert_eq!(requests.len(), 1, "只发一条管理方法");
        let sent: Value = serde_json::from_str(&requests[0]).expect("请求是 JSON");
        assert_eq!(sent["method"], Value::from("daemon.shutdown"));
        assert_eq!(sent["protocol"], Value::from(FERRY_IPC_PROTOCOL));
        assert!(!engine.socket.exists(), "让位之后 socket 文件必须消失");
    }

    /// 收下了却不退:等待有上限,到点返回 false 交给调用方降级,不能挂住启动。
    #[test]
    fn waiting_gives_up_when_the_daemon_never_releases_the_socket() {
        let dir = scratch_dir("timeout");
        let socket = dir.join("e.sock");
        // 只要 socket 文件在就算没让位,连不连得上不影响这一步的判断。
        let _listener = UnixListener::bind(&socket).expect("可监听");
        let started = std::time::Instant::now();
        assert!(!wait_for_release(&socket, Duration::from_millis(300)));
        let elapsed = started.elapsed();
        assert!(elapsed >= Duration::from_millis(300), "{elapsed:?}");
        assert!(
            elapsed < Duration::from_secs(3),
            "不能等到天荒地老: {elapsed:?}"
        );
        assert!(socket.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 被拒(对面是另一个 App 的引擎)时立刻返回:等它退出没有意义。
    #[test]
    fn a_refused_shutdown_does_not_wait() {
        let refused = serde_json::json!({
            "protocol": FERRY_IPC_PROTOCOL,
            "id": "host_daemon_shutdown",
            "ok": false,
            "error": {
                "code": "rpc.invalid_request",
                "params": {"reason": "app_mode", "message": "App 共享的引擎不接受 daemon.shutdown"},
            },
        });
        let mut engine = FakeEngine::start("refused", refused, false);
        let started = std::time::Instant::now();
        evict(&engine.socket);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "拒绝之后不该继续等"
        );
        assert_eq!(engine.requests().len(), 1);
        assert!(engine.socket.exists(), "被拒时不能动对方的 socket");
    }

    /// 没有人监听:连接失败即「没有 daemon 要赶」,立刻返回。
    #[test]
    fn evicting_an_empty_path_returns_immediately() {
        let dir = scratch_dir("empty");
        let socket = dir.join("e.sock");
        let started = std::time::Instant::now();
        evict(&socket);
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(wait_for_release(&socket, Duration::from_millis(10)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
