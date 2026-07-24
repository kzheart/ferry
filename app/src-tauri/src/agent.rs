use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::Duration;
use tauri::{Emitter, Manager};

use crate::contracts::ipc::{FERRY_CONTRACT_HASH, FERRY_IPC_PROTOCOL};
use crate::sidecar::engine_request_blocking;

const MAX_COMMAND_BYTES: usize = 16 * 1024 * 1024;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const ORGANIZATION_TIMEOUT: Duration = Duration::from_secs(130);
const OPERATION_POLL_INTERVAL: Duration = Duration::from_millis(125);
const OPERATION_WAIT_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);
static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static AUTO_SESSIONS: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();

type PendingResult = Result<String, String>;
type Pending = Arc<Mutex<HashMap<String, mpsc::Sender<PendingResult>>>>;

struct AgentProcess {
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Pending,
}

impl Drop for AgentProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl AgentProcess {
    fn request(&mut self, request: &str, timeout: Duration) -> Result<String, String> {
        let value: Value = serde_json::from_str(request)
            .map_err(|error| format!("Agent 命令不是有效 JSON: {error}"))?;
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .ok_or("Agent 命令缺少 id")?
            .to_owned();
        let (sender, receiver) = mpsc::channel();
        self.pending
            .lock()
            .map_err(|_| "Agent pending 锁损坏".to_owned())?
            .insert(id.clone(), sender);
        if let Err(error) = write_line(&self.stdin, request) {
            self.pending.lock().ok().and_then(|mut map| map.remove(&id));
            return Err(error);
        }
        receiver.recv_timeout(timeout).map_err(|error| {
            self.pending.lock().ok().and_then(|mut map| map.remove(&id));
            format!("Agent 命令等待失败: {error}")
        })?
    }
}

static AGENT: OnceLock<Mutex<Option<AgentProcess>>> = OnceLock::new();

fn next_id(prefix: &str) -> String {
    format!(
        "{prefix}_{}_{}",
        std::process::id(),
        REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

fn write_line(stdin: &Arc<Mutex<ChildStdin>>, line: &str) -> Result<(), String> {
    let mut writer = stdin.lock().map_err(|_| "Agent stdin 锁损坏".to_owned())?;
    writer
        .write_all(line.as_bytes())
        .and_then(|_| writer.write_all(b"\n"))
        .and_then(|_| writer.flush())
        .map_err(|error| format!("写入 Agent 失败: {error}"))
}

fn spawn_agent(app: &tauri::AppHandle, resource_dir: &Path) -> Result<AgentProcess, String> {
    let mut command = agent_binary_command(resource_dir)?;
    command.env_clear();
    for name in [
        "PATH",
        "HOME",
        "USERPROFILE",
        "TMPDIR",
        "TEMP",
        "TMP",
        "SystemRoot",
        "WINDIR",
        "LANG",
        "LC_ALL",
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "NO_PROXY",
        "SSL_CERT_FILE",
        "NODE_EXTRA_CA_CERTS",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let data_dir = app
        .path()
        .home_dir()
        .map_err(|error| error.to_string())?
        .join(".ferry");
    std::fs::create_dir_all(&data_dir).map_err(|error| error.to_string())?;
    command.env("FERRY_AGENT_DATA_DIR", data_dir);
    crate::platform::configure_background_command(&mut command);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|error| format!("启动 Agent 失败: {error}"))?;
    let stdin = Arc::new(Mutex::new(child.stdin.take().ok_or("Agent stdin 不可用")?));
    let stdout = child.stdout.take().ok_or("Agent stdout 不可用")?;
    let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
    let reader_stdin = stdin.clone();
    let reader_pending = pending.clone();
    let reader_app = app.clone();
    let reader_resource = resource_dir.to_owned();
    std::thread::spawn(move || {
        read_agent_output(
            reader_app,
            reader_resource,
            BufReader::new(stdout),
            reader_stdin,
            reader_pending,
        )
    });
    Ok(AgentProcess {
        child,
        stdin,
        pending,
    })
}

fn read_agent_output(
    app: tauri::AppHandle,
    resource_dir: PathBuf,
    mut stdout: impl BufRead,
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Pending,
) {
    let mut line = String::new();
    loop {
        line.clear();
        match stdout.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }
        let trimmed = line.trim_end();
        let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        if value.get("type").is_some() {
            if value.get("type").and_then(Value::as_str) == Some("engine.request") {
                let worker_resource = resource_dir.clone();
                let worker_stdin = stdin.clone();
                std::thread::spawn(move || {
                    complete_engine_request(&worker_resource, &worker_stdin, &value)
                });
                continue;
            }
            if matches!(
                value.get("type").and_then(Value::as_str),
                Some("run.completed" | "run.failed" | "run.cancelled")
            ) {
                if let Some(session_id) = value.get("session_id").and_then(Value::as_str) {
                    forget_auto_policy(session_id);
                }
            }
            let _ = app.emit("ferry-agent-event", &value);
            if value.get("type").and_then(Value::as_str) == Some("tool.request") {
                let worker_app = app.clone();
                let worker_resource = resource_dir.clone();
                let worker_stdin = stdin.clone();
                std::thread::spawn(move || {
                    complete_tool_request(&worker_app, &worker_resource, &worker_stdin, &value)
                });
            }
            continue;
        }
        if let Some(id) = value.get("id").and_then(Value::as_str) {
            if let Some(sender) = pending.lock().ok().and_then(|mut map| map.remove(id)) {
                let _ = sender.send(Ok(trimmed.to_owned()));
            }
        }
    }
    if let Ok(mut waiters) = pending.lock() {
        for (_, sender) in waiters.drain() {
            let _ = sender.send(Err("Agent 进程已退出".to_owned()));
        }
    }
    let _ = app.emit(
        "ferry-agent-event",
        json!({"protocol": FERRY_IPC_PROTOCOL, "type": "runtime.disconnected"}),
    );
}

fn complete_tool_request(
    app: &tauri::AppHandle,
    resource_dir: &Path,
    stdin: &Arc<Mutex<ChildStdin>>,
    event: &Value,
) {
    let session_id = event
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let run_id = event.get("run_id").and_then(Value::as_str).unwrap_or("");
    let payload = event.get("payload").and_then(Value::as_object);
    let request_id = payload
        .and_then(|value| value.get("request_id"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let name = payload
        .and_then(|value| value.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let role_apply_policy = payload
        .and_then(|value| value.get("apply_policy"))
        .and_then(Value::as_str)
        .unwrap_or("manual");
    let args = payload
        .and_then(|value| value.get("args"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    // execute intent 与 metadata 变更由 Rust 审批边界决定；preview 只生成 plan。
    let mutation = is_mutating_tool(name, &args);
    let mut outcome = route_tool(resource_dir, name, args, run_id);
    if mutation {
        if let Ok(operation) = outcome.clone() {
            let auto = allows_auto_apply(auto_policy(session_id), role_apply_policy);
            if auto {
                match apply_routed_operation(resource_dir, &operation) {
                    Ok(result) => {
                        let _ = app.emit(
                            "ferry-agent-event",
                            json!({
                                "protocol": FERRY_IPC_PROTOCOL,
                                "session_id": session_id,
                                "run_id": run_id,
                                "type": "operation.applied",
                                "payload": { "tool": name, "operation": operation.clone(),
                                             "result": result, "auto": true },
                            }),
                        );
                        outcome = Ok(json!({"operation": operation, "status": "applied",
                                            "result": result}));
                    }
                    Err(code) => {
                        let _ = app.emit(
                            "ferry-agent-event",
                            json!({
                                "protocol": FERRY_IPC_PROTOCOL,
                                "session_id": session_id,
                                "run_id": run_id,
                                "type": "operation.failed",
                                "payload": { "tool": name, "operation": operation.clone(),
                                             "error": code, "auto": true },
                            }),
                        );
                        outcome = Err(code);
                    }
                }
            } else {
                let _ = app.emit(
                    "ferry-agent-event",
                    json!({
                        "protocol": FERRY_IPC_PROTOCOL,
                        "session_id": session_id,
                        "run_id": run_id,
                        "type": "operation.proposed",
                        "payload": { "tool": name, "operation": operation },
                    }),
                );
            }
        }
    }
    send_gateway_result(stdin, session_id, request_id, outcome);
}

fn send_gateway_result(
    stdin: &Arc<Mutex<ChildStdin>>,
    session_id: &str,
    request_id: &str,
    outcome: Result<Value, String>,
) {
    let params = match outcome {
        Ok(result) => json!({
            "request_id": request_id,
            "session_id": session_id,
            "ok": true,
            "result": result,
        }),
        Err(code) => json!({
            "request_id": request_id,
            "session_id": session_id,
            "ok": false,
            "error": code,
        }),
    };
    let command = json!({
        "protocol": FERRY_IPC_PROTOCOL,
        "id": next_id("tool_result"),
        "method": "tool.result",
        "params": params,
    });
    let _ = write_line(stdin, &command.to_string());
}

fn complete_engine_request(resource_dir: &Path, stdin: &Arc<Mutex<ChildStdin>>, event: &Value) {
    let session_id = event
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let payload = event.get("payload").and_then(Value::as_object);
    let request_id = payload
        .and_then(|value| value.get("request_id"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let method = payload
        .and_then(|value| value.get("method"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let params = payload
        .and_then(|value| value.get("params"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let outcome = if is_runtime_engine_method(method) {
        engine_value(resource_dir, method, params)
    } else {
        Err("engine.method_not_allowed".to_owned())
    };
    send_gateway_result(stdin, session_id, request_id, outcome);
}

fn is_runtime_engine_method(method: &str) -> bool {
    matches!(
        method,
        "session_backbone"
            | "session_summaries_set"
            | "organization_digest_context"
            | "organization_proposals_list"
            | "organization_propose"
            | "runtime_sessions.load_all"
            | "runtime_sessions.commit"
            | "runtime_sessions.delete"
    )
}

fn route_tool(
    resource_dir: &Path,
    name: &str,
    args: Map<String, Value>,
    run_id: &str,
) -> Result<Value, String> {
    let route =
        resolve_tool_request(name, &args, run_id).ok_or_else(|| "tool.not_allowed".to_owned())?;
    if route.requires_approval && run_id.is_empty() {
        return Err("agent.run_missing".to_owned());
    }
    let request = json!({
        "method": route.method,
        "params": route.params,
    });
    let response =
        engine_request_blocking(resource_dir, &request.to_string()).map_err(|error| {
            if error.contains("等待引擎响应失败") {
                "engine.timeout".to_owned()
            } else {
                "engine.unavailable".to_owned()
            }
        })?;
    let envelope: Value =
        serde_json::from_str(&response).map_err(|_| "engine.invalid_response".to_owned())?;
    if envelope.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(envelope.get("result").cloned().unwrap_or(Value::Null))
    } else {
        Err(structured_engine_error(&envelope))
    }
}

fn structured_engine_error(envelope: &Value) -> String {
    let error = envelope.get("error").and_then(Value::as_object);
    let params = error
        .and_then(|value| value.get("params"))
        .cloned()
        .filter(|value| value.to_string().len() <= 4096)
        .unwrap_or_else(|| json!({}));
    json!({
        "code": error.and_then(|value| value.get("code")).and_then(Value::as_str)
            .unwrap_or("engine.request_failed"),
        "category": error.and_then(|value| value.get("category")).and_then(Value::as_str)
            .unwrap_or("internal"),
        "retryable": error.and_then(|value| value.get("retryable")).and_then(Value::as_bool)
            .unwrap_or(false),
        "params": params,
    })
    .to_string()
}

#[derive(Debug, PartialEq)]
struct ToolRequestRoute {
    method: &'static str,
    params: Value,
    requires_approval: bool,
}

fn has_exact_keys(args: &Map<String, Value>, required: &[&str], optional: &[&str]) -> bool {
    required.iter().all(|key| args.contains_key(*key))
        && args
            .keys()
            .all(|key| required.contains(&key.as_str()) || optional.contains(&key.as_str()))
}

fn execution_intent(args: &Map<String, Value>) -> Option<bool> {
    match args.get("intent").and_then(Value::as_str) {
        Some("preview") => Some(false),
        Some("execute") => Some(true),
        _ => None,
    }
}

// Model 参数只在这里转换成固定 Engine 请求；intent 永远不会进入 Operation input。
fn resolve_tool_request(
    name: &str,
    args: &Map<String, Value>,
    _run_id: &str,
) -> Option<ToolRequestRoute> {
    let read = |method| ToolRequestRoute {
        method,
        params: Value::Object(args.clone()),
        requires_approval: false,
    };
    Some(match name {
        "session_search" => read("agent_search_sessions"),
        "session_read" => read("agent_session_read"),
        "usage" => read("agent_get_usage"),
        "migrate" => {
            if !has_exact_keys(
                args,
                &["source_tool", "ref", "target_tool", "intent"],
                &["max_turn"],
            ) {
                return None;
            }
            let execute = execution_intent(args)?;
            let mut input = Map::new();
            input.insert("kind".to_owned(), Value::String("migration".to_owned()));
            for key in ["source_tool", "ref", "target_tool"] {
                input.insert(key.to_owned(), args.get(key)?.clone());
            }
            if let Some(max_turn) = args.get("max_turn") {
                input.insert("max_turn".to_owned(), max_turn.clone());
            }
            input.insert("probe".to_owned(), Value::Bool(false));
            ToolRequestRoute {
                method: "operation.plan",
                params: json!({"input": Value::Object(input)}),
                requires_approval: execute,
            }
        }
        "session_edit" => match (args.contains_key("ops"), args.contains_key("patch")) {
            (true, false) => {
                if !has_exact_keys(args, &["tool", "ref", "ops", "intent"], &[]) {
                    return None;
                }
                let execute = execution_intent(args)?;
                ToolRequestRoute {
                    method: "operation.plan",
                    params: json!({"input": {
                        "kind": "edit",
                        "tool": args.get("tool")?,
                        "ref": args.get("ref")?,
                        "ops": args.get("ops")?,
                        "probe": false,
                    }}),
                    requires_approval: execute,
                }
            }
            (false, true) => {
                if !has_exact_keys(args, &["tool", "ref", "patch"], &[]) {
                    return None;
                }
                ToolRequestRoute {
                    method: "operation.plan",
                    params: json!({"input": {
                        "kind": "metadata",
                        "tool": args.get("tool")?,
                        "ref": args.get("ref")?,
                        "patch": args.get("patch")?,
                    }}),
                    requires_approval: true,
                }
            }
            _ => return None,
        },
        _ => return None,
    })
}

fn is_mutating_tool(name: &str, args: &Map<String, Value>) -> bool {
    resolve_tool_request(name, args, "mutation-check")
        .map(|route| route.requires_approval)
        .unwrap_or(false)
}

fn allows_auto_apply(prompt_auto: bool, role_policy: &str) -> bool {
    prompt_auto && role_policy == "auto"
}

fn request_agent(
    app: &tauri::AppHandle,
    resource_dir: &Path,
    request: &str,
) -> Result<String, String> {
    let slot = AGENT.get_or_init(|| Mutex::new(None));
    let mut guard = slot.lock().map_err(|_| "Agent 状态锁损坏".to_owned())?;
    let exited = guard
        .as_mut()
        .and_then(|process| process.child.try_wait().ok().flatten())
        .is_some();
    if exited {
        *guard = None;
    }
    if guard.is_none() {
        let mut candidate = spawn_agent(app, resource_dir)?;
        let health = json!({
            "protocol": FERRY_IPC_PROTOCOL,
            "id": next_id("health"),
            "method": "health",
            "params": {},
        });
        let response = candidate.request(&health.to_string(), Duration::from_secs(10))?;
        let value: Value = serde_json::from_str(&response).map_err(|e| e.to_string())?;
        if value.get("ok").and_then(Value::as_bool) != Some(true)
            || value.pointer("/result/service").and_then(Value::as_str) != Some("ferry-runtime")
            || value
                .pointer("/result/contract_hash")
                .and_then(Value::as_str)
                != Some(FERRY_CONTRACT_HASH)
        {
            return Err("Agent 协议握手失败".to_owned());
        }
        *guard = Some(candidate);
    }
    let timeout = serde_json::from_str::<Value>(request)
        .ok()
        .and_then(|value| {
            value
                .get("method")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .filter(|method| method == "organization.start")
        .map(|_| ORGANIZATION_TIMEOUT)
        .unwrap_or(COMMAND_TIMEOUT);
    let result = guard
        .as_mut()
        .expect("agent ensured")
        .request(request, timeout);
    if result.is_err() {
        *guard = None;
    }
    result
}

fn validate_public_command(request: &str) -> Result<(), String> {
    if request.len() > MAX_COMMAND_BYTES || request.contains('\n') || request.contains('\r') {
        return Err("Agent 命令 framing 非法".to_owned());
    }
    let value: Value = serde_json::from_str(request).map_err(|error| error.to_string())?;
    if value.get("protocol").and_then(Value::as_str) != Some(FERRY_IPC_PROTOCOL) {
        return Err("Agent 协议不兼容".to_owned());
    }
    let method = value.get("method").and_then(Value::as_str).unwrap_or("");
    if !matches!(
        method,
        "health"
            | "session.create"
            | "session.rename"
            | "session.pin"
            | "session.delete"
            | "roles.list"
            | "role.create"
            | "role.update"
            | "role.copy"
            | "role.delete"
            | "organization.start"
            | "prompt"
            | "abort"
            | "steer"
            | "follow_up"
            | "state"
            | "sessions.list"
            | "events.replay"
            | "providers.list"
            | "provider.enabled.set"
            | "provider.test"
            | "models.list"
            | "models.enabled"
            | "models.catalog"
            | "custom_model.add"
            | "custom_model.delete"
            | "models.visibility.set"
            | "models.refresh"
            | "config.get"
            | "credential.set"
            | "provider.logout"
            | "model.select"
            | "custom_provider.upsert"
            | "custom_provider.delete"
            | "auth.login.start"
            | "auth.login.respond"
            | "auth.login.cancel"
    ) {
        return Err("Agent 命令不允许从前端调用".to_owned());
    }
    Ok(())
}

fn engine_value(resource_dir: &Path, method: &str, params: Value) -> Result<Value, String> {
    let request = json!({"method": method, "params": params});
    let response = engine_request_blocking(resource_dir, &request.to_string())?;
    let envelope: Value = serde_json::from_str(&response).map_err(|e| e.to_string())?;
    if envelope.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(envelope.get("result").cloned().unwrap_or(Value::Null))
    } else {
        Err(structured_engine_error(&envelope))
    }
}

fn apply_operation_plan(resource_dir: &Path, plan_id: &str) -> Result<Value, String> {
    let accepted = engine_value(resource_dir, "operation.apply", json!({"plan_id": plan_id}))?;
    wait_for_operation(resource_dir, plan_id, accepted)
}

/// Runtime 工具调用需要一个最终工具结果；它只轮询 Engine 状态，绝不把长写
/// 操作维持在同一个 IPC request 上。
fn wait_for_operation(
    resource_dir: &Path,
    plan_id: &str,
    mut status: Value,
) -> Result<Value, String> {
    let deadline = std::time::Instant::now() + OPERATION_WAIT_TIMEOUT;
    loop {
        match status.get("status").and_then(Value::as_str) {
            Some("applied") => return Ok(status.get("result").cloned().unwrap_or(Value::Null)),
            Some("failed" | "cancelled" | "expired") => {
                let error_type = status
                    .get("error_type")
                    .and_then(Value::as_str)
                    .unwrap_or("OperationNotApplied");
                return Err(format!("operation.{error_type}"));
            }
            Some("queued" | "applying") if std::time::Instant::now() < deadline => {
                std::thread::sleep(OPERATION_POLL_INTERVAL);
                status = engine_value(
                    resource_dir,
                    "operation.status",
                    json!({"plan_id": plan_id}),
                )?;
            }
            Some(_) => return Err("operation.invalid_status".to_owned()),
            None => return Err("operation.invalid_status".to_owned()),
        }
    }
}

fn operation_plan_id(operation: &Value) -> Result<&str, String> {
    operation
        .get("plan_id")
        .and_then(Value::as_str)
        .filter(|plan_id| plan_id.starts_with("op_"))
        .ok_or_else(|| "Engine 未返回可审批的 operation plan_id".to_owned())
}

fn apply_routed_operation(resource_dir: &Path, operation: &Value) -> Result<Value, String> {
    let plan_id = operation_plan_id(operation)?;
    apply_operation_plan(resource_dir, plan_id)
}

fn remember_auto_policy(request: &str) {
    let Ok(value) = serde_json::from_str::<Value>(request) else {
        return;
    };
    let method = value.get("method").and_then(Value::as_str).unwrap_or("");
    // 自动策略绑定一次完整 run；steer/follow_up 不应在运行中途改写它。
    if method != "prompt" {
        return;
    }
    let Some(params) = value.get("params") else {
        return;
    };
    let Some(session_id) = params.get("session_id").and_then(Value::as_str) else {
        return;
    };
    let auto = params
        .get("auto_apply")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if let Ok(mut sessions) = AUTO_SESSIONS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
    {
        sessions.insert(session_id.to_owned(), auto);
    }
}

fn auto_policy(session_id: &str) -> bool {
    AUTO_SESSIONS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .ok()
        .and_then(|sessions| sessions.get(session_id).copied())
        .unwrap_or(false)
}

fn forget_auto_policy(session_id: &str) {
    if let Ok(mut sessions) = AUTO_SESSIONS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
    {
        sessions.remove(session_id);
    }
}

#[tauri::command]
pub(crate) async fn agent_command(
    app: tauri::AppHandle,
    request: String,
) -> Result<String, String> {
    validate_public_command(&request)?;
    remember_auto_policy(&request);
    let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || request_agent(&app, &resource_dir, &request))
        .await
        .map_err(|e| e.to_string())?
}

pub(crate) fn warm_up(app: tauri::AppHandle, resource_dir: PathBuf) {
    std::thread::spawn(move || {
        let request = json!({"protocol": FERRY_IPC_PROTOCOL, "id": next_id("warmup"),
                             "method": "health", "params": {}});
        let _ = request_agent(&app, &resource_dir, &request.to_string());
    });
}

#[cfg(debug_assertions)]
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn agent_binary_command(resource_dir: &Path) -> Result<Command, String> {
    let name = if cfg!(target_os = "windows") {
        "ferry-agent.exe"
    } else {
        "ferry-agent"
    };
    let candidates = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join(name)))
        .into_iter()
        .chain(std::iter::once(resource_dir.join(name)))
        .collect::<Vec<_>>();
    if let Some(binary) = candidates.iter().find(|path| path.is_file()) {
        return Ok(Command::new(binary));
    }
    #[cfg(debug_assertions)]
    {
        let mut command = Command::new("node");
        command.arg(repo_root().join("agent-runtime/dist/cli.js"));
        command.current_dir(repo_root());
        Ok(command)
    }
    #[cfg(not(debug_assertions))]
    Err(format!(
        "正式包缺少 Agent sidecar: {}",
        candidates
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("; ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_gateway_is_an_exact_allowlist() {
        let map = |value: Value| value.as_object().cloned().unwrap_or_default();
        let read = resolve_tool_request("session_read", &map(json!({"ref": "fsr_a"})), "").unwrap();
        assert_eq!(read.method, "agent_session_read");
        assert!(!read.requires_approval);

        let migrate_preview = resolve_tool_request(
            "migrate",
            &map(json!({
                "source_tool": "claude",
                "ref": "fsr_a",
                "target_tool": "codex",
                "max_turn": 3,
                "intent": "preview",
            })),
            "",
        )
        .unwrap();
        assert_eq!(migrate_preview.method, "operation.plan");
        assert!(!migrate_preview.requires_approval);
        assert_eq!(
            migrate_preview.params,
            json!({"input": {
                "kind": "migration",
                "source_tool": "claude",
                "ref": "fsr_a",
                "target_tool": "codex",
                "max_turn": 3,
                "probe": false,
            }})
        );

        let migrate_execute = resolve_tool_request(
            "migrate",
            &map(json!({
                "source_tool": "claude",
                "ref": "fsr_a",
                "target_tool": "codex",
                "intent": "execute",
            })),
            "run-1",
        )
        .unwrap();
        assert_eq!(migrate_execute.method, "operation.plan");
        assert!(migrate_execute.requires_approval);
        assert!(migrate_execute.params.pointer("/input/intent").is_none());

        let edit_preview = resolve_tool_request(
            "session_edit",
            &map(json!({
                "tool": "claude",
                "ref": "fsr_a",
                "ops": [{"op": "delete-turn", "turn": 1}],
                "intent": "preview",
            })),
            "",
        )
        .unwrap();
        assert_eq!(edit_preview.method, "operation.plan");
        assert!(!edit_preview.requires_approval);
        assert_eq!(
            edit_preview.params,
            json!({"input": {
                "kind": "edit",
                "tool": "claude",
                "ref": "fsr_a",
                "ops": [{"op": "delete-turn", "turn": 1}],
                "probe": false,
            }})
        );

        let edit_execute = resolve_tool_request(
            "session_edit",
            &map(json!({
                "tool": "claude",
                "ref": "fsr_a",
                "ops": [{"op": "delete-turn", "turn": 1}],
                "intent": "execute",
            })),
            "run-1",
        )
        .unwrap();
        assert!(edit_execute.requires_approval);
        assert!(edit_execute.params.pointer("/input/intent").is_none());

        let metadata = resolve_tool_request(
            "session_edit",
            &map(json!({
                "tool": "claude",
                "ref": "fsr_a",
                "patch": {"pinned": true},
            })),
            "run-1",
        )
        .unwrap();
        assert_eq!(metadata.method, "operation.plan");
        assert!(metadata.requires_approval);
        assert_eq!(
            metadata.params,
            json!({"input": {
                "kind": "metadata",
                "tool": "claude",
                "ref": "fsr_a",
                "patch": {"pinned": true},
            }})
        );

        assert_eq!(
            resolve_tool_request(
                "session_edit",
                &map(json!({
                    "tool": "claude",
                    "ref": "fsr_a",
                    "ops": [],
                    "patch": {"pinned": true},
                    "intent": "execute",
                })),
                "run-1",
            ),
            None
        );
        assert_eq!(
            resolve_tool_request(
                "migrate",
                &map(json!({
                    "source_tool": "claude",
                    "ref": "fsr_a",
                    "target_tool": "codex",
                    "intent": "execute",
                    "method": "operation.apply",
                })),
                "run-1",
            ),
            None
        );
        assert_eq!(
            resolve_tool_request("operation.apply", &map(json!({})), ""),
            None
        );
        assert_eq!(resolve_tool_request("shell", &map(json!({})), ""), None);
    }

    #[test]
    fn preview_never_requires_approval_and_execute_always_does() {
        for name in ["migrate", "session_edit"] {
            let base = if name == "migrate" {
                json!({"source_tool": "claude", "ref": "fsr_a",
                       "target_tool": "codex"})
            } else {
                json!({"tool": "claude", "ref": "fsr_a",
                       "ops": [{"op": "delete-turn", "turn": 1}]})
            };
            let mut preview = base.as_object().unwrap().clone();
            preview.insert("intent".to_owned(), Value::String("preview".to_owned()));
            assert!(!is_mutating_tool(name, &preview));
            let mut execute = base.as_object().unwrap().clone();
            execute.insert("intent".to_owned(), Value::String("execute".to_owned()));
            assert!(is_mutating_tool(name, &execute));
        }
    }

    #[test]
    fn operation_plans_require_a_plan_id() {
        assert_eq!(
            operation_plan_id(&json!({"plan_id": "op_fixture"})).unwrap(),
            "op_fixture"
        );
        assert!(operation_plan_id(&json!({"id": "wrong_fixture"})).is_err());
        assert!(operation_plan_id(&json!({})).is_err());
    }

    #[test]
    fn frontend_cannot_submit_tool_results() {
        let request = json!({"protocol": FERRY_IPC_PROTOCOL, "id": "x",
                             "method": "tool.result", "params": {}});
        assert!(validate_public_command(&request.to_string()).is_err());
    }

    #[test]
    fn runtime_engine_gateway_is_an_exact_allowlist() {
        for method in [
            "session_backbone",
            "session_summaries_set",
            "organization_digest_context",
            "organization_proposals_list",
            "organization_propose",
        ] {
            assert!(is_runtime_engine_method(method));
        }
        for method in [
            "runtime_sessions.load_all",
            "runtime_sessions.commit",
            "runtime_sessions.delete",
        ] {
            assert!(is_runtime_engine_method(method));
        }
        assert!(!is_runtime_engine_method("operation.apply"));
        assert!(!is_runtime_engine_method("session_delete"));
    }

    #[test]
    fn engine_errors_keep_safe_recovery_details() {
        let envelope = json!({"ok": false, "error": {
            "code": "agent.reference_invalid", "category": "validation",
            "retryable": false, "params": {"field": "locator", "hint": "read context"}
        }});
        let error: Value = serde_json::from_str(&structured_engine_error(&envelope)).unwrap();
        assert_eq!(error["code"], "agent.reference_invalid");
        assert_eq!(error["params"]["hint"], "read context");
    }

    #[test]
    fn automatic_apply_policy_is_bound_to_the_prompt_run() {
        let session_id = "test-auto-policy-prompt-run";
        forget_auto_policy(session_id);
        remember_auto_policy(
            &json!({"method": "prompt", "params": {
                "session_id": session_id, "auto_apply": true
            }})
            .to_string(),
        );
        assert!(auto_policy(session_id));

        remember_auto_policy(
            &json!({"method": "follow_up", "params": {
                "session_id": session_id, "auto_apply": false
            }})
            .to_string(),
        );
        assert!(auto_policy(session_id));
        forget_auto_policy(session_id);
        assert!(!auto_policy(session_id));
    }

    #[test]
    fn role_manual_policy_cannot_be_overridden_by_prompt_auto_mode() {
        assert!(!allows_auto_apply(true, "manual"));
        assert!(allows_auto_apply(true, "auto"));
        assert!(!allows_auto_apply(false, "auto"));
    }
}
