//! 统一写操作计划：plan / apply / status / cancel 的门面与单 worker 队列。
//!
//! 硬约束：
//! - 所有写操作在**同一个单 worker 队列**里串行执行（`MUTATION_WORKERS = 1`），
//!   IPC 请求立即返回，不放宽 adapter 的写并发假设；
//! - `_run` 里 `execute` 必须在锁外（§2.4 第 22 条）；
//! - cancel 不中断执行中的任务：worker 见状态非 `queued` 即静默退出，零写入（第 21 条）；
//! - 崩溃恢复只在本 service 打开状态库时跑一次（`OperationPlanStore::database`
//!   用 `recover_interrupted = true`，metadata/history 路径用 false，§2.3 第 20 条）。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::{Map, Value};

use crate::errors::DomainError;
use crate::operations::executor::OperationExecutor;
use crate::operations::plan_store::{OperationPlan, OperationPlanStore, OperationState};
use crate::operations::planner::OperationPlanner;
use crate::operations::state_store::AuditEntry;
use crate::operations::types::{EngineError, EngineResult, Ports, Resolver};
use crate::storage::database::{canonical_json, digest_json, Clock, SystemClock};

pub const MUTATION_WORKERS: usize = 1;

/// 一次入队作业的完成信号。
#[derive(Default)]
struct Job {
    outcome: Mutex<Option<Result<(), EngineError>>>,
    signal: Condvar,
}

impl Job {
    fn complete(&self, outcome: Result<(), EngineError>) {
        *self
            .outcome
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(outcome);
        self.signal.notify_all();
    }

    fn wait(&self, timeout: Option<Duration>) -> Option<Result<(), EngineError>> {
        let mut guard = self
            .outcome
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        match timeout {
            Some(timeout) => {
                let (next, _) = self
                    .signal
                    .wait_timeout_while(guard, timeout, |outcome| outcome.is_none())
                    .unwrap_or_else(|error| error.into_inner());
                guard = next;
            }
            None => {
                while guard.is_none() {
                    guard = self
                        .signal
                        .wait(guard)
                        .unwrap_or_else(|error| error.into_inner());
                }
            }
        }
        guard.clone()
    }
}

struct ServiceInner {
    /// 对齐 Python 的 `threading.RLock()`：状态读写与入队的互斥区。
    lock: Mutex<()>,
    plans: OperationPlanStore,
    planner: OperationPlanner,
    executor: OperationExecutor,
    clock: Arc<dyn Clock>,
    jobs: Mutex<HashMap<String, Arc<Job>>>,
}

pub struct OperationService {
    inner: Arc<ServiceInner>,
    sender: Mutex<Option<Sender<String>>>,
    worker: Mutex<Option<JoinHandle<()>>>,
    cancelled: Arc<AtomicBool>,
}

impl OperationService {
    pub fn new(ports: Ports, index: Resolver) -> Self {
        Self::with_clock(ports, index, Arc::new(SystemClock))
    }

    pub fn with_clock(ports: Ports, index: Resolver, clock: Arc<dyn Clock>) -> Self {
        let inner = Arc::new(ServiceInner {
            lock: Mutex::new(()),
            plans: OperationPlanStore::new(ports.state_dir(), Arc::clone(&clock)),
            planner: OperationPlanner::new(Ports::clone(&ports), Resolver::clone(&index)),
            executor: OperationExecutor::new(ports, index),
            clock,
            jobs: Mutex::new(HashMap::new()),
        });
        let (sender, receiver) = channel::<String>();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker = {
            let inner = Arc::clone(&inner);
            let cancelled = Arc::clone(&cancelled);
            std::thread::Builder::new()
                .name("engine-operation".into())
                .spawn(move || {
                    for plan_id in receiver {
                        // shutdown(cancel_futures=True)：已入队未开始的任务直接丢弃。
                        if cancelled.load(Ordering::SeqCst) {
                            break;
                        }
                        let outcome = inner.run(&plan_id);
                        let job = inner
                            .jobs
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .get(&plan_id)
                            .cloned();
                        if let Some(job) = job {
                            job.complete(outcome);
                        }
                    }
                })
                .expect("无法启动 operation worker 线程")
        };
        Self {
            inner,
            sender: Mutex::new(Some(sender)),
            worker: Mutex::new(Some(worker)),
            cancelled,
        }
    }

    pub fn plan(&self, value: &Value) -> EngineResult<Value> {
        let prepared = self.inner.planner.plan(value)?;
        let _guard = self
            .inner
            .lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.inner.plans.create(
            &prepared.input,
            &prepared.preview,
            &prepared.base_revision,
            prepared.document_revision.as_deref(),
        )
    }

    pub fn apply(&self, plan_id: &Value) -> EngineResult<Value> {
        {
            let _guard = self
                .inner
                .lock
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let (operation, mut state) = self.inner.plans.get(plan_id)?;
            self.inner.plans.expire(&operation, &mut state)?;
            if state.status != "planned" {
                return Err(not_applicable(&operation.plan_id, &state.status));
            }
            // 一次性批准：靠 UPDATE ... WHERE status='planned' 的 rowcount，
            // 而不是上面那次读——两个线程会读到同一个 planned。
            if !self
                .inner
                .plans
                .database()?
                .operations
                .enqueue(&operation.plan_id, self.inner.clock.now_ms())?
            {
                let (_operation, current) = self.inner.plans.get(plan_id)?;
                return Err(not_applicable(&operation.plan_id, &current.status));
            }
            let job = Arc::new(Job::default());
            self.inner
                .jobs
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .insert(operation.plan_id.clone(), job);
            if let Some(sender) = self
                .sender
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_ref()
            {
                let _ = sender.send(operation.plan_id.clone());
            }
        }
        self.status(plan_id)
    }

    pub fn status(&self, plan_id: &Value) -> EngineResult<Value> {
        let _guard = self
            .inner
            .lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let (operation, mut state) = self.inner.plans.get(plan_id)?;
        self.inner.plans.expire(&operation, &mut state)?;
        Ok(Value::Object(status_dto(&operation, &state)?))
    }

    pub fn cancel(&self, plan_id: &Value) -> EngineResult<Value> {
        let _guard = self
            .inner
            .lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let (operation, mut state) = self.inner.plans.get(plan_id)?;
        self.inner.plans.expire(&operation, &mut state)?;
        if !matches!(state.status.as_str(), "planned" | "queued") {
            let mut params = Map::new();
            params.insert("plan_id".into(), Value::from(operation.plan_id.as_str()));
            params.insert("status".into(), Value::from(state.status.as_str()));
            return Err(DomainError::new(
                "agent.request_invalid",
                "AgentRequestError",
                "仅 planned 或 queued operation 可以取消",
                params,
            )
            .into());
        }
        if !self.inner.plans.database()?.operations.cancel(
            &operation.plan_id,
            &state.status,
            self.inner.clock.now_ms(),
        )? {
            let mut params = Map::new();
            params.insert("plan_id".into(), Value::from(operation.plan_id.as_str()));
            return Err(DomainError::new(
                "agent.request_invalid",
                "AgentRequestError",
                "operation plan 当前状态不可取消",
                params,
            )
            .into());
        }
        let mut result = Map::new();
        result.insert("plan_id".into(), Value::from(operation.plan_id.as_str()));
        result.insert("status".into(), Value::from("cancelled"));
        Ok(Value::Object(result))
    }

    /// 测试与进程内编排辅助：等待已排队任务，不属于 RPC surface。
    ///
    /// 与 `Future.result()` 一致——worker 里抛出的异常在这里原样重现。
    pub fn wait(&self, plan_id: &Value, timeout: Option<Duration>) -> EngineResult<Value> {
        let key = plan_id.as_str().unwrap_or_default().to_string();
        let job = self
            .inner
            .jobs
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&key)
            .cloned();
        if let Some(job) = job {
            if let Some(Err(error)) = job.wait(timeout) {
                return Err(error);
            }
        }
        self.status(plan_id)
    }

    pub fn audit(&self, plan_id: &Value) -> EngineResult<Vec<AuditEntry>> {
        let _guard = self
            .inner
            .lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let (operation, _state) = self.inner.plans.get(plan_id)?;
        self.inner
            .plans
            .database()?
            .operations
            .audit(&operation.plan_id)
    }

    /// 仅在 Engine 重建或测试清理时调用，确保不遗留后台写入线程。
    pub fn shutdown(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.sender
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Some(worker) = self
            .worker
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            let _ = worker.join();
        }
    }
}

impl Drop for OperationService {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl ServiceInner {
    fn run(&self, plan_id: &str) -> Result<(), EngineError> {
        let key = Value::from(plan_id);
        let operation = {
            let _guard = self.lock.lock().unwrap_or_else(|error| error.into_inner());
            let (operation, state) = self.plans.get(&key)?;
            // cancel 不中断执行中的任务，但已取消的任务在这里静默退出，零写入。
            if state.status != "queued" {
                return Ok(());
            }
            if !self
                .plans
                .database()?
                .operations
                .claim_queued(plan_id, self.clock.now_ms())?
            {
                return Ok(());
            }
            operation
        };

        // execute 在锁外：写操作可能跑很久，握着锁会把 status/cancel 一起堵死。
        let result = match self.executor.execute(&operation) {
            Ok(result) => result,
            Err(error) => {
                let _guard = self.lock.lock().unwrap_or_else(|error| error.into_inner());
                self.plans.database()?.operations.fail(
                    plan_id,
                    error.error_type(),
                    self.clock.now_ms(),
                )?;
                return Err(error);
            }
        };
        let result_json = canonical_json(&result)?;
        let digest = digest_json(&result_json);
        let _guard = self.lock.lock().unwrap_or_else(|error| error.into_inner());
        self.plans.database()?.operations.finish(
            plan_id,
            &result_json,
            &digest,
            self.clock.now_ms(),
        )
    }
}

fn not_applicable(plan_id: &str, status: &str) -> EngineError {
    let mut params = Map::new();
    params.insert("plan_id".into(), Value::from(plan_id));
    params.insert("status".into(), Value::from(status));
    DomainError::new(
        "agent.request_invalid",
        "AgentRequestError",
        "operation plan 当前状态不可执行",
        params,
    )
    .into()
}

fn status_dto(
    operation: &OperationPlan,
    state: &OperationState,
) -> EngineResult<Map<String, Value>> {
    let mut result = Map::new();
    result.insert("plan_id".into(), Value::from(operation.plan_id.as_str()));
    result.insert("kind".into(), Value::from(operation.kind.as_str()));
    result.insert("status".into(), Value::from(state.status.as_str()));
    result.insert("created_at".into(), Value::from(operation.created_at));
    result.insert("expires_at".into(), Value::from(operation.expires_at));
    result.insert("updated_at".into(), Value::from(state.updated_at));
    if let Some(error_type) = state.error_type.as_deref().filter(|text| !text.is_empty()) {
        result.insert("error_type".into(), Value::from(error_type));
    }
    if let Some(result_json) = &state.result_json {
        result.insert(
            "result".into(),
            serde_json::from_str(result_json)
                .map_err(|error| EngineError::value_error(error.to_string()))?,
        );
    }
    Ok(result)
}
