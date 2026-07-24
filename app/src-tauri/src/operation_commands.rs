//! 统一 Operation 的 WebView 命令边界。
//!
//! 这里负责把已类型化的计划输入校验并编码为固定 Engine RPC；sidecar 只负责
//! 进程监督与请求传输，不能混入具体 operation 的字段规则。

use crate::operation_input::{
    DeleteOperationPlanInput, EditOperationPlanInput, MetadataOperationPlanInput,
    MigrationOperationPlanInput, OperationPlanInput, RestoreDeleteOperationPlanInput,
};
use crate::operation_request::{operation_plan_id_request, operation_plan_request};
use crate::operation_validation::{is_known_agent, validate_opaque_ref, validate_reply};
use crate::sidecar::engine_request_blocking;
use serde_json::Value;
use std::collections::HashSet;

fn validate_edit_operation_input(input: &EditOperationPlanInput) -> Result<(), String> {
    if !is_known_agent(&input.tool) {
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
    let mut delete_turns = HashSet::new();
    let mut authored_turns = HashSet::new();
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
                let turn = fields["turn"].as_u64().expect("turn validated");
                if !delete_turns.insert(turn) {
                    return Err("Operation 不允许重复 delete-turn 目标".to_owned());
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
            Some("replace-assistant-reply") => {
                if fields.len() != 3
                    || !fields.contains_key("turn")
                    || !fields.contains_key("reply")
                {
                    return Err("Operation replace-assistant-reply 参数无效".to_owned());
                }
                let turn = match &fields["turn"] {
                    Value::Number(value) => {
                        let turn = value.as_u64().filter(|turn| *turn > 0).ok_or_else(|| {
                            "Operation replace-assistant-reply turn 无效".to_owned()
                        })?;
                        format!("number:{turn}")
                    }
                    Value::String(value)
                        if !value.is_empty()
                            && value.len() <= 512
                            && !value.chars().any(char::is_control) =>
                    {
                        format!("string:{value}")
                    }
                    _ => return Err("Operation replace-assistant-reply turn 无效".to_owned()),
                };
                if !authored_turns.insert(turn) {
                    return Err("Operation 不允许重复 replace-assistant-reply 目标".to_owned());
                }
                validate_reply(&fields["reply"])?;
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

fn validate_migration_operation_input(input: &MigrationOperationPlanInput) -> Result<(), String> {
    if !is_known_agent(&input.source_tool) || !is_known_agent(&input.target_tool) {
        return Err("Migration Operation Agent 标识无效".to_owned());
    }
    if input.source_tool == input.target_tool {
        return Err("Migration Operation 源和目标 Agent 不能相同".to_owned());
    }
    validate_opaque_ref(&input.reference, "Migration Operation")?;
    if input.max_turn == Some(0) || input.max_turn.is_some_and(|turn| turn > 100_000) {
        return Err("Migration Operation max_turn 无效".to_owned());
    }
    if input.probe_model.as_ref().is_some_and(|model| {
        model.is_empty() || model.chars().count() > 512 || model.chars().any(char::is_control)
    }) {
        return Err("Migration Operation probe_model 无效".to_owned());
    }
    if !input.probe && input.probe_model.is_some() {
        return Err("Migration Operation 未启用 probe 时不能指定模型".to_owned());
    }
    Ok(())
}

fn validate_metadata_operation_input(input: &MetadataOperationPlanInput) -> Result<(), String> {
    if !is_known_agent(&input.tool) {
        return Err("Metadata Operation Agent 标识无效".to_owned());
    }
    validate_opaque_ref(&input.reference, "Metadata Operation")?;
    let patch = &input.patch;
    if patch.name.is_none()
        && patch.pinned.is_none()
        && patch.archived.is_none()
        && patch.tags.is_none()
    {
        return Err("Metadata Operation patch 不能为空".to_owned());
    }
    if patch
        .name
        .as_ref()
        .is_some_and(|name| name.chars().count() > 200)
    {
        return Err("Metadata Operation name 过长".to_owned());
    }
    if patch.tags.as_ref().is_some_and(|tags| {
        tags.len() > 20
            || tags
                .iter()
                .any(|tag| tag.is_empty() || tag.chars().count() > 64)
    }) {
        return Err("Metadata Operation tags 无效".to_owned());
    }
    Ok(())
}

fn validate_delete_operation_input(input: &DeleteOperationPlanInput) -> Result<(), String> {
    if !is_known_agent(&input.tool) {
        return Err("Delete Operation Agent 标识无效".to_owned());
    }
    validate_opaque_ref(&input.reference, "Delete Operation")?;
    Ok(())
}

fn validate_restore_delete_operation_input(
    input: &RestoreDeleteOperationPlanInput,
) -> Result<(), String> {
    if !(16..=128).contains(&input.recovery_id.len())
        || !input.recovery_id.starts_with("recovery_")
        || !input
            .recovery_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err("Restore Delete Operation recovery_id 无效".to_owned());
    }
    Ok(())
}

pub(crate) fn validate_operation_plan_input(input: &OperationPlanInput) -> Result<(), String> {
    match input {
        OperationPlanInput::Edit(edit) => validate_edit_operation_input(edit),
        OperationPlanInput::Migration(migration) => validate_migration_operation_input(migration),
        OperationPlanInput::Metadata(metadata) => validate_metadata_operation_input(metadata),
        OperationPlanInput::Delete(delete) => validate_delete_operation_input(delete),
        OperationPlanInput::RestoreDelete(restore) => {
            validate_restore_delete_operation_input(restore)
        }
    }
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
    operation_engine_request(
        app,
        operation_plan_request(&input, validate_operation_plan_input)?,
    )
    .await
}

/// 此命令只接受已经生成的 plan_id；业务参数不会在应用阶段再次进入 Engine。
#[tauri::command]
pub(crate) async fn operation_apply(
    app: tauri::AppHandle,
    plan_id: String,
) -> Result<String, String> {
    operation_engine_request(app, operation_plan_id_request("operation.apply", &plan_id)?).await
}

#[tauri::command]
pub(crate) async fn operation_status(
    app: tauri::AppHandle,
    plan_id: String,
) -> Result<String, String> {
    operation_engine_request(
        app,
        operation_plan_id_request("operation.status", &plan_id)?,
    )
    .await
}

#[tauri::command]
pub(crate) async fn operation_cancel(
    app: tauri::AppHandle,
    plan_id: String,
) -> Result<String, String> {
    operation_engine_request(
        app,
        operation_plan_id_request("operation.cancel", &plan_id)?,
    )
    .await
}
