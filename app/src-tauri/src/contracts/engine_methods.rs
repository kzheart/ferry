// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
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
    pub(crate) timeout: TimeoutClass,
    pub(crate) retry: RetryPolicy,
}

pub(crate) fn policy(method: &str) -> Option<EngineMethodPolicy> {
    match method {
        "health" => Some(EngineMethodPolicy {
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "version" => Some(EngineMethodPolicy {
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "scan" => Some(EngineMethodPolicy {
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "scan_progress" => Some(EngineMethodPolicy {
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "env" => Some(EngineMethodPolicy {
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "resume" => Some(EngineMethodPolicy {
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "models" => Some(EngineMethodPolicy {
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "history" => Some(EngineMethodPolicy {
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "pricing" => Some(EngineMethodPolicy {
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "show" => Some(EngineMethodPolicy {
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "session_asset" => Some(EngineMethodPolicy {
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "session_meta_list" => Some(EngineMethodPolicy {
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "session_search" => Some(EngineMethodPolicy {
            timeout: TimeoutClass::Lookup,
            retry: RetryPolicy::Never,
        }),
        "runtime_sessions.load_all" => Some(EngineMethodPolicy {
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "runtime_sessions.commit" => Some(EngineMethodPolicy {
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::Never,
        }),
        "runtime_sessions.delete" => Some(EngineMethodPolicy {
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::Never,
        }),
        "runtime_sessions.truncate" => Some(EngineMethodPolicy {
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::Never,
        }),
        "content_search" => Some(EngineMethodPolicy {
            timeout: TimeoutClass::Lookup,
            retry: RetryPolicy::Never,
        }),
        "session_read" => Some(EngineMethodPolicy {
            timeout: TimeoutClass::Lookup,
            retry: RetryPolicy::Never,
        }),
        "usage_stats" => Some(EngineMethodPolicy {
            timeout: TimeoutClass::Lookup,
            retry: RetryPolicy::Never,
        }),
        "agent_prompt" => Some(EngineMethodPolicy {
            timeout: TimeoutClass::AgentRun,
            retry: RetryPolicy::Never,
        }),
        "operation.plan" => Some(EngineMethodPolicy {
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::Never,
        }),
        "operation.apply" => Some(EngineMethodPolicy {
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::Never,
        }),
        "operation.status" => Some(EngineMethodPolicy {
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "operation.cancel" => Some(EngineMethodPolicy {
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::Never,
        }),
        _ => None,
    }
}

/// WebView 的通用 Engine RPC 通道允许按方法名调用的方法。
pub(crate) const UI_ENGINE_METHODS: &[&str] = &[
    "health",
    "version",
    "scan",
    "scan_progress",
    "env",
    "resume",
    "models",
    "history",
    "pricing",
    "show",
    "session_asset",
    "session_meta_list",
    "session_search",
];

pub(crate) fn is_ui_engine_method(method: &str) -> bool {
    UI_ENGINE_METHODS.contains(&method)
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
