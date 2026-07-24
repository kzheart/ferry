// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
export const RUNTIME_ERROR_POLICIES = {
  already_restored: {
    category: "conflict",
    retryable: false,
  },
  auth_in_progress: {
    category: "conflict",
    retryable: false,
  },
  auth_login_not_found: {
    category: "not-found",
    retryable: false,
  },
  auth_prompt_not_found: {
    category: "not-found",
    retryable: false,
  },
  auth_type_unsupported: {
    category: "unsupported",
    retryable: false,
  },
  internal_error: {
    category: "internal",
    retryable: false,
  },
  invalid_json: {
    category: "validation",
    retryable: false,
  },
  invalid_params: {
    category: "validation",
    retryable: false,
  },
  invalid_provider_config: {
    category: "validation",
    retryable: false,
  },
  invalid_request: {
    category: "validation",
    retryable: false,
  },
  invalid_role: {
    category: "validation",
    retryable: false,
  },
  invalid_workflow: {
    category: "validation",
    retryable: false,
  },
  model_capability_mismatch: {
    category: "unsupported",
    retryable: false,
  },
  model_not_found: {
    category: "not-found",
    retryable: false,
  },
  no_active_run: {
    category: "conflict",
    retryable: false,
  },
  organization_failed: {
    category: "execution",
    retryable: false,
  },
  organizer_invalid_response: {
    category: "execution",
    retryable: false,
  },
  provider_in_use: {
    category: "conflict",
    retryable: false,
  },
  provider_not_found: {
    category: "not-found",
    retryable: false,
  },
  provider_unavailable: {
    category: "unavailable",
    retryable: true,
  },
  provider_unreachable: {
    category: "unavailable",
    retryable: true,
  },
  role_not_found: {
    category: "not-found",
    retryable: false,
  },
  run_in_progress: {
    category: "conflict",
    retryable: false,
  },
  session_exists: {
    category: "conflict",
    retryable: false,
  },
  session_not_found: {
    category: "not-found",
    retryable: false,
  },
  unknown_method: {
    category: "validation",
    retryable: false,
  },
  unknown_tool_request: {
    category: "not-found",
    retryable: false,
  },
  unsupported: {
    category: "unsupported",
    retryable: false,
  },
  unsupported_protocol: {
    category: "unsupported",
    retryable: false,
  },
  workflow_already_started: {
    category: "conflict",
    retryable: false,
  },
} as const;
export type RuntimeErrorCode = keyof typeof RUNTIME_ERROR_POLICIES;
export function runtimeErrorPolicy(code: RuntimeErrorCode) {
  return RUNTIME_ERROR_POLICIES[code];
}
