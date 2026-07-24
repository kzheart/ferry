use crate::contracts::engine_methods::{self, Exposure};
use crate::contracts::ipc::{FERRY_CONTRACT_HASH, FERRY_IPC_PROTOCOL};
use crate::process::client::{JsonlProcessClient, PendingResponses};
use crate::process::error::ProcessError;
use crate::sidecar_policy::{request_attempts, request_timeout};
use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex, OnceLock,
};
use std::time::Duration;

#[derive(Clone)]
struct EngineClient {
    generation: u64,
    transport: JsonlProcessClient,
}

struct EngineProcess {
    generation: u64,
    child: Child,
    client: EngineClient,
}

impl Drop for EngineProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

static ENGINE: OnceLock<Mutex<Option<EngineProcess>>> = OnceLock::new();
static ENGINE_REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static ENGINE_GENERATION_SEQUENCE: AtomicU64 = AtomicU64::new(1);

fn stamp_engine_request(request: &str) -> Result<(String, String), String> {
    let value: Value = serde_json::from_str(request)
        .map_err(|error| format!("Engine 请求不是有效 JSON: {error}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "Engine 请求必须是 JSON object".to_owned())?;
    let method = object
        .get("method")
        .and_then(Value::as_str)
        .ok_or_else(|| "Engine 请求缺少 method".to_owned())?;
    let params = object
        .get("params")
        .cloned()
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
    if !params.is_object() {
        return Err("Engine 请求 params 必须是 JSON object".to_owned());
    }
    let request_id = format!(
        "engine_{:x}",
        ENGINE_REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed),
    );
    let envelope = serde_json::json!({
        "protocol": FERRY_IPC_PROTOCOL,
        "id": request_id,
        "method": method,
        "params": params,
    });
    Ok((envelope.to_string(), request_id))
}

fn validate_engine_response_id(response: &str, request_id: &str) -> Result<(), String> {
    let value: Value = serde_json::from_str(response)
        .map_err(|error| format!("Engine 响应不是有效 JSON: {error}"))?;
    if value.get("protocol").and_then(Value::as_str) != Some(FERRY_IPC_PROTOCOL) {
        return Err("Engine 响应 protocol 不匹配".to_owned());
    }
    if value.get("id").and_then(Value::as_str) != Some(request_id) {
        return Err("Engine 响应 id 不匹配".to_owned());
    }
    Ok(())
}

fn spawn_engine(resource_dir: &Path) -> Result<EngineProcess, String> {
    let mut command = engine_command(resource_dir)?;
    command.arg("serve");
    crate::platform::configure_background_command(&mut command);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|error| format!("启动引擎失败: {error}"))?;
    let stdin = child.stdin.take().ok_or("引擎 stdin 不可用")?;
    let stdout = child.stdout.take().ok_or("引擎 stdout 不可用")?;
    let transport = JsonlProcessClient::new("Engine", stdin);
    let reader_pending = transport.pending();
    std::thread::spawn(move || {
        read_engine_output(BufReader::new(stdout), reader_pending);
    });
    let generation = ENGINE_GENERATION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let client = EngineClient {
        generation,
        transport,
    };
    let engine = EngineProcess {
        generation,
        child,
        client,
    };
    handshake(&engine.client)?;
    Ok(engine)
}

/// 协议握手作为常驻进程的首条请求完成:独立的一次性 health 子进程
/// 在 release 下会让 PyInstaller onefile 多解压一整次,冷启动时间翻倍。
fn handshake(engine: &EngineClient) -> Result<(), String> {
    let (request, request_id) = stamp_engine_request(r#"{"method":"health"}"#)?;
    let line = engine
        .transport
        .request(&request_id, &request, Duration::from_secs(15))
        .map_err(|error| error.to_string())
        .map_err(|error| format!("引擎健康检查失败: {error}"))?;
    validate_engine_response_id(&line, &request_id)
        .map_err(|error| format!("引擎健康检查失败: {error}"))?;
    let health: Value = serde_json::from_str(&line)
        .map_err(|error| format!("引擎健康检查返回无效 JSON: {error}"))?;
    if health.get("ok").and_then(Value::as_bool) != Some(true)
        || health.pointer("/result/service").and_then(Value::as_str) != Some("engine")
        || health
            .pointer("/result/contract_hash")
            .and_then(Value::as_str)
            != Some(FERRY_CONTRACT_HASH)
    {
        return Err("引擎协议或契约握手失败".to_owned());
    }
    Ok(())
}

impl EngineClient {
    fn request(&self, request: &str, timeout: Duration) -> Result<String, ProcessError> {
        let value: Value = serde_json::from_str(request).map_err(|error| {
            ProcessError::InvalidFrame(format!("Engine 请求不是有效 JSON: {error}"))
        })?;
        let request_id = value
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| ProcessError::InvalidFrame("Engine 请求缺少 id".to_owned()))?;
        self.transport.request(request_id, request, timeout)
    }
}

fn read_engine_output(mut stdout: impl BufRead, pending: PendingResponses) {
    let mut line = String::new();
    loop {
        line.clear();
        match stdout.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(error) => {
                pending.fail_all(ProcessError::Exited(format!("读取引擎失败: {error}")));
                return;
            }
        }
        let response = line.trim_end();
        let request_id = serde_json::from_str::<Value>(response)
            .ok()
            .and_then(|value| value.get("id").and_then(Value::as_str).map(str::to_owned));
        let Some(request_id) = request_id else {
            pending.fail_all(ProcessError::Exited("Engine 响应缺少 id".to_owned()));
            return;
        };
        pending.complete(&request_id, response.to_owned());
    }
    pending.fail_all(ProcessError::Exited("引擎进程已退出".to_owned()));
}

fn engine_client(resource_dir: &Path) -> Result<EngineClient, String> {
    let slot = ENGINE.get_or_init(|| Mutex::new(None));
    let mut guard = slot.lock().map_err(|_| "引擎状态锁损坏".to_owned())?;
    if guard.is_none() {
        *guard = Some(spawn_engine(resource_dir)?);
    }
    Ok(guard.as_ref().expect("engine just ensured").client.clone())
}

fn invalidate_engine(generation: u64) {
    let slot = ENGINE.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = slot.lock() {
        if guard
            .as_ref()
            .is_some_and(|engine| engine.generation == generation)
        {
            *guard = None;
        }
    }
}

pub(crate) fn engine_request_blocking(
    resource_dir: &Path,
    request: &str,
) -> Result<String, String> {
    let (request, request_id) = stamp_engine_request(request)?;
    let timeout = request_timeout(&request);
    let mut last_error = String::new();
    for _attempt in 0..request_attempts(&request) {
        let client = engine_client(resource_dir)?;
        match client.request(&request, timeout) {
            Ok(line) => match validate_engine_response_id(&line, &request_id) {
                Ok(()) => return Ok(line),
                Err(error) => {
                    last_error = error;
                    invalidate_engine(client.generation);
                }
            },
            Err(error) => {
                last_error = error.to_string();
                if error.invalidates_process() {
                    invalidate_engine(client.generation);
                }
            }
        }
    }
    Err(format!("引擎通信失败: {last_error}"))
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
        command.args(["-m", "engine.server.cli"]);
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
    validate_engine_request_exposure(&request, Exposure::Public)?;
    let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || engine_request_blocking(&resource_dir, &request))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(crate) async fn trusted_engine_rpc(
    app: tauri::AppHandle,
    request: String,
) -> Result<String, String> {
    use tauri::Manager;
    validate_engine_request_exposure(&request, Exposure::TrustedUi)?;
    let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || engine_request_blocking(&resource_dir, &request))
        .await
        .map_err(|e| e.to_string())?
}

fn validate_engine_request_exposure(request: &str, expected: Exposure) -> Result<(), String> {
    let value: Value = serde_json::from_str(request)
        .map_err(|error| format!("Engine 请求不是有效 JSON: {error}"))?;
    let method = value.get("method").and_then(Value::as_str).unwrap_or("");
    if !engine_methods::policy(method).is_some_and(|policy| policy.exposure == expected) {
        return Err("该 Engine 方法不允许从当前前端通道调用".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        read_engine_output, request_attempts, request_timeout, stamp_engine_request,
        validate_engine_request_exposure, validate_engine_response_id, FERRY_IPC_PROTOCOL,
    };
    use crate::contracts::engine_methods::Exposure;
    use crate::operation_commands::validate_operation_plan_input;
    use crate::operation_input::{
        DeleteOperationPlanInput, EditOperationPlanInput, MetadataOperationPlanInput,
        MetadataPatch, MigrationOperationPlanInput, OperationPlanInput,
        RestoreDeleteOperationPlanInput,
    };
    use crate::operation_request::{
        operation_plan_id_request, operation_plan_request, validate_plan_id,
    };
    use crate::process::client::PendingResponses;
    use crate::sidecar_policy::{AGENT_LOOKUP_TIMEOUT, ENGINE_TIMEOUT};
    use std::io::Cursor;

    #[test]
    fn engine_output_is_dispatched_by_id_even_when_responses_are_reordered() {
        let pending = PendingResponses::default();
        let first_receiver = pending.register("engine_first").unwrap();
        let second_receiver = pending.register("engine_second").unwrap();

        read_engine_output(
            Cursor::new(
                b"{\"id\":\"engine_second\",\"ok\":true}\n{\"id\":\"engine_first\",\"ok\":true}\n",
            ),
            pending.clone(),
        );

        assert!(first_receiver
            .recv()
            .unwrap()
            .unwrap()
            .contains("engine_first"));
        assert!(second_receiver
            .recv()
            .unwrap()
            .unwrap()
            .contains("engine_second"));
    }

    #[test]
    fn malformed_engine_output_releases_all_waiting_requests() {
        let pending = PendingResponses::default();
        let receiver = pending.register("engine_waiting").unwrap();

        read_engine_output(Cursor::new(b"not-json\n"), pending);

        assert_eq!(
            receiver.recv().unwrap().unwrap_err().to_string(),
            "Engine 响应缺少 id",
        );
    }

    #[test]
    fn sensitive_agent_methods_are_not_generic_rpc_methods() {
        assert!(validate_engine_request_exposure(
            r#"{"method":"operation.apply"}"#,
            Exposure::Public,
        )
        .is_err());
        assert!(
            validate_engine_request_exposure(r#"{"method":"scan"}"#, Exposure::Public,).is_ok()
        );
        assert!(validate_engine_request_exposure(
            r#"{"method":"organization_proposals_list"}"#,
            Exposure::Public,
        )
        .is_err());
        assert!(validate_engine_request_exposure(
            r#"{"method":"organization_proposals_list"}"#,
            Exposure::TrustedUi,
        )
        .is_ok());
        assert!(validate_engine_request_exposure(
            r#"{"method":"session_backbone"}"#,
            Exposure::TrustedUi,
        )
        .is_err());
        assert!(validate_engine_request_exposure(
            r#"{"method":"agent_session_read"}"#,
            Exposure::Public,
        )
        .is_err());
        // 删除迁移记录只动 Ferry 自己的历史文件,不写目标工具的会话
        assert!(validate_engine_request_exposure(
            r#"{"method":"history_delete"}"#,
            Exposure::Public,
        )
        .is_ok());
    }

    #[test]
    fn operation_enqueue_uses_normal_rpc_timeout() {
        assert_eq!(
            request_timeout(r#"{"method":"operation.apply"}"#),
            ENGINE_TIMEOUT
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

    fn edit_operation_input() -> EditOperationPlanInput {
        EditOperationPlanInput {
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
        let request = operation_plan_request(
            &OperationPlanInput::Edit(edit_operation_input()),
            validate_operation_plan_input,
        )
        .unwrap();
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
    fn operation_plan_id_requests_cannot_override_the_engine_method() {
        for method in ["operation.apply", "operation.status", "operation.cancel"] {
            let request = operation_plan_id_request(method, "op_fixture-123").unwrap();
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
        assert!(operation_plan_id_request("show", "op_fixture-123").is_err());
    }

    #[test]
    fn operation_inputs_are_strictly_validated() {
        assert!(
            validate_operation_plan_input(&OperationPlanInput::Edit(edit_operation_input()))
                .is_ok()
        );
        let mut unknown_tool = edit_operation_input();
        unknown_tool.tool = "unknown".to_owned();
        assert!(validate_operation_plan_input(&OperationPlanInput::Edit(unknown_tool)).is_err());
        let mut extra_field = edit_operation_input();
        extra_field.ops = vec![serde_json::json!({
            "op": "delete-turn", "turn": 1, "method": "operation.apply",
        })];
        assert!(validate_operation_plan_input(&OperationPlanInput::Edit(extra_field)).is_err());
        let mut duplicate = edit_operation_input();
        duplicate.ops = vec![
            serde_json::json!({"op": "rewrite", "locator": "fml_a", "text": "a"}),
            serde_json::json!({"op": "rewrite", "locator": "fml_a", "text": "b"}),
        ];
        assert!(validate_operation_plan_input(&OperationPlanInput::Edit(duplicate)).is_err());
    }

    fn migration_operation_input() -> MigrationOperationPlanInput {
        MigrationOperationPlanInput {
            source_tool: "claude".to_owned(),
            reference: "fsr_fixture".to_owned(),
            target_tool: "codex".to_owned(),
            max_turn: Some(3),
            probe: true,
            probe_model: Some("gpt-5".to_owned()),
        }
    }

    #[test]
    fn operation_accepts_strict_tagged_metadata_input() {
        let input = OperationPlanInput::Metadata(MetadataOperationPlanInput {
            tool: "claude".to_owned(),
            reference: "fsr_fixture".to_owned(),
            patch: MetadataPatch {
                name: Some("新名称".to_owned()),
                pinned: Some(true),
                archived: None,
                tags: Some(vec!["ferry".to_owned()]),
            },
        });
        assert!(validate_operation_plan_input(&input).is_ok());

        let request = operation_plan_request(&input, validate_operation_plan_input).unwrap();
        let value: serde_json::Value = serde_json::from_str(&request).unwrap();
        assert_eq!(
            value
                .pointer("/params/input/kind")
                .and_then(serde_json::Value::as_str),
            Some("metadata")
        );
        assert_eq!(
            value
                .pointer("/params/input/patch/pinned")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn operation_accepts_strict_tagged_delete_input() {
        let input = OperationPlanInput::Delete(DeleteOperationPlanInput {
            tool: "claude".to_owned(),
            reference: "fsr_fixture".to_owned(),
        });
        assert!(validate_operation_plan_input(&input).is_ok());

        let request = operation_plan_request(&input, validate_operation_plan_input).unwrap();
        let value: serde_json::Value = serde_json::from_str(&request).unwrap();
        assert_eq!(
            value
                .pointer("/params/input/kind")
                .and_then(serde_json::Value::as_str),
            Some("delete")
        );
        assert!(value.pointer("/params/input/ref").is_some());
        assert!(value.pointer("/params/input/ops").is_none());

        let unknown = OperationPlanInput::Delete(DeleteOperationPlanInput {
            tool: "unknown".to_owned(),
            reference: "fsr_fixture".to_owned(),
        });
        assert!(validate_operation_plan_input(&unknown).is_err());
    }

    #[test]
    fn operation_accepts_strict_restore_delete_input() {
        let input = OperationPlanInput::RestoreDelete(RestoreDeleteOperationPlanInput {
            recovery_id: "recovery_fixture-123".to_owned(),
        });
        assert!(validate_operation_plan_input(&input).is_ok());

        let request = operation_plan_request(&input, validate_operation_plan_input).unwrap();
        let value: serde_json::Value = serde_json::from_str(&request).unwrap();
        assert_eq!(
            value
                .pointer("/params/input/kind")
                .and_then(serde_json::Value::as_str),
            Some("restore-delete")
        );
        assert!(value.pointer("/params/input/recovery_id").is_some());
        assert!(value.pointer("/params/input/ref").is_none());
    }

    #[test]
    fn operation_accepts_strict_tagged_migration_input() {
        let input = OperationPlanInput::Migration(migration_operation_input());
        assert!(validate_operation_plan_input(&input).is_ok());

        let request = operation_plan_request(&input, validate_operation_plan_input).unwrap();
        let value: serde_json::Value = serde_json::from_str(&request).unwrap();
        assert_eq!(
            value
                .pointer("/params/input/kind")
                .and_then(serde_json::Value::as_str),
            Some("migration")
        );
        assert_eq!(
            value
                .pointer("/params/input/source_tool")
                .and_then(serde_json::Value::as_str),
            Some("claude")
        );
        assert_eq!(
            value
                .pointer("/params/input/target_tool")
                .and_then(serde_json::Value::as_str),
            Some("codex")
        );
        assert_eq!(
            value
                .pointer("/params/input/ref")
                .and_then(serde_json::Value::as_str),
            Some("fsr_fixture")
        );
        assert!(value.pointer("/params/input/tool").is_none());
        assert!(value.pointer("/params/input/ops").is_none());
    }

    #[test]
    fn operation_migration_input_rejects_invalid_agents_and_options() {
        let mut same_agent = migration_operation_input();
        same_agent.target_tool = "claude".to_owned();
        assert!(validate_operation_plan_input(&OperationPlanInput::Migration(same_agent)).is_err());

        let mut unknown_agent = migration_operation_input();
        unknown_agent.source_tool = "unknown".to_owned();
        assert!(
            validate_operation_plan_input(&OperationPlanInput::Migration(unknown_agent)).is_err()
        );

        let mut native_ref = migration_operation_input();
        native_ref.reference = "/tmp/session.jsonl".to_owned();
        assert!(validate_operation_plan_input(&OperationPlanInput::Migration(native_ref)).is_err());

        let mut invalid_turn = migration_operation_input();
        invalid_turn.max_turn = Some(0);
        assert!(
            validate_operation_plan_input(&OperationPlanInput::Migration(invalid_turn)).is_err()
        );

        let mut unused_model = migration_operation_input();
        unused_model.probe = false;
        assert!(
            validate_operation_plan_input(&OperationPlanInput::Migration(unused_model)).is_err()
        );

        let mut invalid_model = migration_operation_input();
        invalid_model.probe_model = Some("bad\nmodel".to_owned());
        assert!(
            validate_operation_plan_input(&OperationPlanInput::Migration(invalid_model)).is_err()
        );
    }

    #[test]
    fn operation_tagged_inputs_deny_unknown_or_cross_variant_fields() {
        let unknown = serde_json::json!({
            "kind": "migration",
            "source_tool": "claude",
            "ref": "fsr_fixture",
            "target_tool": "codex",
            "probe": false,
            "method": "operation.apply",
        });
        assert!(serde_json::from_value::<OperationPlanInput>(unknown).is_err());

        let mixed = serde_json::json!({
            "kind": "edit",
            "tool": "claude",
            "ref": "fsr_fixture",
            "ops": [{"op": "delete-turn", "turn": 1}],
            "probe": false,
            "target_tool": "codex",
        });
        assert!(serde_json::from_value::<OperationPlanInput>(mixed).is_err());
    }

    #[test]
    fn operation_accepts_current_replace_assistant_reply_shape() {
        let mut input = edit_operation_input();
        input.ops = vec![serde_json::json!({
            "op": "replace-assistant-reply",
            "turn": "turn:fixture",
            "reply": {
                "items": [
                    {"kind": "text", "text": "updated answer"},
                    {
                        "kind": "tool",
                        "name": "read",
                        "input": {"path": "/tmp/file"},
                        "output": "contents",
                    },
                ],
            },
        })];

        let input = OperationPlanInput::Edit(input);
        assert!(validate_operation_plan_input(&input).is_ok());
        let request = operation_plan_request(&input, validate_operation_plan_input).unwrap();
        let value: serde_json::Value = serde_json::from_str(&request).unwrap();
        assert_eq!(
            value
                .pointer("/params/input/ops/0/op")
                .and_then(serde_json::Value::as_str),
            Some("replace-assistant-reply")
        );
    }

    #[test]
    fn operation_rejects_invalid_replace_assistant_reply_shapes() {
        for operation in [
            serde_json::json!({
                "op": "replace-assistant-reply",
                "turn": 0,
                "reply": {"items": [{"kind": "text", "text": "x"}]},
            }),
            serde_json::json!({
                "op": "replace-assistant-reply",
                "turn": 1,
                "reply": {"items": []},
            }),
            serde_json::json!({
                "op": "replace-assistant-reply",
                "turn": 1,
                "reply": {"items": [{"kind": "text", "text": "x", "extra": true}]},
            }),
            serde_json::json!({
                "op": "replace-assistant-reply",
                "turn": 1,
                "reply": {
                    "items": [{
                        "kind": "tool",
                        "name": "read",
                        "input": [],
                        "output": "x",
                    }],
                },
            }),
            serde_json::json!({
                "op": "replace-assistant-reply",
                "turn": 1,
                "reply": {"items": [{"kind": "text", "text": "x"}]},
                "method": "operation.apply",
            }),
        ] {
            let mut input = edit_operation_input();
            input.ops = vec![operation];
            assert!(validate_operation_plan_input(&OperationPlanInput::Edit(input)).is_err());
        }
    }

    #[test]
    fn operation_rejects_oversized_or_duplicate_reply_targets() {
        let mut oversized = edit_operation_input();
        oversized.ops = vec![serde_json::json!({
            "op": "replace-assistant-reply",
            "turn": 1,
            "reply": {"items": [{"kind": "text", "text": "x".repeat(20_001)}]},
        })];
        assert!(validate_operation_plan_input(&OperationPlanInput::Edit(oversized)).is_err());

        let authored = serde_json::json!({
            "op": "replace-assistant-reply",
            "turn": 1,
            "reply": {"items": [{"kind": "text", "text": "x"}]},
        });
        let mut duplicate = edit_operation_input();
        duplicate.ops = vec![authored.clone(), authored];
        assert!(validate_operation_plan_input(&OperationPlanInput::Edit(duplicate)).is_err());
    }

    #[test]
    fn operation_plan_id_validation_rejects_injection_and_bad_shapes() {
        assert!(validate_plan_id("op_fixture-123").is_ok());
        assert!(validate_plan_id("operation_fixture").is_err());
        assert!(validate_plan_id("op_bad\nmethod").is_err());
        assert!(validate_plan_id(&format!("op_{}", "a".repeat(126))).is_err());
    }

    #[test]
    fn engine_requests_receive_host_owned_correlation_ids() {
        let (request, request_id) =
            stamp_engine_request(r#"{"method":"health","request_id":"untrusted"}"#).unwrap();
        let value: serde_json::Value = serde_json::from_str(&request).unwrap();
        assert_eq!(
            value.get("id").and_then(serde_json::Value::as_str),
            Some(request_id.as_str()),
        );
        assert_eq!(
            value.get("protocol").and_then(serde_json::Value::as_str),
            Some(FERRY_IPC_PROTOCOL),
        );
        assert_ne!(request_id, "untrusted");
        assert!(validate_engine_response_id(
            &serde_json::json!({
                "protocol": FERRY_IPC_PROTOCOL,
                "id": request_id,
            })
            .to_string(),
            &request_id,
        )
        .is_ok());
        assert!(validate_engine_response_id(
            &serde_json::json!({
                "protocol": FERRY_IPC_PROTOCOL,
                "id": "other",
            })
            .to_string(),
            &request_id,
        )
        .is_err());
    }
}
