use super::policy::{AGENT_LOOKUP_TIMEOUT, AGENT_RUN_TIMEOUT, ENGINE_TIMEOUT};
use super::{
    read_engine_output, request_attempts, request_timeout, socket_argument, stamp_engine_request,
    validate_engine_request_caller, validate_engine_response_id, FERRY_IPC_PROTOCOL,
};
use crate::contracts::engine_methods::is_runtime_gateway_method;
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
use std::path::PathBuf;

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

/// 共享开关与平台能力任一为否,sidecar 就不带 `--socket` 起——降级永远存在。
#[test]
fn sharing_needs_both_the_switch_and_a_platform_socket() {
    let path = || Ok(PathBuf::from("/home/u/.ferry/engine.sock"));
    assert_eq!(socket_argument(true, path()), Some(path().unwrap()));
    assert_eq!(socket_argument(false, path()), None);
    assert_eq!(
        socket_argument(true, Err("当前平台尚未实现引擎 socket".to_owned())),
        None
    );
    assert_eq!(socket_argument(false, Err("无所谓".to_owned())), None);
}

#[test]
fn sensitive_agent_methods_are_not_generic_rpc_methods() {
    assert!(validate_engine_request_caller(r#"{"method":"operation.apply"}"#).is_err());
    assert!(validate_engine_request_caller(r#"{"method":"scan"}"#).is_ok());
    assert!(validate_engine_request_caller(r#"{"method":"agent_prompt"}"#).is_err());
    assert!(validate_engine_request_caller(r#"{"method":"session_read"}"#).is_err());
    // 删除迁移记录只动 Ferry 自己的历史文件,不写目标工具的会话
    assert!(validate_engine_request_caller(r#"{"method":"history_delete"}"#).is_ok());
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
    assert_eq!(AGENT_RUN_TIMEOUT, std::time::Duration::from_secs(390));
    assert!(is_runtime_gateway_method("agent_prompt"));
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
        validate_operation_plan_input(&OperationPlanInput::Edit(edit_operation_input())).is_ok()
    );
    let mut unknown_tool = edit_operation_input();
    unknown_tool.tool = "unknown".to_owned();
    assert!(validate_operation_plan_input(&OperationPlanInput::Edit(unknown_tool)).is_err());
    let mut unsupported_tool = edit_operation_input();
    unsupported_tool.tool = "grok".to_owned();
    assert!(validate_operation_plan_input(&OperationPlanInput::Edit(unsupported_tool)).is_err());
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
fn operation_migration_input_rejects_invalid_agents_and_options() {
    let mut same_agent = migration_operation_input();
    same_agent.target_tool = "claude".to_owned();
    assert!(validate_operation_plan_input(&OperationPlanInput::Migration(same_agent)).is_err());

    let mut unknown_agent = migration_operation_input();
    unknown_agent.source_tool = "unknown".to_owned();
    assert!(validate_operation_plan_input(&OperationPlanInput::Migration(unknown_agent)).is_err());

    let mut native_ref = migration_operation_input();
    native_ref.reference = "/tmp/session.jsonl".to_owned();
    assert!(validate_operation_plan_input(&OperationPlanInput::Migration(native_ref)).is_err());

    let mut invalid_turn = migration_operation_input();
    invalid_turn.max_turn = Some(0);
    assert!(validate_operation_plan_input(&OperationPlanInput::Migration(invalid_turn)).is_err());

    let mut unused_model = migration_operation_input();
    unused_model.probe = false;
    assert!(validate_operation_plan_input(&OperationPlanInput::Migration(unused_model)).is_err());

    let mut invalid_model = migration_operation_input();
    invalid_model.probe_model = Some("bad\nmodel".to_owned());
    assert!(validate_operation_plan_input(&OperationPlanInput::Migration(invalid_model)).is_err());
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
