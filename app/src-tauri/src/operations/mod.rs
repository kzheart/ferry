//! 统一 Operation 的 WebView 命令边界。
//!
//! 这里负责把已类型化的计划输入校验并编码为固定 Engine RPC；sidecar 只负责
//! 进程监督与请求传输，不能混入具体 operation 的字段规则。

pub(crate) mod request;
mod validation;

use self::request::{operation_plan_id_request, operation_plan_request};
use self::validation::{agent_has_capability, is_known_agent, validate_opaque_ref, validate_reply};
use crate::contracts::operations::{
    EditOperationPlanInput, MetadataOperationPlanInput, MigrationOperationPlanInput,
    OperationPlanInput,
};
use crate::contracts::operations::{EDIT_OPERATION_KINDS, OPERATION_KINDS};
use crate::engine::engine_request_blocking;
use serde_json::Value;
use std::collections::HashSet;

fn validate_edit_operation_input(input: &EditOperationPlanInput) -> Result<(), String> {
    if !is_known_agent(&input.tool) {
        return Err("Operation 工具标识无效".to_owned());
    }
    if !agent_has_capability(&input.tool, "edit") {
        return Err("Operation 工具不支持编辑".to_owned());
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
        let kind = fields
            .get("op")
            .and_then(Value::as_str)
            .filter(|value| EDIT_OPERATION_KINDS.contains(value))
            .ok_or_else(|| "Operation edit op 不受支持".to_owned())?;
        match kind {
            "delete-turn" => {
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
            "rewrite" => {
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
            "replace-assistant-reply" => {
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
            // 契约已穷尽过滤,走到这里说明共享契约新增了 kind 而这里还没补校验分支
            kind => return Err(format!("Operation edit op {kind} 缺少校验实现")),
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
    if !agent_has_capability(&input.source_tool, "migration-source")
        || !agent_has_capability(&input.target_tool, "migration-target")
    {
        return Err("Migration Operation Agent 不支持对应迁移方向".to_owned());
    }
    validate_opaque_ref(&input.reference, "Migration Operation")?;
    if input.max_turn == Some(0) || input.max_turn.is_some_and(|turn| turn > 100_000) {
        return Err("Migration Operation max_turn 无效".to_owned());
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

pub(crate) fn validate_operation_plan_input(input: &OperationPlanInput) -> Result<(), String> {
    if !OPERATION_KINDS.contains(&input.kind()) {
        return Err("Operation kind 未在共享契约中声明".to_owned());
    }
    match input {
        OperationPlanInput::Edit(edit) => validate_edit_operation_input(edit),
        OperationPlanInput::Migration(migration) => validate_migration_operation_input(migration),
        OperationPlanInput::Metadata(metadata) => validate_metadata_operation_input(metadata),
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

/// 这道关卡与 `crates/ferry-engine/src/operations/validation.rs` 是同一判定的
/// 两侧实现：host 先挡一道，引擎再挡一道，非法输入的字面量两边保持一致。
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const REF: &str = "fsr_0123456789abcdef";

    fn validate(input: Value) -> Result<(), String> {
        validate_operation_plan_input(
            &serde_json::from_value::<OperationPlanInput>(input).expect("input decodes"),
        )
    }

    fn edit(overrides: Value) -> Value {
        merge(
            json!({
                "kind": "edit",
                "tool": "claude",
                "ref": REF,
                "ops": [{"op": "delete-turn", "turn": 1}],
            }),
            overrides,
        )
    }

    fn migration(overrides: Value) -> Value {
        merge(
            json!({
                "kind": "migration",
                "source_tool": "claude",
                "ref": REF,
                "target_tool": "opencode",
            }),
            overrides,
        )
    }

    fn metadata(patch: Value) -> Value {
        json!({"kind": "metadata", "tool": "claude", "ref": REF, "patch": patch})
    }

    fn merge(mut base: Value, overrides: Value) -> Value {
        let fields = base.as_object_mut().expect("base is an object");
        for (key, value) in overrides.as_object().expect("overrides is an object") {
            fields.insert(key.clone(), value.clone());
        }
        base
    }

    #[test]
    fn edit_accepts_every_declared_edit_op() {
        assert!(validate(edit(json!({}))).is_ok());
        assert!(validate(edit(json!({
            "ops": [
                {"op": "rewrite", "locator": "fml_one", "text": "改写"},
                {
                    "op": "replace-assistant-reply",
                    "turn": 2,
                    "reply": {"items": [{"kind": "text", "text": "答"}]},
                },
            ],
        })))
        .is_ok());
    }

    #[test]
    fn edit_rejects_unknown_agent_and_missing_capability() {
        assert!(validate(edit(json!({"tool": "unknown"}))).is_err());
        // grok 在共享契约里没有 edit 能力。
        assert!(validate(edit(json!({"tool": "grok"}))).is_err());
    }

    #[test]
    fn edit_rejects_bad_reference_and_op_count() {
        assert!(validate(edit(json!({"ref": ""}))).is_err());
        assert!(validate(edit(json!({"ref": "a".repeat(513)}))).is_err());
        assert!(validate(edit(json!({"ref": "fsr_bad\u{7}ref"}))).is_err());
        assert!(validate(edit(json!({"ops": []}))).is_err());
        let too_many: Vec<Value> = (1..=51)
            .map(|turn| json!({"op": "delete-turn", "turn": turn}))
            .collect();
        assert!(validate(edit(json!({"ops": too_many}))).is_err());
    }

    #[test]
    fn edit_rejects_malformed_or_duplicated_ops() {
        assert!(validate(edit(json!({"ops": ["delete-turn"]}))).is_err());
        assert!(validate(edit(json!({"ops": [{"op": "purge", "turn": 1}]}))).is_err());
        assert!(validate(edit(json!({"ops": [{"op": "delete-turn", "turn": 0}]}))).is_err());
        assert!(validate(edit(json!({
            "ops": [{"op": "delete-turn", "turn": 1}, {"op": "delete-turn", "turn": 1}],
        })))
        .is_err());
        assert!(validate(edit(json!({
            "ops": [{"op": "rewrite", "locator": "no-prefix", "text": "改写"}],
        })))
        .is_err());
        let duplicated_reply = json!({
            "op": "replace-assistant-reply",
            "turn": 1,
            "reply": {"items": [{"kind": "text", "text": "答"}]},
        });
        assert!(validate(edit(json!({
            "ops": [duplicated_reply.clone(), duplicated_reply],
        })))
        .is_err());
        assert!(validate(edit(json!({
            "ops": [{
                "op": "replace-assistant-reply",
                "turn": 1,
                "reply": {"items": []},
            }],
        })))
        .is_err());
        // op 里夹带 method 字段是注入企图,必须拒。
        assert!(validate(edit(json!({
            "ops": [{"op": "delete-turn", "turn": 1, "method": "operation.apply"}],
        })))
        .is_err());
    }

    #[test]
    fn edit_rejects_ops_payload_over_64_kib() {
        let ops: Vec<Value> = (0..5)
            .map(|index| {
                json!({
                    "op": "rewrite",
                    "locator": format!("fml_{index}"),
                    "text": "x".repeat(20_000),
                })
            })
            .collect();

        assert!(validate(edit(json!({"ops": ops}))).is_err());

        // 单条回复正文 20_001 字同样超限。
        assert!(validate(edit(json!({
            "ops": [{
                "op": "replace-assistant-reply",
                "turn": 1,
                "reply": {"items": [{"kind": "text", "text": "x".repeat(20_001)}]},
            }],
        })))
        .is_err());
    }

    #[test]
    fn migration_accepts_declared_options() {
        assert!(validate(migration(json!({}))).is_ok());
        assert!(validate(migration(json!({"max_turn": 7}))).is_ok());
    }

    #[test]
    fn migration_rejects_unknown_identical_or_incapable_agents() {
        assert!(validate(migration(json!({"source_tool": "unknown"}))).is_err());
        assert!(validate(migration(json!({"target_tool": "claude"}))).is_err());
    }

    #[test]
    fn migration_rejects_bad_reference_and_out_of_range_max_turn() {
        assert!(validate(migration(json!({"ref": "not-opaque"}))).is_err());
        assert!(validate(migration(json!({"max_turn": 0}))).is_err());
        assert!(validate(migration(json!({"max_turn": 100_001}))).is_err());
    }

    #[test]
    fn metadata_accepts_any_non_empty_subset_of_the_patch() {
        assert!(validate(metadata(json!({"name": "新名称"}))).is_ok());
        assert!(validate(metadata(json!({
            "name": "n", "pinned": true, "archived": false, "tags": ["a"],
        })))
        .is_ok());
    }

    #[test]
    fn metadata_rejects_empty_patch_and_out_of_range_fields() {
        assert!(validate(metadata(json!({}))).is_err());
        assert!(validate(metadata(json!({"name": "名".repeat(201)}))).is_err());
        assert!(validate(metadata(json!({"tags": vec!["a"; 21]}))).is_err());
        assert!(validate(metadata(json!({"tags": [""]}))).is_err());
        assert!(validate(metadata(json!({"tags": ["标".repeat(65)]}))).is_err());
    }

    #[test]
    fn metadata_rejects_unknown_agent_and_non_opaque_reference() {
        assert!(validate(json!({
            "kind": "metadata", "tool": "unknown", "ref": REF, "patch": {"pinned": true},
        }))
        .is_err());
        assert!(validate(json!({
            "kind": "metadata", "tool": "claude", "ref": "", "patch": {"pinned": true},
        }))
        .is_err());
    }
}
