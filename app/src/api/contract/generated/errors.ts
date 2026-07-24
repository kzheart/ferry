// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
export const FERRY_ERROR_POLICIES = {
  "agent.approval_invalid": {
    "category": "permission",
    "retryable": false,
    "sources": [
      "engine"
    ]
  },
  "agent.format_changed": {
    "category": "unsupported",
    "retryable": false,
    "sources": [
      "engine"
    ]
  },
  "agent.reference_invalid": {
    "category": "validation",
    "retryable": false,
    "sources": [
      "engine"
    ]
  },
  "agent.request_invalid": {
    "category": "validation",
    "retryable": false,
    "sources": [
      "engine"
    ]
  },
  "agent.run_missing": {
    "category": "validation",
    "retryable": false,
    "sources": [
      "host"
    ]
  },
  "already_restored": {
    "category": "conflict",
    "retryable": false,
    "sources": [
      "runtime"
    ]
  },
  "auth_in_progress": {
    "category": "conflict",
    "retryable": false,
    "sources": [
      "runtime"
    ]
  },
  "auth_login_not_found": {
    "category": "not-found",
    "retryable": false,
    "sources": [
      "runtime"
    ]
  },
  "auth_prompt_not_found": {
    "category": "not-found",
    "retryable": false,
    "sources": [
      "runtime"
    ]
  },
  "auth_type_unsupported": {
    "category": "unsupported",
    "retryable": false,
    "sources": [
      "runtime"
    ]
  },
  "edit.invalid_reply": {
    "category": "validation",
    "retryable": false,
    "sources": [
      "engine"
    ]
  },
  "edit.operation_unsupported": {
    "category": "unsupported",
    "retryable": false,
    "sources": [
      "engine"
    ]
  },
  "edit.subagent_not_supported": {
    "category": "unsupported",
    "retryable": false,
    "sources": [
      "engine"
    ]
  },
  "edit.turn_out_of_range": {
    "category": "validation",
    "retryable": false,
    "sources": [
      "engine"
    ]
  },
  "engine.invalid_response": {
    "category": "internal",
    "retryable": false,
    "sources": [
      "host"
    ]
  },
  "engine.request_failed": {
    "category": "internal",
    "retryable": false,
    "sources": [
      "host"
    ]
  },
  "engine.timeout": {
    "category": "unavailable",
    "retryable": true,
    "sources": [
      "host"
    ]
  },
  "engine.unavailable": {
    "category": "unavailable",
    "retryable": true,
    "sources": [
      "host"
    ]
  },
  "internal.unexpected": {
    "category": "internal",
    "retryable": false,
    "sources": [
      "engine"
    ]
  },
  "internal_error": {
    "category": "internal",
    "retryable": false,
    "sources": [
      "runtime"
    ]
  },
  "invalid_json": {
    "category": "validation",
    "retryable": false,
    "sources": [
      "runtime"
    ]
  },
  "invalid_params": {
    "category": "validation",
    "retryable": false,
    "sources": [
      "runtime"
    ]
  },
  "invalid_provider_config": {
    "category": "validation",
    "retryable": false,
    "sources": [
      "runtime"
    ]
  },
  "invalid_request": {
    "category": "validation",
    "retryable": false,
    "sources": [
      "runtime"
    ]
  },
  "invalid_role": {
    "category": "validation",
    "retryable": false,
    "sources": [
      "runtime"
    ]
  },
  "invalid_workflow": {
    "category": "validation",
    "retryable": false,
    "sources": [
      "runtime"
    ]
  },
  "model_capability_mismatch": {
    "category": "unsupported",
    "retryable": false,
    "sources": [
      "runtime"
    ]
  },
  "model_not_found": {
    "category": "not-found",
    "retryable": false,
    "sources": [
      "runtime"
    ]
  },
  "no_active_run": {
    "category": "conflict",
    "retryable": false,
    "sources": [
      "runtime"
    ]
  },
  "operation.invalid_status": {
    "category": "internal",
    "retryable": false,
    "sources": [
      "host"
    ]
  },
  "operation.unknown_status": {
    "category": "internal",
    "retryable": false,
    "sources": [
      "host"
    ]
  },
  "organization.proposal_invalid": {
    "category": "validation",
    "retryable": false,
    "sources": [
      "engine"
    ]
  },
  "organization.proposal_not_found": {
    "category": "not-found",
    "retryable": false,
    "sources": [
      "engine"
    ]
  },
  "organization.proposal_stale": {
    "category": "conflict",
    "retryable": true,
    "sources": [
      "engine"
    ]
  },
  "organization_failed": {
    "category": "execution",
    "retryable": false,
    "sources": [
      "runtime"
    ]
  },
  "organizer_invalid_response": {
    "category": "execution",
    "retryable": false,
    "sources": [
      "runtime"
    ]
  },
  "provider_in_use": {
    "category": "conflict",
    "retryable": false,
    "sources": [
      "runtime"
    ]
  },
  "provider_not_found": {
    "category": "not-found",
    "retryable": false,
    "sources": [
      "runtime"
    ]
  },
  "provider_unavailable": {
    "category": "unavailable",
    "retryable": true,
    "sources": [
      "runtime"
    ]
  },
  "provider_unreachable": {
    "category": "unavailable",
    "retryable": true,
    "sources": [
      "runtime"
    ]
  },
  "role_not_found": {
    "category": "not-found",
    "retryable": false,
    "sources": [
      "runtime"
    ]
  },
  "rpc.invalid_json": {
    "category": "validation",
    "retryable": false,
    "sources": [
      "engine"
    ]
  },
  "rpc.invalid_request": {
    "category": "validation",
    "retryable": false,
    "sources": [
      "engine"
    ]
  },
  "rpc.missing_param": {
    "category": "validation",
    "retryable": false,
    "sources": [
      "engine"
    ]
  },
  "rpc.unknown_method": {
    "category": "validation",
    "retryable": false,
    "sources": [
      "engine"
    ]
  },
  "rpc.unsupported_protocol": {
    "category": "unsupported",
    "retryable": false,
    "sources": [
      "engine"
    ]
  },
  "run_in_progress": {
    "category": "conflict",
    "retryable": false,
    "sources": [
      "runtime"
    ]
  },
  "session.asset_not_found": {
    "category": "not-found",
    "retryable": false,
    "sources": [
      "engine"
    ]
  },
  "session.concurrent_modification": {
    "category": "conflict",
    "retryable": true,
    "sources": [
      "engine"
    ]
  },
  "session.locator_stale": {
    "category": "conflict",
    "retryable": true,
    "sources": [
      "engine"
    ]
  },
  "session.not_found": {
    "category": "not-found",
    "retryable": false,
    "sources": [
      "engine"
    ]
  },
  "session.store_unavailable": {
    "category": "unavailable",
    "retryable": true,
    "sources": [
      "engine"
    ]
  },
  "session_exists": {
    "category": "conflict",
    "retryable": false,
    "sources": [
      "runtime"
    ]
  },
  "session_not_found": {
    "category": "not-found",
    "retryable": false,
    "sources": [
      "runtime"
    ]
  },
  "snapshot.invalid_source": {
    "category": "validation",
    "retryable": false,
    "sources": [
      "engine"
    ]
  },
  "summary.backbone_missing": {
    "category": "not-found",
    "retryable": false,
    "sources": [
      "engine"
    ]
  },
  "tool.not_allowed": {
    "category": "permission",
    "retryable": false,
    "sources": [
      "host"
    ]
  },
  "tool.unknown": {
    "category": "not-found",
    "retryable": false,
    "sources": [
      "engine"
    ]
  },
  "unknown_method": {
    "category": "validation",
    "retryable": false,
    "sources": [
      "runtime"
    ]
  },
  "unknown_tool_request": {
    "category": "not-found",
    "retryable": false,
    "sources": [
      "runtime"
    ]
  },
  "unsupported": {
    "category": "unsupported",
    "retryable": false,
    "sources": [
      "runtime"
    ]
  },
  "unsupported_protocol": {
    "category": "unsupported",
    "retryable": false,
    "sources": [
      "runtime"
    ]
  },
  "workflow_already_started": {
    "category": "conflict",
    "retryable": false,
    "sources": [
      "runtime"
    ]
  }
} as const;
export type FerryErrorCode = keyof typeof FERRY_ERROR_POLICIES;
export const isFerryErrorCode = (value: unknown): value is FerryErrorCode =>
  typeof value === "string" &&
  Object.prototype.hasOwnProperty.call(FERRY_ERROR_POLICIES, value);
