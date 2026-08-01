// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Exposure {
    Public,
    TrustedUi,
    Internal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TimeoutClass {
    Normal,
    Lookup,
    AgentRun,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RetryPolicy {
    SafeRead,
    Never,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EngineMethodPolicy {
    pub(crate) exposure: Exposure,
    pub(crate) timeout: TimeoutClass,
    pub(crate) retry: RetryPolicy,
}

pub(crate) fn policy(method: &str) -> Option<EngineMethodPolicy> {
    match method {
        "health" => Some(EngineMethodPolicy {
            exposure: Exposure::Public,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "version" => Some(EngineMethodPolicy {
            exposure: Exposure::Public,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "scan" => Some(EngineMethodPolicy {
            exposure: Exposure::Public,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "scan_progress" => Some(EngineMethodPolicy {
            exposure: Exposure::Public,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "env" => Some(EngineMethodPolicy {
            exposure: Exposure::Public,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "resume" => Some(EngineMethodPolicy {
            exposure: Exposure::Public,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "models" => Some(EngineMethodPolicy {
            exposure: Exposure::Public,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "history" => Some(EngineMethodPolicy {
            exposure: Exposure::Public,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "history_delete" => Some(EngineMethodPolicy {
            exposure: Exposure::Public,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::Never,
        }),
        "pricing" => Some(EngineMethodPolicy {
            exposure: Exposure::Public,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "show" => Some(EngineMethodPolicy {
            exposure: Exposure::Public,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "session_asset" => Some(EngineMethodPolicy {
            exposure: Exposure::Public,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "session_meta_list" => Some(EngineMethodPolicy {
            exposure: Exposure::Public,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "session_search" => Some(EngineMethodPolicy {
            exposure: Exposure::Public,
            timeout: TimeoutClass::Lookup,
            retry: RetryPolicy::Never,
        }),
        "runtime_sessions.load_all" => Some(EngineMethodPolicy {
            exposure: Exposure::Internal,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "runtime_sessions.commit" => Some(EngineMethodPolicy {
            exposure: Exposure::Internal,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::Never,
        }),
        "runtime_sessions.delete" => Some(EngineMethodPolicy {
            exposure: Exposure::Internal,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::Never,
        }),
        "runtime_sessions.truncate" => Some(EngineMethodPolicy {
            exposure: Exposure::Internal,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::Never,
        }),
        "agent_search_sessions" => Some(EngineMethodPolicy {
            exposure: Exposure::Internal,
            timeout: TimeoutClass::Lookup,
            retry: RetryPolicy::Never,
        }),
        "agent_session_read" => Some(EngineMethodPolicy {
            exposure: Exposure::Internal,
            timeout: TimeoutClass::Lookup,
            retry: RetryPolicy::Never,
        }),
        "agent_get_usage" => Some(EngineMethodPolicy {
            exposure: Exposure::Internal,
            timeout: TimeoutClass::Lookup,
            retry: RetryPolicy::Never,
        }),
        "agent_cleanup_inventory" => Some(EngineMethodPolicy {
            exposure: Exposure::Internal,
            timeout: TimeoutClass::Lookup,
            retry: RetryPolicy::Never,
        }),
        "agent_cleanup_triage" => Some(EngineMethodPolicy {
            exposure: Exposure::Internal,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::Never,
        }),
        "agent_prompt" => Some(EngineMethodPolicy {
            exposure: Exposure::Internal,
            timeout: TimeoutClass::AgentRun,
            retry: RetryPolicy::Never,
        }),
        "operation.plan" => Some(EngineMethodPolicy {
            exposure: Exposure::Internal,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::Never,
        }),
        "operation.apply" => Some(EngineMethodPolicy {
            exposure: Exposure::Internal,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::Never,
        }),
        "operation.status" => Some(EngineMethodPolicy {
            exposure: Exposure::Internal,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "operation.cancel" => Some(EngineMethodPolicy {
            exposure: Exposure::Internal,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::Never,
        }),
        _ => None,
    }
}

/// Ferry Runtime 允许经网关转发到 Engine 的方法白名单。
pub(crate) const RUNTIME_GATEWAY_METHODS: &[&str] = &[
    "runtime_sessions.load_all",
    "runtime_sessions.commit",
    "runtime_sessions.delete",
    "runtime_sessions.truncate",
    "agent_prompt",
];

pub(crate) fn is_runtime_gateway_method(method: &str) -> bool {
    RUNTIME_GATEWAY_METHODS.contains(&method)
}
