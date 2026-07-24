// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ErrorPolicy {
    pub(crate) category: &'static str,
    pub(crate) retryable: bool,
}

pub(crate) fn error_policy(code: &str) -> Option<ErrorPolicy> {
    match code {
        "agent.approval_invalid" => Some(ErrorPolicy {
            category: "permission",
            retryable: false,
        }),
        "agent.format_changed" => Some(ErrorPolicy {
            category: "unsupported",
            retryable: false,
        }),
        "agent.reference_invalid" => Some(ErrorPolicy {
            category: "validation",
            retryable: false,
        }),
        "agent.request_invalid" => Some(ErrorPolicy {
            category: "validation",
            retryable: false,
        }),
        "agent.run_missing" => Some(ErrorPolicy {
            category: "validation",
            retryable: false,
        }),
        "already_restored" => Some(ErrorPolicy {
            category: "conflict",
            retryable: false,
        }),
        "auth_in_progress" => Some(ErrorPolicy {
            category: "conflict",
            retryable: false,
        }),
        "auth_login_not_found" => Some(ErrorPolicy {
            category: "not-found",
            retryable: false,
        }),
        "auth_prompt_not_found" => Some(ErrorPolicy {
            category: "not-found",
            retryable: false,
        }),
        "auth_type_unsupported" => Some(ErrorPolicy {
            category: "unsupported",
            retryable: false,
        }),
        "edit.invalid_reply" => Some(ErrorPolicy {
            category: "validation",
            retryable: false,
        }),
        "edit.operation_unsupported" => Some(ErrorPolicy {
            category: "unsupported",
            retryable: false,
        }),
        "edit.subagent_not_supported" => Some(ErrorPolicy {
            category: "unsupported",
            retryable: false,
        }),
        "edit.turn_out_of_range" => Some(ErrorPolicy {
            category: "validation",
            retryable: false,
        }),
        "engine.invalid_response" => Some(ErrorPolicy {
            category: "internal",
            retryable: false,
        }),
        "engine.request_failed" => Some(ErrorPolicy {
            category: "internal",
            retryable: false,
        }),
        "engine.timeout" => Some(ErrorPolicy {
            category: "unavailable",
            retryable: true,
        }),
        "engine.unavailable" => Some(ErrorPolicy {
            category: "unavailable",
            retryable: true,
        }),
        "internal.unexpected" => Some(ErrorPolicy {
            category: "internal",
            retryable: false,
        }),
        "internal_error" => Some(ErrorPolicy {
            category: "internal",
            retryable: false,
        }),
        "invalid_json" => Some(ErrorPolicy {
            category: "validation",
            retryable: false,
        }),
        "invalid_params" => Some(ErrorPolicy {
            category: "validation",
            retryable: false,
        }),
        "invalid_provider_config" => Some(ErrorPolicy {
            category: "validation",
            retryable: false,
        }),
        "invalid_request" => Some(ErrorPolicy {
            category: "validation",
            retryable: false,
        }),
        "invalid_role" => Some(ErrorPolicy {
            category: "validation",
            retryable: false,
        }),
        "invalid_workflow" => Some(ErrorPolicy {
            category: "validation",
            retryable: false,
        }),
        "model_capability_mismatch" => Some(ErrorPolicy {
            category: "unsupported",
            retryable: false,
        }),
        "model_not_found" => Some(ErrorPolicy {
            category: "not-found",
            retryable: false,
        }),
        "no_active_run" => Some(ErrorPolicy {
            category: "conflict",
            retryable: false,
        }),
        "operation.invalid_status" => Some(ErrorPolicy {
            category: "internal",
            retryable: false,
        }),
        "operation.unknown_status" => Some(ErrorPolicy {
            category: "internal",
            retryable: false,
        }),
        "organization.proposal_invalid" => Some(ErrorPolicy {
            category: "validation",
            retryable: false,
        }),
        "organization.proposal_not_found" => Some(ErrorPolicy {
            category: "not-found",
            retryable: false,
        }),
        "organization.proposal_stale" => Some(ErrorPolicy {
            category: "conflict",
            retryable: true,
        }),
        "organization_failed" => Some(ErrorPolicy {
            category: "execution",
            retryable: false,
        }),
        "organizer_invalid_response" => Some(ErrorPolicy {
            category: "execution",
            retryable: false,
        }),
        "provider_in_use" => Some(ErrorPolicy {
            category: "conflict",
            retryable: false,
        }),
        "provider_not_found" => Some(ErrorPolicy {
            category: "not-found",
            retryable: false,
        }),
        "provider_unavailable" => Some(ErrorPolicy {
            category: "unavailable",
            retryable: true,
        }),
        "provider_unreachable" => Some(ErrorPolicy {
            category: "unavailable",
            retryable: true,
        }),
        "role_not_found" => Some(ErrorPolicy {
            category: "not-found",
            retryable: false,
        }),
        "rpc.invalid_json" => Some(ErrorPolicy {
            category: "validation",
            retryable: false,
        }),
        "rpc.invalid_request" => Some(ErrorPolicy {
            category: "validation",
            retryable: false,
        }),
        "rpc.missing_param" => Some(ErrorPolicy {
            category: "validation",
            retryable: false,
        }),
        "rpc.unknown_method" => Some(ErrorPolicy {
            category: "validation",
            retryable: false,
        }),
        "rpc.unsupported_protocol" => Some(ErrorPolicy {
            category: "unsupported",
            retryable: false,
        }),
        "run_in_progress" => Some(ErrorPolicy {
            category: "conflict",
            retryable: false,
        }),
        "session.asset_not_found" => Some(ErrorPolicy {
            category: "not-found",
            retryable: false,
        }),
        "session.concurrent_modification" => Some(ErrorPolicy {
            category: "conflict",
            retryable: true,
        }),
        "session.locator_stale" => Some(ErrorPolicy {
            category: "conflict",
            retryable: true,
        }),
        "session.not_found" => Some(ErrorPolicy {
            category: "not-found",
            retryable: false,
        }),
        "session.store_unavailable" => Some(ErrorPolicy {
            category: "unavailable",
            retryable: true,
        }),
        "session_exists" => Some(ErrorPolicy {
            category: "conflict",
            retryable: false,
        }),
        "session_not_found" => Some(ErrorPolicy {
            category: "not-found",
            retryable: false,
        }),
        "snapshot.invalid_source" => Some(ErrorPolicy {
            category: "validation",
            retryable: false,
        }),
        "summary.backbone_missing" => Some(ErrorPolicy {
            category: "not-found",
            retryable: false,
        }),
        "tool.not_allowed" => Some(ErrorPolicy {
            category: "permission",
            retryable: false,
        }),
        "tool.unknown" => Some(ErrorPolicy {
            category: "not-found",
            retryable: false,
        }),
        "unknown_method" => Some(ErrorPolicy {
            category: "validation",
            retryable: false,
        }),
        "unknown_tool_request" => Some(ErrorPolicy {
            category: "not-found",
            retryable: false,
        }),
        "unsupported" => Some(ErrorPolicy {
            category: "unsupported",
            retryable: false,
        }),
        "unsupported_protocol" => Some(ErrorPolicy {
            category: "unsupported",
            retryable: false,
        }),
        "workflow_already_started" => Some(ErrorPolicy {
            category: "conflict",
            retryable: false,
        }),
        _ => None,
    }
}
