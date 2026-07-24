// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TimeoutClass {
    Normal,
    Lookup,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RetryPolicy {
    SafeRead,
    Never,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EngineMethodPolicy {
    pub(crate) is_public: bool,
    pub(crate) timeout: TimeoutClass,
    pub(crate) retry: RetryPolicy,
}

pub(crate) fn policy(method: &str) -> Option<EngineMethodPolicy> {
    match method {
        "health" => Some(EngineMethodPolicy {
            is_public: true,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "version" => Some(EngineMethodPolicy {
            is_public: true,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "scan" => Some(EngineMethodPolicy {
            is_public: true,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "env" => Some(EngineMethodPolicy {
            is_public: true,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "resume" => Some(EngineMethodPolicy {
            is_public: true,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "models" => Some(EngineMethodPolicy {
            is_public: true,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "history" => Some(EngineMethodPolicy {
            is_public: true,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "history_delete" => Some(EngineMethodPolicy {
            is_public: true,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::Never,
        }),
        "pricing" => Some(EngineMethodPolicy {
            is_public: true,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "show" => Some(EngineMethodPolicy {
            is_public: true,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "session_asset" => Some(EngineMethodPolicy {
            is_public: true,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "edit_capabilities" => Some(EngineMethodPolicy {
            is_public: true,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "session_meta_list" => Some(EngineMethodPolicy {
            is_public: true,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "session_backbone" => Some(EngineMethodPolicy {
            is_public: true,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "session_summaries_set" => Some(EngineMethodPolicy {
            is_public: true,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::Never,
        }),
        "organization_digest_context" => Some(EngineMethodPolicy {
            is_public: false,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "organization_propose" => Some(EngineMethodPolicy {
            is_public: false,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::Never,
        }),
        "organization_proposals_list" => Some(EngineMethodPolicy {
            is_public: false,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "organization_proposal_modify" => Some(EngineMethodPolicy {
            is_public: false,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::Never,
        }),
        "organization_proposal_decide" => Some(EngineMethodPolicy {
            is_public: false,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::Never,
        }),
        "runtime_sessions.load_all" => Some(EngineMethodPolicy {
            is_public: false,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "runtime_sessions.commit" => Some(EngineMethodPolicy {
            is_public: false,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::Never,
        }),
        "runtime_sessions.delete" => Some(EngineMethodPolicy {
            is_public: false,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::Never,
        }),
        "agent_search_sessions" => Some(EngineMethodPolicy {
            is_public: true,
            timeout: TimeoutClass::Lookup,
            retry: RetryPolicy::Never,
        }),
        "agent_session_read" => Some(EngineMethodPolicy {
            is_public: true,
            timeout: TimeoutClass::Lookup,
            retry: RetryPolicy::Never,
        }),
        "agent_get_usage" => Some(EngineMethodPolicy {
            is_public: true,
            timeout: TimeoutClass::Lookup,
            retry: RetryPolicy::Never,
        }),
        "operation.plan" => Some(EngineMethodPolicy {
            is_public: false,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::Never,
        }),
        "operation.apply" => Some(EngineMethodPolicy {
            is_public: false,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::Never,
        }),
        "operation.status" => Some(EngineMethodPolicy {
            is_public: false,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::SafeRead,
        }),
        "operation.cancel" => Some(EngineMethodPolicy {
            is_public: false,
            timeout: TimeoutClass::Normal,
            retry: RetryPolicy::Never,
        }),
        _ => None,
    }
}
