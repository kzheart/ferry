use serde_json::{json, Map, Value};
use std::path::Path;
use std::time::Duration;

use crate::contracts::errors::error_policy;
use crate::contracts::ipc::FERRY_IPC_PROTOCOL;
use crate::contracts::operations::{
    OPERATION_PLAN_ID_PREFIX, OPERATION_STATUSES, OPERATION_SUCCESS_STATUS,
    OPERATION_TERMINAL_STATUSES,
};
use crate::engine::engine_request_blocking;
use crate::process::framing::JsonlWriter;

use super::approval::{allows_auto_apply, auto_policy};
use super::emit_host_event;
use super::next_id;
use super::tool_routes::{is_mutating_tool, resolve_tool_request};

const OPERATION_POLL_INTERVAL: Duration = Duration::from_millis(125);
const OPERATION_WAIT_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);

pub(super) fn complete_tool_request(
    app: &tauri::AppHandle,
    resource_dir: &Path,
    stdin: &JsonlWriter,
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
    let mutation = is_mutating_tool(name, &args);
    let mut outcome = route_tool(resource_dir, name, args, run_id);
    if mutation {
        if let Ok(operation) = outcome.clone() {
            let auto = allows_auto_apply(auto_policy(session_id), role_apply_policy);
            if auto {
                match apply_routed_operation(resource_dir, &operation) {
                    Ok(result) => {
                        emit_host_event(
                            app,
                            json!({
                                "protocol": FERRY_IPC_PROTOCOL,
                                "session_id": session_id,
                                "run_id": run_id,
                                "type": "operation.applied",
                                "payload": {
                                    "tool": name,
                                    "operation": operation.clone(),
                                    "result": result,
                                    "auto": true
                                },
                            }),
                        );
                        outcome = Ok(json!({
                            "operation": operation,
                            "status": "applied",
                            "result": result
                        }));
                    }
                    Err(code) => {
                        emit_host_event(
                            app,
                            json!({
                                "protocol": FERRY_IPC_PROTOCOL,
                                "session_id": session_id,
                                "run_id": run_id,
                                "type": "operation.failed",
                                "payload": {
                                    "tool": name,
                                    "operation": operation.clone(),
                                    "error": code,
                                    "auto": true
                                },
                            }),
                        );
                        outcome = Err(code);
                    }
                }
            } else {
                emit_host_event(
                    app,
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
    stdin: &JsonlWriter,
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
    let _ = stdin.write_line(&command.to_string());
}

pub(super) fn complete_engine_request(resource_dir: &Path, stdin: &JsonlWriter, event: &Value) {
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

pub(super) fn is_runtime_engine_method(method: &str) -> bool {
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
    let route = resolve_tool_request(name, &args).ok_or_else(|| "tool.not_allowed".to_owned())?;
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

pub(super) fn structured_engine_error(envelope: &Value) -> String {
    let error = envelope.get("error").and_then(Value::as_object);
    let code = error
        .and_then(|value| value.get("code"))
        .and_then(Value::as_str)
        .filter(|value| error_policy(value).is_some())
        .unwrap_or("engine.request_failed");
    let policy = error_policy(code).expect("fallback error policy exists");
    let params = error
        .and_then(|value| value.get("params"))
        .cloned()
        .filter(|value| value.to_string().len() <= 4096)
        .unwrap_or_else(|| json!({}));
    json!({
        "code": code,
        "category": policy.category,
        "retryable": policy.retryable,
        "params": params,
    })
    .to_string()
}

fn engine_value(resource_dir: &Path, method: &str, params: Value) -> Result<Value, String> {
    let request = json!({"method": method, "params": params});
    let response = engine_request_blocking(resource_dir, &request.to_string())?;
    let envelope: Value = serde_json::from_str(&response).map_err(|error| error.to_string())?;
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

fn wait_for_operation(
    resource_dir: &Path,
    plan_id: &str,
    mut status: Value,
) -> Result<Value, String> {
    let deadline = std::time::Instant::now() + OPERATION_WAIT_TIMEOUT;
    loop {
        match status.get("status").and_then(Value::as_str) {
            Some(value) if value == OPERATION_SUCCESS_STATUS => {
                return Ok(status.get("result").cloned().unwrap_or(Value::Null));
            }
            Some(value) if OPERATION_TERMINAL_STATUSES.contains(&value) => {
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
            Some(value) if OPERATION_STATUSES.contains(&value) => {
                return Err("operation.invalid_status".to_owned());
            }
            Some(_) => return Err("operation.unknown_status".to_owned()),
            None => return Err("operation.invalid_status".to_owned()),
        }
    }
}

pub(super) fn operation_plan_id(operation: &Value) -> Result<&str, String> {
    operation
        .get("plan_id")
        .and_then(Value::as_str)
        .filter(|plan_id| plan_id.starts_with(OPERATION_PLAN_ID_PREFIX))
        .ok_or_else(|| "Engine 未返回可审批的 operation plan_id".to_owned())
}

fn apply_routed_operation(resource_dir: &Path, operation: &Value) -> Result<Value, String> {
    let plan_id = operation_plan_id(operation)?;
    apply_operation_plan(resource_dir, plan_id)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn runtime_engine_gateway_is_an_exact_allowlist() {
        for method in [
            "session_backbone",
            "session_summaries_set",
            "organization_digest_context",
            "organization_proposals_list",
            "organization_propose",
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
}
