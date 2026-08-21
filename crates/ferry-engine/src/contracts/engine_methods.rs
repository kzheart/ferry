// 此文件由 scripts/generate-contracts.py 生成，请勿手改。

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MethodKind {
    Read,
    IndexRefresh,
    Mutation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimeoutClass {
    Normal,
    Lookup,
    AgentRun,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetryPolicy {
    SafeRead,
    Never,
}

/// 分发池归属：control 走独立轻量控制池，parallel-read 走 4-worker 只读池，
/// 其余方法一律单 worker 串行保序。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Dispatch {
    Control,
    ParallelRead,
    Serial,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EngineMethodPolicy {
    pub kind: MethodKind,
    pub timeout: TimeoutClass,
    pub retry: RetryPolicy,
    pub dispatch: Dispatch,
}

/// RPC 方法全集；启动自检要求分发表与它逐字相等。
pub const ENGINE_METHOD_NAMES: &[&str] = &[
    "health",
    "version",
    "scan",
    "scan_progress",
    "env",
    "resume",
    "models",
    "history",
    "history_delete",
    "pricing",
    "show",
    "session_asset",
    "session_meta_list",
    "session_search",
    "runtime_sessions.load_all",
    "runtime_sessions.commit",
    "runtime_sessions.delete",
    "runtime_sessions.truncate",
    "content_search",
    "session_read",
    "usage_stats",
    "agent_prompt",
    "operation.plan",
    "operation.apply",
    "operation.status",
    "operation.cancel",
];

/// 允许进并发只读池的方法。
pub const PARALLEL_READ_METHOD_NAMES: &[&str] = &[
    "env",
    "models",
    "history",
    "show",
    "session_asset",
    "session_meta_list",
];

/// 进独立轻量控制池的方法，不得被重读队列阻塞。
pub const CONTROL_METHOD_NAMES: &[&str] = &["health", "version", "scan_progress"];

/// callers 含 cli 的方法：本地 socket 传输只分发这一集合。
pub const CLI_METHOD_NAMES: &[&str] = &[
    "health",
    "version",
    "scan",
    "scan_progress",
    "env",
    "resume",
    "models",
    "history",
    "session_meta_list",
    "content_search",
    "session_read",
    "usage_stats",
    "operation.plan",
    "operation.apply",
    "operation.status",
    "operation.cancel",
];

pub fn policy(method: &str) -> Option<EngineMethodPolicy> {
    match method {
        "health" => Some(EngineMethodPolicy {
            kind: MethodKind::Read,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
            dispatch: Dispatch::Control,
        }),
        "version" => Some(EngineMethodPolicy {
            kind: MethodKind::Read,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
            dispatch: Dispatch::Control,
        }),
        "scan" => Some(EngineMethodPolicy {
            kind: MethodKind::IndexRefresh,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
            dispatch: Dispatch::Serial,
        }),
        "scan_progress" => Some(EngineMethodPolicy {
            kind: MethodKind::Read,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
            dispatch: Dispatch::Control,
        }),
        "env" => Some(EngineMethodPolicy {
            kind: MethodKind::Read,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
            dispatch: Dispatch::ParallelRead,
        }),
        "resume" => Some(EngineMethodPolicy {
            kind: MethodKind::Read,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
            dispatch: Dispatch::Serial,
        }),
        "models" => Some(EngineMethodPolicy {
            kind: MethodKind::Read,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
            dispatch: Dispatch::ParallelRead,
        }),
        "history" => Some(EngineMethodPolicy {
            kind: MethodKind::Read,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
            dispatch: Dispatch::ParallelRead,
        }),
        "history_delete" => Some(EngineMethodPolicy {
            kind: MethodKind::Mutation,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::Never,
            dispatch: Dispatch::Serial,
        }),
        "pricing" => Some(EngineMethodPolicy {
            kind: MethodKind::IndexRefresh,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
            dispatch: Dispatch::Serial,
        }),
        "show" => Some(EngineMethodPolicy {
            kind: MethodKind::Read,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
            dispatch: Dispatch::ParallelRead,
        }),
        "session_asset" => Some(EngineMethodPolicy {
            kind: MethodKind::Read,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
            dispatch: Dispatch::ParallelRead,
        }),
        "session_meta_list" => Some(EngineMethodPolicy {
            kind: MethodKind::Read,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
            dispatch: Dispatch::ParallelRead,
        }),
        "session_search" => Some(EngineMethodPolicy {
            kind: MethodKind::Read,
            timeout: TimeoutClass::Lookup,
            retry: RetryPolicy::Never,
            dispatch: Dispatch::Serial,
        }),
        "runtime_sessions.load_all" => Some(EngineMethodPolicy {
            kind: MethodKind::Read,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
            dispatch: Dispatch::Serial,
        }),
        "runtime_sessions.commit" => Some(EngineMethodPolicy {
            kind: MethodKind::Mutation,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::Never,
            dispatch: Dispatch::Serial,
        }),
        "runtime_sessions.delete" => Some(EngineMethodPolicy {
            kind: MethodKind::Mutation,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::Never,
            dispatch: Dispatch::Serial,
        }),
        "runtime_sessions.truncate" => Some(EngineMethodPolicy {
            kind: MethodKind::Mutation,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::Never,
            dispatch: Dispatch::Serial,
        }),
        "content_search" => Some(EngineMethodPolicy {
            kind: MethodKind::Read,
            timeout: TimeoutClass::Lookup,
            retry: RetryPolicy::Never,
            dispatch: Dispatch::Serial,
        }),
        "session_read" => Some(EngineMethodPolicy {
            kind: MethodKind::Read,
            timeout: TimeoutClass::Lookup,
            retry: RetryPolicy::Never,
            dispatch: Dispatch::Serial,
        }),
        "usage_stats" => Some(EngineMethodPolicy {
            kind: MethodKind::Read,
            timeout: TimeoutClass::Lookup,
            retry: RetryPolicy::Never,
            dispatch: Dispatch::Serial,
        }),
        "agent_prompt" => Some(EngineMethodPolicy {
            kind: MethodKind::Mutation,
            timeout: TimeoutClass::AgentRun,
            retry: RetryPolicy::Never,
            dispatch: Dispatch::Serial,
        }),
        "operation.plan" => Some(EngineMethodPolicy {
            kind: MethodKind::Mutation,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::Never,
            dispatch: Dispatch::Serial,
        }),
        "operation.apply" => Some(EngineMethodPolicy {
            kind: MethodKind::Mutation,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::Never,
            dispatch: Dispatch::Serial,
        }),
        "operation.status" => Some(EngineMethodPolicy {
            kind: MethodKind::Read,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
            dispatch: Dispatch::Serial,
        }),
        "operation.cancel" => Some(EngineMethodPolicy {
            kind: MethodKind::Mutation,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::Never,
            dispatch: Dispatch::Serial,
        }),
        _ => None,
    }
}

pub fn is_parallel_read(method: &str) -> bool {
    PARALLEL_READ_METHOD_NAMES.contains(&method)
}

pub fn is_control(method: &str) -> bool {
    CONTROL_METHOD_NAMES.contains(&method)
}

pub fn is_cli_method(method: &str) -> bool {
    CLI_METHOD_NAMES.contains(&method)
}
