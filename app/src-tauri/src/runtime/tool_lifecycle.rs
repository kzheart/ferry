//! Runtime 工具请求的宿主生命周期。读线程先登记，再派发工作线程。
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct RequestKey {
    session_id: String,
    run_id: String,
    request_id: String,
}

#[derive(Default)]
struct RequestState {
    cancelled: AtomicBool,
    awaiting_approval: AtomicBool,
}

#[derive(Clone)]
pub(super) struct ToolRequest {
    key: RequestKey,
    state: Arc<RequestState>,
}

type Registry = HashMap<RequestKey, Arc<RequestState>>;
static REQUESTS: OnceLock<Mutex<Registry>> = OnceLock::new();

fn requests() -> MutexGuard<'static, Registry> {
    REQUESTS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl ToolRequest {
    pub(super) fn register(session_id: &str, run_id: &str, request_id: &str) -> Self {
        let key = RequestKey {
            session_id: session_id.to_owned(),
            run_id: run_id.to_owned(),
            request_id: request_id.to_owned(),
        };
        let state = Arc::new(RequestState::default());
        if let Some(previous) = requests().insert(key.clone(), state.clone()) {
            previous.cancelled.store(true, Ordering::Release);
        }
        Self { key, state }
    }

    pub(super) fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::Acquire)
    }

    pub(super) fn check(&self) -> Result<(), String> {
        if self.is_cancelled() {
            Err("tool request aborted".to_owned())
        } else {
            Ok(())
        }
    }

    pub(super) fn start<T>(&self, action: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        // 与取消登记串行化，避免已取消请求在 check 与 spawn 之间启动进程。
        let _registry = requests();
        self.check()?;
        action()
    }

    pub(super) fn await_approval(&self) {
        self.state.awaiting_approval.store(true, Ordering::Release);
    }

    pub(super) fn finish(&self) {
        let mut registry = requests();
        if registry
            .get(&self.key)
            .is_some_and(|state| Arc::ptr_eq(state, &self.state))
        {
            registry.remove(&self.key);
        }
    }

    pub(super) fn finish_dispatch(&self) {
        if !self.state.awaiting_approval.load(Ordering::Acquire) || self.is_cancelled() {
            self.finish();
        }
    }
}

pub(super) fn cancel(session_id: &str, run_id: &str, request_id: &str) {
    let key = RequestKey {
        session_id: session_id.to_owned(),
        run_id: run_id.to_owned(),
        request_id: request_id.to_owned(),
    };
    if let Some(state) = requests().remove(&key) {
        state.cancelled.store(true, Ordering::Release);
    }
}

pub(super) fn finish_run(session_id: &str, run_id: &str, completed: bool) {
    requests().retain(|key, state| {
        if key.session_id != session_id || key.run_id != run_id {
            return true;
        }
        // 正常完成后的审批卡仍可执行；取消/失败则撤销尚未批准的请求。
        if completed && state.awaiting_approval.load(Ordering::Acquire) {
            return true;
        }
        state.cancelled.store(true, Ordering::Release);
        false
    });
}

pub(super) fn cancel_all() {
    for (_, state) in requests().drain() {
        state.cancelled.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_is_scoped_to_request_and_run() {
        let a = ToolRequest::register("scope", "run-a", "a");
        let b = ToolRequest::register("scope", "run-b", "b");
        cancel("scope", "wrong-run", "a");
        assert!(!a.is_cancelled());
        finish_run("scope", "run-a", false);
        assert!(a.is_cancelled());
        assert!(!b.is_cancelled());
        b.finish();
    }

    #[test]
    fn completed_run_preserves_manual_approval_but_cancelled_run_revokes_it() {
        let request = ToolRequest::register("approval", "run", "request");
        request.await_approval();
        request.finish_dispatch();
        finish_run("approval", "run", true);
        assert!(!request.is_cancelled());
        finish_run("approval", "run", false);
        assert!(request.is_cancelled());
    }
}
