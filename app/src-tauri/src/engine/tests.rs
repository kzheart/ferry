use super::policy::{AGENT_LOOKUP_TIMEOUT, AGENT_RUN_TIMEOUT, ENGINE_TIMEOUT};
use super::{
    read_engine_output, request_attempts, request_timeout, stamp_engine_request,
    validate_engine_request_caller, validate_engine_response_id, FERRY_IPC_PROTOCOL,
};
use crate::contracts::operations::{
    DeleteOperationPlanInput, EditOperationPlanInput, MetadataOperationPlanInput, MetadataPatch,
    MigrationOperationPlanInput, OperationPlanInput,
};
use crate::operations::request::{
    operation_plan_id_request, operation_plan_request, validate_plan_id,
};
use crate::operations::validate_operation_plan_input;
use crate::process::client::PendingResponses;
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
    assert!(validate_engine_request_caller(r#"{"method":"operation.apply"}"#).is_err());
    assert!(validate_engine_request_caller(r#"{"method":"scan"}"#).is_ok());
    assert!(validate_engine_request_caller(r#"{"method":"agent_prompt"}"#).is_err());
    assert!(validate_engine_request_caller(r#"{"method":"session_read"}"#).is_err());
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
    let request = r#"{"method":"content_search"}"#;
    assert_eq!(request_timeout(request), AGENT_LOOKUP_TIMEOUT);
    assert_eq!(request_attempts(request), 1);
}

#[test]
fn agent_prompt_has_one_long_deadline_and_no_retry() {
    let request = r#"{"method":"agent_prompt"}"#;
    assert_eq!(request_timeout(request), AGENT_RUN_TIMEOUT);
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
    assert!(value.pointer("/params/input/probe").is_none());
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

fn migration_operation_input() -> MigrationOperationPlanInput {
    MigrationOperationPlanInput {
        source_tool: "claude".to_owned(),
        reference: "fsr_fixture".to_owned(),
        target_tool: "codex".to_owned(),
        max_turn: Some(3),
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
        refs: vec!["fsr_fixture".to_owned()],
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
    assert!(value.pointer("/params/input/refs/0").is_some());
    assert!(value.pointer("/params/input/ops").is_none());

    let unknown = OperationPlanInput::Delete(DeleteOperationPlanInput {
        tool: "unknown".to_owned(),
        refs: vec!["fsr_fixture".to_owned()],
    });
    assert!(validate_operation_plan_input(&unknown).is_err());
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
