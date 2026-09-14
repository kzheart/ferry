//! CLI 拉起 Ferry Runtime 侧车的一次性桥。
//!
//! 桌面端常驻 runtime 由 Tauri 宿主管；CLI 没有宿主，`ferry title suggest` 这类
//! 需要模型的命令自己 fork 一个侧车，跑完就 kill。协议与宿主同一套：stdio JSONL
//! `{"protocol":"ferry-ipc/1","id","method","params"}`，先 `health` 校验
//! `contract_hash` 与本进程一致，再发真正的请求。
//!
//! 侧车定位三档：`FERRY_RUNTIME_BIN` → 解析 symlink 后 `current_exe()` 同目录的
//! `ferry-runtime`（Windows 为 `ferry-runtime.exe`）→ debug 构建下
//! `node <repo>/ferry-runtime/dist/server/server.js`。

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::contracts::ipc::{FERRY_CONTRACT_HASH, FERRY_IPC_PROTOCOL};
use crate::errors::DomainError;

/// 与桌面宿主相同：最多 5 批 × 2 次尝试 × 60 秒，另留传输余量。
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(5 * 2 * 60 + 30);
const HEALTH_TIMEOUT: Duration = Duration::from_secs(30);

/// 一个可执行的侧车候选。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Candidate {
    pub program: PathBuf,
    pub args: Vec<String>,
}

impl Candidate {
    fn binary(path: PathBuf) -> Self {
        Self {
            program: path,
            args: Vec::new(),
        }
    }

    fn node(script: PathBuf) -> Self {
        Self {
            program: PathBuf::from("node"),
            args: vec![script.to_string_lossy().into_owned()],
        }
    }

    pub fn describe(&self) -> String {
        let mut parts = vec![self.program.to_string_lossy().into_owned()];
        parts.extend(self.args.iter().cloned());
        parts.join(" ")
    }
}

fn unavailable(message: impl Into<String>, recovery: &str) -> DomainError {
    DomainError::engine_unavailable("runtime_unavailable", message, recovery)
}

/// 三档候选按优先级排列；`exe` 与 `repo` 由调用方注入以便测试。
pub fn candidates(
    env_bin: Option<&str>,
    exe: Option<&Path>,
    repo: Option<&Path>,
) -> Vec<Candidate> {
    let mut found = Vec::new();
    if let Some(bin) = env_bin.map(str::trim).filter(|text| !text.is_empty()) {
        found.push(Candidate::binary(PathBuf::from(bin)));
    }
    if let Some(dir) = exe.and_then(Path::parent) {
        found.push(Candidate::binary(
            dir.join(format!("ferry-runtime{}", std::env::consts::EXE_SUFFIX)),
        ));
    }
    if let Some(repo) = repo {
        found.push(Candidate::node(
            repo.join("ferry-runtime/dist/server/server.js"),
        ));
    }
    found
}

/// 仓库里的 dist 何时算候选：`FERRY_REPO` 显式指定；或 CLI 被软链到仓库 target 目录
/// 里的 release 产物（开发机常态），沿祖先目录能找到仓库根；debug 构建再退到编译期路径。
/// 正式包既无仓库也不会命中，仍只认兄弟侧车。
fn repository_root(exe: Option<&Path>) -> Option<PathBuf> {
    if let Ok(path) = std::env::var("FERRY_REPO") {
        return Some(PathBuf::from(path));
    }
    if let Some(root) = exe.and_then(repo_root_from_exe) {
        return Some(root);
    }
    if !cfg!(debug_assertions) {
        return None;
    }
    Some(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(".")),
    )
}

/// `<repo>/crates/ferry-engine/target/<triple>/release/ferry-engine` 往上最多七层。
pub fn repo_root_from_exe(exe: &Path) -> Option<PathBuf> {
    exe.ancestors()
        .skip(1)
        .take(7)
        .find(|dir| {
            dir.join("crates/ferry-engine/Cargo.toml").is_file()
                && dir.join("ferry-runtime/dist/server/server.js").is_file()
        })
        .map(Path::to_path_buf)
}

/// 本机实际可用的候选（`FERRY_RUNTIME_BIN` 原样保留，其余要求文件存在）。
pub fn resolved_candidates() -> Vec<Candidate> {
    let env_bin = std::env::var("FERRY_RUNTIME_BIN").ok();
    // current_exe 解 symlink：CLI 常被软链到 PATH 上，兄弟侧车在真实目录里。
    let exe = std::env::current_exe()
        .ok()
        .and_then(|path| std::fs::canonicalize(&path).ok().or(Some(path)));
    let repo = repository_root(exe.as_deref());
    candidates(env_bin.as_deref(), exe.as_deref(), repo.as_deref())
        .into_iter()
        .enumerate()
        .filter(|(index, candidate)| {
            *index == 0 && env_bin.is_some()
                || candidate
                    .args
                    .first()
                    .map(|script| Path::new(script).is_file())
                    .unwrap_or_else(|| candidate.program.is_file())
        })
        .map(|(_, candidate)| candidate)
        .collect()
}

struct Sidecar {
    child: Child,
    incoming: Receiver<std::io::Result<String>>,
    outgoing: Sender<Value>,
    sequence: u64,
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Sidecar {
    fn spawn(candidate: &Candidate) -> Result<Self, DomainError> {
        // 与桌面宿主共用 ~/.ferry 的模型配置；显式设了 FERRY_RUNTIME_DATA_DIR 则沿用（测试隔离）。
        let data_dir = std::env::var_os("FERRY_RUNTIME_DATA_DIR")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| crate::system::paths::expanduser("~/.ferry"));
        let mut command = Command::new(&candidate.program);
        command
            .args(&candidate.args)
            .env("FERRY_RUNTIME_DATA_DIR", &data_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command.spawn().map_err(|error| {
            unavailable(
                format!(
                    "启动 Ferry Runtime 失败（{}）: {error}",
                    candidate.describe()
                ),
                "确认已安装 Ferry Runtime，或用 FERRY_RUNTIME_BIN 指定它",
            )
        })?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| unavailable("Ferry Runtime stdout 不可用", "重试一次"))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| unavailable("Ferry Runtime stdin 不可用", "重试一次"))?;
        let (incoming_tx, incoming) = mpsc::sync_channel(16);
        let write_errors = incoming_tx.clone();
        // 管道读写都放在线程中；半行输出或侧车不读 stdin 均不能阻塞请求的截止时间。
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let failed = line.is_err();
                if incoming_tx.send(line).is_err() || failed {
                    return;
                }
            }
            let _ = incoming_tx.send(Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "Ferry Runtime 在应答前退出",
            )));
        });
        let (outgoing, requests) = mpsc::channel::<Value>();
        std::thread::spawn(move || {
            for request in requests {
                if let Err(error) = writeln!(stdin, "{request}").and_then(|()| stdin.flush()) {
                    let _ = write_errors.send(Err(error));
                    return;
                }
            }
        });
        Ok(Self {
            child,
            incoming,
            outgoing,
            sequence: 0,
        })
    }

    /// 发一条请求并读回同 id 的应答；事件与侧车自己发起的请求一概跳过。
    fn call(
        &mut self,
        method: &str,
        params: Value,
        deadline: Instant,
    ) -> Result<Value, DomainError> {
        self.sequence += 1;
        let id = format!("cli-title-{}-{}", std::process::id(), self.sequence);
        let request = json!({
            "protocol": FERRY_IPC_PROTOCOL, "id": id, "method": method, "params": params,
        });
        self.outgoing.send(request).map_err(|error| {
            unavailable(format!("写入 Ferry Runtime 失败: {error}"), "重试一次")
        })?;

        loop {
            if Instant::now() >= deadline {
                return Err(unavailable(
                    format!("等待 Ferry Runtime 的 {method} 应答超时"),
                    "稍后重试，或减少一次处理的会话数",
                ));
            }
            let line = match self
                .incoming
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(line) => line.map_err(|error| {
                    unavailable(format!("Ferry Runtime 通信失败: {error}"), "重试一次")
                })?,
                Err(RecvTimeoutError::Timeout) => {
                    return Err(unavailable(
                        format!("等待 Ferry Runtime 的 {method} 应答超时"),
                        "稍后重试，或减少一次处理的会话数",
                    ))
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(unavailable(
                        "Ferry Runtime 在应答前退出",
                        "检查 ~/.ferry 下的 runtime 日志",
                    ))
                }
            };
            let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
                continue;
            };
            // 侧车启动时会先问宿主一次 engine.request，答不上来它就不发 health。
            if value.get("type").and_then(Value::as_str) == Some("engine.request") {
                let reply = gateway_reply(&value);
                self.outgoing.send(reply).map_err(|error| {
                    unavailable(format!("写入 Ferry Runtime 失败: {error}"), "重试一次")
                })?;
                continue;
            }
            if value.get("id").and_then(Value::as_str) != Some(id.as_str()) {
                continue;
            }
            return Ok(value);
        }
    }
}

/// CLI 侧车是一次性的无状态生成器：不接管 Runtime 会话存储，也不能借 CLI socket
/// 转发 `runtime_sessions.*`（那些方法的 callers 不含 cli）。所以本地只答一条
/// 空清单让它启动完成，其余网关方法一律拒绝。
fn gateway_reply(event: &Value) -> Value {
    let payload = event.get("payload");
    let field = |key: &str| {
        payload
            .and_then(|payload| payload.get(key))
            .and_then(Value::as_str)
            .unwrap_or_default()
    };
    let mut params = json!({
        "request_id": field("request_id"),
        "session_id": event.get("session_id").and_then(Value::as_str).unwrap_or_default(),
    });
    if field("method") == "runtime_sessions.list" {
        params["ok"] = Value::Bool(true);
        params["result"] = json!([]);
    } else {
        params["ok"] = Value::Bool(false);
        params["error"] = Value::from("engine.method_not_allowed");
    }
    json!({
        "protocol": FERRY_IPC_PROTOCOL, "id": format!("cli-gateway-{}", field("request_id")),
        "method": "tool.result", "params": params,
    })
}

/// 握手校验：与宿主 `verify_handshake` 同三条（ok / service / contract_hash）。
pub fn verify_health(response: &Value, contract_hash: &str) -> Result<(), String> {
    if response.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err("health 响应未返回 ok".into());
    }
    if response.pointer("/result/service").and_then(Value::as_str) != Some("ferry-runtime") {
        return Err("health 响应的 service 不是 ferry-runtime".into());
    }
    if response
        .pointer("/result/contract_hash")
        .and_then(Value::as_str)
        != Some(contract_hash)
    {
        return Err("health 响应的契约哈希与本进程不一致".into());
    }
    Ok(())
}

/// 应答信封 → result；错误信封原样转成 DomainError 的载荷由调用方打印。
fn unwrap_result(response: Value, method: &str) -> Result<Value, Value> {
    if response.get("ok").and_then(Value::as_bool) == Some(true) {
        return Ok(response.get("result").cloned().unwrap_or(Value::Null));
    }
    Err(response.get("error").cloned().unwrap_or_else(|| {
        json!({"code": "runtime.invalid_response", "category": "internal",
               "retryable": false, "params": {"message": format!("{method} 应答形状非法")}})
    }))
}

/// 拉起侧车、握手、发一次请求、杀掉。失败时返回可直接打印的错误信封。
pub fn call_once(method: &str, params: Value) -> Result<Value, Value> {
    let found = resolved_candidates();
    let Some(candidate) = found.first() else {
        return Err(unavailable(
            "找不到 Ferry Runtime 侧车",
            "设置 FERRY_RUNTIME_BIN 指向 ferry-runtime，或先在仓库里构建 ferry-runtime/dist",
        )
        .payload());
    };
    let mut sidecar = Sidecar::spawn(candidate).map_err(|error| error.payload())?;
    let health = sidecar
        .call("health", json!({}), Instant::now() + HEALTH_TIMEOUT)
        .map_err(|error| error.payload())?;
    if let Err(reason) = verify_health(&health, FERRY_CONTRACT_HASH) {
        return Err(unavailable(
            format!("Ferry Runtime 协议握手失败: {reason}"),
            "确认 CLI 与 Ferry Runtime 是同一版本",
        )
        .payload());
    }
    let response = sidecar
        .call(method, params, Instant::now() + REQUEST_TIMEOUT)
        .map_err(|error| error.payload())?;
    unwrap_result(response, method)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn sidecar_round_trip_handles_gateway_requests_and_multiple_commands() {
        let pid = std::process::id();
        let health = json!({"id": format!("cli-title-{pid}-1"), "ok": true,
            "result": {"service": "ferry-runtime", "contract_hash": FERRY_CONTRACT_HASH}});
        let generated = json!({"id": format!("cli-title-{pid}-2"), "ok": true,
            "result": {"items": [{"ref": "fsr_a", "title": "新标题"}]}});
        let gateway = json!({"type": "engine.request", "session_id": "runtime",
            "payload": {"request_id": "r1", "method": "runtime_sessions.list"}});
        let candidate = Candidate {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(), format!(
                "read -r request; printf '%s\\n' '{gateway}'; read -r reply; printf '%s\\n' '{health}'; read -r request; printf '%s\\n' '{generated}'"
            )],
        };
        let mut sidecar = Sidecar::spawn(&candidate).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let response = sidecar.call("health", json!({}), deadline).unwrap();
        verify_health(&response, FERRY_CONTRACT_HASH).unwrap();
        let response = sidecar
            .call("title.generate", json!({"sessions": []}), deadline)
            .unwrap();
        assert_eq!(
            unwrap_result(response, "title.generate").unwrap()["items"][0]["title"],
            "新标题"
        );
    }

    #[cfg(unix)]
    #[test]
    fn stalled_sidecars_time_out_and_are_reaped_even_with_partial_output_or_blocked_input() {
        for output in ["", "printf '{'; "] {
            let candidate = Candidate {
                program: PathBuf::from("/bin/sh"),
                args: vec!["-c".into(), format!("{output}exec sleep 30")],
            };
            let mut sidecar = Sidecar::spawn(&candidate).unwrap();
            let pid = sidecar.child.id();
            let start = Instant::now();
            // 大于管道容量，验证不读 stdin 的侧车也不会阻塞截止时间。
            let error = sidecar
                .call(
                    "title.generate",
                    json!({"text": "x".repeat(256 * 1024)}),
                    start + Duration::from_millis(100),
                )
                .unwrap_err();
            assert!(error.message().contains("超时"));
            drop(sidecar);
            assert!(start.elapsed() < Duration::from_secs(3));
            assert_eq!(unsafe { libc::kill(pid as libc::pid_t, 0) }, -1);
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_sidecar_that_exits_before_reply_is_reported_without_waiting_for_timeout() {
        let candidate = Candidate {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(), "exit 0".into()],
        };
        let mut sidecar = Sidecar::spawn(&candidate).unwrap();
        let start = Instant::now();
        assert!(sidecar
            .call("health", json!({}), start + Duration::from_secs(30))
            .is_err());
        assert!(start.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn repo_root_is_found_from_a_release_binary_inside_the_repository() {
        let root = std::env::temp_dir().join(format!("ferry-bridge-{}", std::process::id()));
        let exe = root.join("crates/ferry-engine/target/aarch64-apple-darwin/release/ferry-engine");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::create_dir_all(root.join("ferry-runtime/dist/server")).unwrap();
        std::fs::write(root.join("crates/ferry-engine/Cargo.toml"), "").unwrap();
        std::fs::write(root.join("ferry-runtime/dist/server/server.js"), "").unwrap();
        assert_eq!(repo_root_from_exe(&exe), Some(root.clone()));
        assert_eq!(
            repo_root_from_exe(Path::new(
                "/Applications/Ferry.app/Contents/MacOS/ferry-engine"
            )),
            None
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn candidates_follow_the_documented_priority() {
        let exe = PathBuf::from("/Applications/Ferry.app/Contents/MacOS/ferry-engine");
        let repo = PathBuf::from("/src/ferry");
        let runtime_name = if cfg!(windows) {
            "ferry-runtime.exe"
        } else {
            "ferry-runtime"
        };
        let script = repo.join("ferry-runtime/dist/server/server.js");
        let found = candidates(Some("/custom/ferry-runtime"), Some(&exe), Some(&repo));
        assert_eq!(
            found,
            vec![
                Candidate::binary(PathBuf::from("/custom/ferry-runtime")),
                Candidate::binary(exe.with_file_name(runtime_name)),
                Candidate::node(script.clone()),
            ]
        );
        assert_eq!(found[2].describe(), format!("node {}", script.display()));
    }

    #[test]
    fn packaged_runtime_candidate_points_to_the_platform_binary() {
        let dir = tempfile::tempdir().unwrap();
        let (engine, runtime) = if cfg!(windows) {
            ("ferry-engine.exe", "ferry-runtime.exe")
        } else {
            ("ferry-engine", "ferry-runtime")
        };
        std::fs::write(dir.path().join(runtime), b"fixture").unwrap();
        let found = candidates(None, Some(&dir.path().join(engine)), None);
        assert_eq!(found.len(), 1);
        assert!(found[0].program.is_file());
    }

    #[test]
    fn each_source_is_optional() {
        assert!(candidates(None, None, None).is_empty());
        assert_eq!(candidates(Some("  "), None, None).len(), 0);
        assert_eq!(
            candidates(None, Some(Path::new("/bin/ferry")), None).len(),
            1
        );
    }

    #[test]
    fn the_local_gateway_answers_only_the_startup_session_listing() {
        let event = |method: &str| {
            json!({"type": "engine.request", "session_id": "runtime",
                   "payload": {"request_id": "r1", "method": method, "params": {}}})
        };
        let listed = gateway_reply(&event("runtime_sessions.list"));
        assert_eq!(listed["method"], json!("tool.result"));
        assert_eq!(listed["params"]["request_id"], json!("r1"));
        assert_eq!(listed["params"]["ok"], json!(true));
        assert_eq!(listed["params"]["result"], json!([]));

        let refused = gateway_reply(&event("agent_prompt"));
        assert_eq!(refused["params"]["ok"], json!(false));
        assert_eq!(
            refused["params"]["error"],
            json!("engine.method_not_allowed")
        );
    }

    #[test]
    fn handshake_requires_ok_service_and_matching_hash() {
        let good =
            json!({"ok": true, "result": {"service": "ferry-runtime", "contract_hash": "h"}});
        assert!(verify_health(&good, "h").is_ok());
        assert!(verify_health(&good, "other").is_err());
        assert!(verify_health(
            &json!({"ok": true, "result": {"service": "engine", "contract_hash": "h"}}),
            "h"
        )
        .is_err());
        assert!(verify_health(&json!({"ok": false, "result": {}}), "h").is_err());
        assert!(verify_health(&json!({}), "h").is_err());
    }

    #[test]
    fn result_is_unwrapped_and_error_envelopes_pass_through() {
        assert_eq!(
            unwrap_result(
                json!({"ok": true, "result": {"items": []}}),
                "title.generate"
            ),
            Ok(json!({"items": []}))
        );
        let error = json!({"code": "provider_unavailable", "category": "config",
                           "retryable": false, "params": {}});
        assert_eq!(
            unwrap_result(
                json!({"ok": false, "error": error.clone()}),
                "title.generate"
            ),
            Err(error)
        );
        assert!(unwrap_result(json!({"ok": false}), "title.generate")
            .unwrap_err()
            .get("code")
            .is_some());
    }
}
