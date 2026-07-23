use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{mpsc, Mutex, OnceLock};
use std::time::Duration;

const ENGINE_PROTOCOL: u64 = 2;
const ENGINE_TIMEOUT: Duration = Duration::from_secs(120);
const AGENT_LOOKUP_TIMEOUT: Duration = Duration::from_secs(20);
const ENGINE_COMMIT_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);

/// 常驻引擎进程:按行请求/响应,避免每次 RPC 冷启动(release 下 PyInstaller 解压开销显著)。
struct EngineProcess {
    child: Child,
    stdin: ChildStdin,
    responses: mpsc::Receiver<Result<String, String>>,
}

impl Drop for EngineProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

static ENGINE: OnceLock<Mutex<Option<EngineProcess>>> = OnceLock::new();

fn spawn_engine(resource_dir: &Path) -> Result<EngineProcess, String> {
    let mut command = engine_command(resource_dir)?;
    command.arg("serve");
    hide_console(&mut command);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|error| format!("启动引擎失败: {error}"))?;
    let stdin = child.stdin.take().ok_or("引擎 stdin 不可用")?;
    let stdout = child.stdout.take().ok_or("引擎 stdout 不可用")?;
    let (sender, responses) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if sender.send(Ok(line.trim_end().to_owned())).is_err() {
                        return;
                    }
                }
                Err(error) => {
                    let _ = sender.send(Err(format!("读取引擎失败: {error}")));
                    return;
                }
            }
        }
        let _ = sender.send(Err("引擎进程已退出".to_owned()));
    });
    let mut engine = EngineProcess {
        child,
        stdin,
        responses,
    };
    handshake(&mut engine)?;
    Ok(engine)
}

/// 协议握手作为常驻进程的首条请求完成:独立的一次性 health 子进程
/// 在 release 下会让 PyInstaller onefile 多解压一整次,冷启动时间翻倍。
fn handshake(engine: &mut EngineProcess) -> Result<(), String> {
    let line = engine
        .exchange(r#"{"method":"health"}"#, Duration::from_secs(15))
        .map_err(|error| format!("引擎健康检查失败: {error}"))?;
    let health: Value = serde_json::from_str(&line)
        .map_err(|error| format!("引擎健康检查返回无效 JSON: {error}"))?;
    let protocol = health.get("protocol").and_then(Value::as_u64);
    if health.get("ok").and_then(Value::as_bool) != Some(true) || protocol != Some(ENGINE_PROTOCOL)
    {
        return Err(format!(
            "引擎协议不兼容: 需要 {ENGINE_PROTOCOL}，实际 {protocol:?}"
        ));
    }
    Ok(())
}

impl EngineProcess {
    fn exchange(&mut self, request: &str, timeout: Duration) -> Result<String, String> {
        self.stdin
            .write_all(request.as_bytes())
            .and_then(|_| self.stdin.write_all(b"\n"))
            .and_then(|_| self.stdin.flush())
            .map_err(|error| format!("写入引擎失败: {error}"))?;
        self.responses
            .recv_timeout(timeout)
            .map_err(|error| format!("等待引擎响应失败: {error}"))?
    }
}

pub(crate) fn engine_request_blocking(
    resource_dir: &Path,
    request: &str,
) -> Result<String, String> {
    let timeout = request_timeout(request);
    let slot = ENGINE.get_or_init(|| Mutex::new(None));
    let mut guard = slot.lock().map_err(|_| "引擎状态锁损坏".to_owned())?;
    let mut last_error = String::new();
    for _attempt in 0..request_attempts(request) {
        if guard.is_none() {
            *guard = Some(spawn_engine(resource_dir)?);
        }
        let engine = guard.as_mut().expect("engine just ensured");
        let exchange = engine.exchange(request, timeout);
        match exchange {
            Ok(line) => return Ok(line),
            Err(error) => {
                last_error = error;
                *guard = None; // Drop 会回收进程,下一轮重启
            }
        }
    }
    Err(format!("引擎通信失败: {last_error}"))
}

fn request_timeout(request: &str) -> Duration {
    let value = serde_json::from_str::<Value>(request).ok();
    let method = value
        .as_ref()
        .and_then(|value| value.get("method").and_then(Value::as_str));
    if matches!(method, Some("agent_operation_apply" | "operation.apply")) {
        ENGINE_COMMIT_TIMEOUT
    } else if method == Some("migrate") {
        let dry_run = value
            .as_ref()
            .and_then(|value| value.get("params"))
            .and_then(|params| params.get("dry_run"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if dry_run {
            ENGINE_TIMEOUT
        } else {
            ENGINE_COMMIT_TIMEOUT
        }
    } else if matches!(
        method,
        Some("agent_search_sessions" | "agent_session_read" | "agent_get_usage")
    ) {
        AGENT_LOOKUP_TIMEOUT
    } else {
        ENGINE_TIMEOUT
    }
}

fn request_attempts(request: &str) -> u8 {
    let method = serde_json::from_str::<Value>(request)
        .ok()
        .and_then(|value| {
            value
                .get("method")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
    if method
        .as_deref()
        .is_some_and(|name| name.starts_with("agent_"))
        || matches!(
            method.as_deref(),
            Some("migrate" | "operation.plan" | "operation.apply" | "operation.cancel")
        )
    {
        1
    } else {
        2
    }
}

/// 引擎仓库根目录:优先 FERRY_REPO 环境变量,
/// 否则取本 crate 上两级(app/src-tauri → 仓库根,开发形态)。
#[cfg(debug_assertions)]
fn repo_root() -> PathBuf {
    if let Ok(p) = std::env::var("FERRY_REPO") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from("."))
}

/// externalBin 在 macOS 上被放进 Contents/MacOS(主程序旁),Windows 上在安装根目录;
/// 依次尝试可执行文件所在目录与 resource_dir,取第一个存在的。
fn bundled_engine_candidates(resource_dir: &Path) -> Vec<PathBuf> {
    let name = bundled_engine_name(cfg!(target_os = "windows"));
    let mut candidates = Vec::new();
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    {
        candidates.push(exe_dir.join(name));
    }
    candidates.push(resource_dir.join(name));
    candidates
}

fn bundled_engine_name(is_windows: bool) -> &'static str {
    if is_windows {
        "ferry-engine.exe"
    } else {
        "ferry-engine"
    }
}

fn engine_command(resource_dir: &Path) -> Result<Command, String> {
    let candidates = bundled_engine_candidates(resource_dir);
    if let Some(sidecar) = candidates.iter().find(|path| path.is_file()) {
        return Ok(Command::new(sidecar));
    }

    #[cfg(debug_assertions)]
    {
        let mut command = Command::new(if cfg!(target_os = "windows") {
            "python"
        } else {
            "python3"
        });
        command.args(["-m", "engine.api"]);
        command.current_dir(repo_root());
        Ok(command)
    }

    #[cfg(not(debug_assertions))]
    Err(format!(
        "正式包缺少引擎 sidecar,已尝试: {}",
        candidates
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("; ")
    ))
}

#[cfg(target_os = "windows")]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x0800_0000);
}

#[cfg(not(target_os = "windows"))]
fn hide_console(_command: &mut Command) {}

/// 应用启动即预热常驻引擎:PyInstaller 解压与 webview 启动并行,
/// 首个前端 RPC 到达时引擎大概率已就绪。失败静默,错误会在首个真实 RPC 上重现。
pub(crate) fn warm_up(resource_dir: PathBuf) {
    std::thread::spawn(move || {
        let _ = engine_request_blocking(&resource_dir, r#"{"method":"health"}"#);
    });
}

#[tauri::command]
pub(crate) async fn engine_rpc(app: tauri::AppHandle, request: String) -> Result<String, String> {
    use tauri::Manager;
    validate_public_engine_request(&request)?;
    let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || engine_request_blocking(&resource_dir, &request))
        .await
        .map_err(|e| e.to_string())?
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MigrationInput {
    src: String,
    dst: String,
    reference: String,
    max_turn: Option<u32>,
    #[serde(default)]
    probe: bool,
    probe_model: Option<String>,
}

fn validate_migration_input(input: &MigrationInput) -> Result<(), String> {
    for (label, value) in [("源工具", &input.src), ("目标工具", &input.dst)] {
        if value.is_empty()
            || value.len() > 64
            || !value.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
        {
            return Err(format!("{label} 标识无效"));
        }
    }
    if input.reference.is_empty()
        || input.reference.len() > 16 * 1024
        || input.reference.contains('\0')
    {
        return Err("会话引用无效".to_owned());
    }
    if input.max_turn == Some(0) || input.max_turn.is_some_and(|turn| turn > 100_000) {
        return Err("迁移轮数无效".to_owned());
    }
    if input
        .probe_model
        .as_ref()
        .is_some_and(|model| model.len() > 512 || model.chars().any(char::is_control))
    {
        return Err("探针模型标识无效".to_owned());
    }
    Ok(())
}

fn migration_request(input: &MigrationInput, dry_run: bool) -> Result<String, String> {
    validate_migration_input(input)?;
    serde_json::to_string(&json!({
        "method": "migrate",
        "params": {
            "src": input.src,
            "dst": input.dst,
            "ref": input.reference,
            "max_turn": input.max_turn,
            "dry_run": dry_run,
            "probe": !dry_run && input.probe,
            "probe_model": if dry_run { None } else { input.probe_model.as_deref() },
        }
    }))
    .map_err(|error| error.to_string())
}

async fn migration_engine_request(
    app: tauri::AppHandle,
    input: MigrationInput,
    dry_run: bool,
) -> Result<String, String> {
    let request = migration_request(&input, dry_run)?;
    use tauri::Manager;
    let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || engine_request_blocking(&resource_dir, &request))
        .await
        .map_err(|e| e.to_string())?
}

/// 只读预演：强制 dry_run，不接受 cwd 或其它引擎参数。
#[tauri::command]
pub(crate) async fn migration_preview(
    app: tauri::AppHandle,
    input: MigrationInput,
) -> Result<String, String> {
    migration_engine_request(app, input, true).await
}

/// 用户在确认页明确提交后才会调用，且只接受白名单字段。
#[tauri::command]
pub(crate) async fn migration_commit(
    app: tauri::AppHandle,
    input: MigrationInput,
) -> Result<String, String> {
    migration_engine_request(app, input, false).await
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OperationPlanInput {
    kind: String,
    tool: String,
    #[serde(rename = "ref")]
    reference: String,
    ops: Vec<Value>,
    #[serde(default)]
    probe: bool,
}

fn validate_operation_plan_input(input: &OperationPlanInput) -> Result<(), String> {
    if input.kind != "edit" {
        return Err("当前 Operation 仅允许 edit".to_owned());
    }
    if !matches!(input.tool.as_str(), "claude" | "codex" | "opencode") {
        return Err("Operation 工具标识无效".to_owned());
    }
    if input.reference.is_empty()
        || input.reference.len() > 512
        || input.reference.chars().any(char::is_control)
    {
        return Err("Operation 会话引用无效".to_owned());
    }
    if input.ops.is_empty() || input.ops.len() > 50 {
        return Err("Operation ops 必须包含 1 到 50 项".to_owned());
    }
    let mut rewrite_locators = HashSet::new();
    for operation in &input.ops {
        let fields = operation
            .as_object()
            .ok_or_else(|| "Operation edit op 必须是 object".to_owned())?;
        match fields.get("op").and_then(Value::as_str) {
            Some("delete-turn") => {
                if fields.len() != 2
                    || !fields.contains_key("turn")
                    || fields
                        .get("turn")
                        .and_then(Value::as_u64)
                        .is_none_or(|turn| turn == 0)
                {
                    return Err("Operation delete-turn 参数无效".to_owned());
                }
            }
            Some("rewrite") => {
                if fields.len() != 3
                    || !fields.contains_key("locator")
                    || !fields.contains_key("text")
                {
                    return Err("Operation rewrite 参数无效".to_owned());
                }
                let locator = fields
                    .get("locator")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "Operation rewrite locator 无效".to_owned())?;
                let text = fields
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "Operation rewrite text 无效".to_owned())?;
                if !locator.starts_with("fml_")
                    || locator.len() > 512
                    || locator.chars().any(char::is_control)
                    || text.is_empty()
                    || text.chars().count() > 20_000
                {
                    return Err("Operation rewrite locator/text 无效".to_owned());
                }
                if !rewrite_locators.insert(locator) {
                    return Err("Operation 不允许重复 rewrite locator".to_owned());
                }
            }
            _ => return Err("Operation edit op 不受支持".to_owned()),
        }
    }
    let encoded = serde_json::to_vec(&input.ops).map_err(|error| error.to_string())?;
    if encoded.len() > 64 * 1024 {
        return Err("Operation ops 超过 64 KiB".to_owned());
    }
    Ok(())
}

fn validate_plan_id(plan_id: &str) -> Result<(), String> {
    if !(8..=128).contains(&plan_id.len())
        || !plan_id.starts_with("op_")
        || !plan_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err("Operation plan_id 无效".to_owned());
    }
    Ok(())
}

fn operation_plan_request(input: &OperationPlanInput) -> Result<String, String> {
    validate_operation_plan_input(input)?;
    serde_json::to_string(&json!({
        "method": "operation.plan",
        "params": {"input": input},
    }))
    .map_err(|error| error.to_string())
}

fn operation_id_request(method: &'static str, plan_id: &str) -> Result<String, String> {
    validate_plan_id(plan_id)?;
    if !matches!(
        method,
        "operation.apply" | "operation.status" | "operation.cancel"
    ) {
        return Err("Operation Engine 方法无效".to_owned());
    }
    serde_json::to_string(&json!({
        "method": method,
        "params": {"plan_id": plan_id},
    }))
    .map_err(|error| error.to_string())
}

async fn operation_engine_request(
    app: tauri::AppHandle,
    request: String,
) -> Result<String, String> {
    use tauri::Manager;
    let resource_dir = app
        .path()
        .resource_dir()
        .map_err(|error| error.to_string())?;
    tauri::async_runtime::spawn_blocking(move || engine_request_blocking(&resource_dir, &request))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn operation_plan(
    app: tauri::AppHandle,
    input: OperationPlanInput,
) -> Result<String, String> {
    operation_engine_request(app, operation_plan_request(&input)?).await
}

/// 此命令只接受已经生成的 plan_id；业务参数不会在应用阶段再次进入 Engine。
#[tauri::command]
pub(crate) async fn operation_apply(
    app: tauri::AppHandle,
    plan_id: String,
) -> Result<String, String> {
    operation_engine_request(app, operation_id_request("operation.apply", &plan_id)?).await
}

#[tauri::command]
pub(crate) async fn operation_status(
    app: tauri::AppHandle,
    plan_id: String,
) -> Result<String, String> {
    operation_engine_request(app, operation_id_request("operation.status", &plan_id)?).await
}

#[tauri::command]
pub(crate) async fn operation_cancel(
    app: tauri::AppHandle,
    plan_id: String,
) -> Result<String, String> {
    operation_engine_request(app, operation_id_request("operation.cancel", &plan_id)?).await
}

fn validate_public_engine_request(request: &str) -> Result<(), String> {
    let value: Value = serde_json::from_str(request)
        .map_err(|error| format!("Engine 请求不是有效 JSON: {error}"))?;
    let method = value.get("method").and_then(Value::as_str).unwrap_or("");
    if !matches!(
        method,
        "health"
            | "version"
            | "scan"
            | "env"
            | "tools"
            | "resume"
            | "models"
            | "history"
            | "history_delete"
            | "pricing"
            | "show"
            | "session_asset"
            | "authoring_capabilities"
            | "authoring_preview"
            | "edit_capabilities"
            | "edit_preview"
            | "session_meta_list"
            | "session_backbone"
            | "session_summaries_set"
            | "agent_search_sessions"
            | "agent_session_read"
            | "agent_get_usage"
            | "agent_preview_migration"
            | "agent_preview_edit"
    ) {
        return Err("该 Engine 方法不允许从通用前端 RPC 调用".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        migration_request, operation_id_request, operation_plan_request, request_attempts,
        request_timeout, validate_migration_input, validate_operation_plan_input, validate_plan_id,
        validate_public_engine_request, MigrationInput, OperationPlanInput, AGENT_LOOKUP_TIMEOUT,
        ENGINE_COMMIT_TIMEOUT, ENGINE_TIMEOUT,
    };

    #[test]
    fn sensitive_agent_methods_are_not_generic_rpc_methods() {
        assert!(validate_public_engine_request(r#"{"method":"agent_operation_apply"}"#).is_err());
        assert!(validate_public_engine_request(r#"{"method":"operation.apply"}"#).is_err());
        assert!(validate_public_engine_request(r#"{"method":"operation.plan"}"#).is_err());
        assert!(validate_public_engine_request(r#"{"method":"operation.status"}"#).is_err());
        assert!(validate_public_engine_request(r#"{"method":"operation.cancel"}"#).is_err());
        assert!(validate_public_engine_request(r#"{"method":"edit_apply"}"#).is_err());
        assert!(validate_public_engine_request(r#"{"method":"migrate"}"#).is_err());
        assert!(validate_public_engine_request(r#"{"method":"scan"}"#).is_ok());
        // 删除迁移记录只动 Ferry 自己的历史文件,不写目标工具的会话
        assert!(validate_public_engine_request(r#"{"method":"history_delete"}"#).is_ok());
    }

    #[test]
    fn mutation_commit_is_not_killed_by_normal_rpc_timeout() {
        assert_eq!(
            request_timeout(r#"{"method":"agent_operation_apply"}"#),
            ENGINE_COMMIT_TIMEOUT
        );
        assert_eq!(
            request_timeout(r#"{"method":"migrate","params":{"dry_run":true}}"#),
            ENGINE_TIMEOUT
        );
        assert_eq!(
            request_timeout(r#"{"method":"migrate"}"#),
            ENGINE_COMMIT_TIMEOUT
        );
        assert_eq!(request_attempts(r#"{"method":"migrate"}"#), 1);
        assert_eq!(
            request_timeout(r#"{"method":"operation.apply"}"#),
            ENGINE_COMMIT_TIMEOUT
        );
        assert_eq!(request_attempts(r#"{"method":"operation.apply"}"#), 1);
        assert_eq!(request_attempts(r#"{"method":"operation.plan"}"#), 1);
        assert_eq!(request_attempts(r#"{"method":"operation.cancel"}"#), 1);
    }

    #[test]
    fn agent_lookups_have_one_short_deadline() {
        let request = r#"{"method":"agent_search_sessions"}"#;
        assert_eq!(request_timeout(request), AGENT_LOOKUP_TIMEOUT);
        assert_eq!(request_attempts(request), 1);
    }

    fn migration_input() -> MigrationInput {
        MigrationInput {
            src: "claude".to_owned(),
            dst: "codex".to_owned(),
            reference: "/tmp/session.jsonl".to_owned(),
            max_turn: Some(3),
            probe: true,
            probe_model: Some("gpt-5".to_owned()),
        }
    }

    #[test]
    fn migration_preview_forces_read_only_request_shape() {
        let request = migration_request(&migration_input(), true).unwrap();
        let value: serde_json::Value = serde_json::from_str(&request).unwrap();
        let params = value.get("params").unwrap();
        assert_eq!(
            params.get("dry_run").and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            params.get("probe").and_then(serde_json::Value::as_bool),
            Some(false)
        );
        assert!(params.get("cwd").is_none());
        assert!(params.get("content_locale").is_none());
    }

    #[test]
    fn migration_input_rejects_invalid_reference() {
        let mut input = migration_input();
        input.reference = "bad\0reference".to_owned();
        assert!(validate_migration_input(&input).is_err());
    }

    fn edit_operation_input() -> OperationPlanInput {
        OperationPlanInput {
            kind: "edit".to_owned(),
            tool: "claude".to_owned(),
            reference: "fsr_fixture".to_owned(),
            ops: vec![
                serde_json::json!({"op": "delete-turn", "turn": 1}),
                serde_json::json!({
                    "op": "rewrite",
                    "locator": "fml_fixture",
                    "text": "updated",
                }),
            ],
            probe: true,
        }
    }

    #[test]
    fn operation_plan_request_has_a_fixed_method_and_tagged_input() {
        let request = operation_plan_request(&edit_operation_input()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&request).unwrap();
        assert_eq!(
            value.get("method").and_then(serde_json::Value::as_str),
            Some("operation.plan")
        );
        assert_eq!(
            value
                .pointer("/params/input/kind")
                .and_then(serde_json::Value::as_str),
            Some("edit")
        );
        assert!(value.pointer("/params/input/tool").is_some());
        assert!(value.pointer("/params/input/ref").is_some());
        assert!(value.pointer("/params/input/ops").is_some());
        assert_eq!(
            value
                .pointer("/params/input/probe")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            value
                .get("params")
                .and_then(serde_json::Value::as_object)
                .map(serde_json::Map::len),
            Some(1)
        );
    }

    #[test]
    fn operation_id_requests_cannot_override_the_engine_method() {
        for method in ["operation.apply", "operation.status", "operation.cancel"] {
            let request = operation_id_request(method, "op_fixture-123").unwrap();
            let value: serde_json::Value = serde_json::from_str(&request).unwrap();
            assert_eq!(
                value.get("method").and_then(serde_json::Value::as_str),
                Some(method)
            );
            assert_eq!(
                value
                    .pointer("/params/plan_id")
                    .and_then(serde_json::Value::as_str),
                Some("op_fixture-123")
            );
            assert_eq!(
                value
                    .get("params")
                    .and_then(serde_json::Value::as_object)
                    .map(serde_json::Map::len),
                Some(1)
            );
        }
        assert!(operation_id_request("show", "op_fixture-123").is_err());
    }

    #[test]
    fn operation_inputs_are_strictly_validated() {
        assert!(validate_operation_plan_input(&edit_operation_input()).is_ok());
        let mut wrong_kind = edit_operation_input();
        wrong_kind.kind = "migration".to_owned();
        assert!(validate_operation_plan_input(&wrong_kind).is_err());
        let mut unknown_tool = edit_operation_input();
        unknown_tool.tool = "unknown".to_owned();
        assert!(validate_operation_plan_input(&unknown_tool).is_err());
        let mut extra_field = edit_operation_input();
        extra_field.ops = vec![serde_json::json!({
            "op": "delete-turn", "turn": 1, "method": "operation.apply",
        })];
        assert!(validate_operation_plan_input(&extra_field).is_err());
        let mut duplicate = edit_operation_input();
        duplicate.ops = vec![
            serde_json::json!({"op": "rewrite", "locator": "fml_a", "text": "a"}),
            serde_json::json!({"op": "rewrite", "locator": "fml_a", "text": "b"}),
        ];
        assert!(validate_operation_plan_input(&duplicate).is_err());
    }

    #[test]
    fn operation_plan_id_validation_rejects_injection_and_bad_shapes() {
        assert!(validate_plan_id("op_fixture-123").is_ok());
        assert!(validate_plan_id("operation_fixture").is_err());
        assert!(validate_plan_id("op_bad\nmethod").is_err());
        assert!(validate_plan_id(&format!("op_{}", "a".repeat(126))).is_err());
    }
}
