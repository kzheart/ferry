//! Engine RPC 的超时与重试策略；不依赖进程生命周期或 Tauri。

use crate::contracts::engine_methods::{self, RetryPolicy, TimeoutClass};
use serde_json::Value;
use std::time::Duration;

pub(crate) const ENGINE_TIMEOUT: Duration = Duration::from_secs(120);
pub(crate) const AGENT_LOOKUP_TIMEOUT: Duration = Duration::from_secs(20);
pub(crate) const ENGINE_COMMIT_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);

pub(crate) fn request_timeout(request: &str) -> Duration {
    match request_policy(request).map(|policy| policy.timeout) {
        Some(TimeoutClass::Lookup) => AGENT_LOOKUP_TIMEOUT,
        Some(TimeoutClass::Commit) => ENGINE_COMMIT_TIMEOUT,
        Some(TimeoutClass::Normal) | None => ENGINE_TIMEOUT,
    }
}

pub(crate) fn request_attempts(request: &str) -> u8 {
    match request_policy(request).map(|policy| policy.retry) {
        Some(RetryPolicy::SafeRead) => 2,
        Some(RetryPolicy::Never) | None => 1,
    }
}

fn request_policy(request: &str) -> Option<engine_methods::EngineMethodPolicy> {
    serde_json::from_str::<Value>(request)
        .ok()
        .and_then(|value| {
            value
                .get("method")
                .and_then(Value::as_str)
                .map(engine_methods::policy)
        })
        .flatten()
}
