//! EngineService 门面：把 26 个 RPC 方法映射到能力包。
//!
//! 分发层（`server::rpc`）只做信封校验与默认值填充，把**原始 JSON 值**交到这里
//! ——与 Python 的 `lambda p: application.xxx(p["tool"], ...)` 完全一致。因此本
//! 模块里所有的 `&Value` → 具体类型的转换都必须复刻 Python 在同一位置的行为
//! （`isinstance` 假分支落到哪个错误、缺键与显式 `null` 的区别等）。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Map, Value};

use crate::context::EngineContext;
use crate::contracts::ipc::FERRY_CONTRACT_HASH;
use crate::errors::{DomainError, DomainResult};
use crate::jsonutil::{python_str, python_truthy};
use crate::operations::service::OperationService;
use crate::operations::types::{
    EngineError, EngineResult, IndexedSession as OperationSession, Ports, ResolvedMessageLocator,
    SessionResolver,
};
use crate::operations::{history, metadata, verification};
use crate::runtime::sessions as runtime_sessions;
use crate::server::notify::Notifier;
use crate::server::rpc::{ContentSearchRequest, EngineService, SessionReadRequest};
use crate::sessions::content_index::ContentIndex;
use crate::sessions::index::{AgentSessionIndex, IndexedSession, SessionPorts};
use crate::sessions::live::LiveIndexService;
use crate::sessions::search::SearchRequest;
use crate::sessions::{agent_read, read as session_read, scan as scanning, search, usage};
use crate::system::{environment, models, pricing};

/// `tool` 参数取字符串；非字符串等价 Python 的 `self._items[tool]` 落空。
fn tool_name(value: &Value) -> DomainResult<&str> {
    value
        .as_str()
        .ok_or_else(|| DomainError::tool_unknown(&python_str(value)))
}

/// `ref` 参数取字符串；非字符串交给 `is_opaque_session_ref` 统一报错
/// （空串必然不是 opaque ref，与 Python 传 `None` 进去的结果同码同文案）。
fn reference_name(value: &Value) -> &str {
    value.as_str().unwrap_or_default()
}

/// Python 的 `int(...)` 语义：非整数在下游算术里抛 `TypeError`。
fn integer_param(value: &Value, name: &str) -> EngineResult<i64> {
    value.as_i64().ok_or_else(|| EngineError::Internal {
        error_type: "TypeError",
        message: format!("{name} 必须是整数"),
    })
}

/// `from_message == 1`：Python 的数值比较跨 int/float/bool。
fn equals_one(value: &Value) -> bool {
    match value {
        Value::Bool(flag) => *flag,
        other => other.as_f64() == Some(1.0),
    }
}

// ---------------------------------------------------------------------------
// operations ← sessions 的端口桥接
// ---------------------------------------------------------------------------

/// 把 `sessions::index` 的完整记录投影成 operations 的窄记录。
fn project(record: &IndexedSession) -> OperationSession {
    OperationSession {
        tool: record.tool.clone(),
        opaque_ref: record.opaque_ref.clone(),
        canonical_ref: record.canonical_ref.clone(),
        revision: record.revision.clone(),
        row: record.row.clone(),
    }
}

/// `session_changed` 自愈的重试上限（首次尝试之外的次数）。
const HEAL_ATTEMPTS: u32 = 3;

/// 每次重试前的静默等待。agent 的写入是阵发的，几百毫秒足以落进写入间隙；
/// 定向重扫本身实测约 0.75s，这个间隔不构成额外的用户可感延迟。
const HEAL_BACKOFF: Duration = Duration::from_millis(250);

/// 刷新完成、即将重试 resolve 时的观察点；只在测试里注入。
type HealHook = Arc<dyn Fn(u32) + Send + Sync>;

/// 自愈策略。次数与间隔可注入，测试据此确定性驱动而不真的 sleep。
struct HealPolicy {
    attempts: u32,
    backoff: Duration,
    hook: Option<HealHook>,
}

/// 该错误是否是「上次扫描后会话又被写过」这一**瞬态**失败。
/// 其余 reason（`session_missing` / `unknown_ref` / `tool_mismatch`）重扫也不会
/// 变，必须立即抛出。
fn is_session_changed(error: &DomainError) -> bool {
    error.code == "agent.reference_invalid"
        && error.params().get("reason").and_then(Value::as_str) == Some("session_changed")
}

/// `operations::types::SessionResolver` 的生产实现，背靠 [`AgentSessionIndex`]。
///
/// 操作路径（`operation.plan` 全线）在这里做有界自愈：`pin_content=true` 撞上
/// `session_changed` 时定向重扫该工具后用同一个 ref 重试（ref 按
/// `(tool, canonical)` 签发，刷新不换发）。UI 只读浏览走 `pin_content=false`、
/// Agent 读取路径直接调 `AgentSessionIndex::resolve`，都不受影响。
pub struct IndexResolver {
    index: Arc<AgentSessionIndex>,
    heal: HealPolicy,
}

impl IndexResolver {
    pub fn new(index: Arc<AgentSessionIndex>) -> Self {
        Self {
            index,
            heal: HealPolicy {
                attempts: HEAL_ATTEMPTS,
                backoff: HEAL_BACKOFF,
                hook: None,
            },
        }
    }

    /// 测试专用：把重试次数、间隔与观察点注进来，避免依赖真实 sleep。
    #[cfg(test)]
    fn with_heal_policy(
        index: Arc<AgentSessionIndex>,
        attempts: u32,
        backoff: Duration,
        hook: Option<HealHook>,
    ) -> Self {
        Self {
            index,
            heal: HealPolicy {
                attempts,
                backoff,
                hook,
            },
        }
    }

    /// 把窄记录换回索引里的完整记录（`read_indexed_session` 需要
    /// `root`/`storage_kind`）。ref 已从索引中消失即视为引用失效。
    fn full(&self, record: &OperationSession) -> DomainResult<IndexedSession> {
        self.index
            .record(&record.opaque_ref)
            .ok_or_else(|| DomainError::agent_reference_invalid("ref 不在当前扫描索引中"))
    }
}

impl SessionResolver for IndexResolver {
    fn resolve(&self, tool: &str, reference: &str) -> DomainResult<OperationSession> {
        // Python 的 `AgentSessionIndex.resolve` 默认 `pin_content=True`；
        // operations 全线走的都是这条钉内容的路径。
        let mut attempt = 0;
        loop {
            let failure = match self.index.resolve(tool, reference, true) {
                Ok(record) => return Ok(project(&record)),
                Err(error) => error,
            };
            if attempt >= self.heal.attempts || !is_session_changed(&failure) {
                // fail-closed 语义不变：重试耗尽仍在变，原样抛给调用方。
                return Err(failure);
            }
            attempt += 1;
            if !self.heal.backoff.is_zero() {
                std::thread::sleep(self.heal.backoff);
            }
            // 失败的 pin 分支已把刚算出的真实摘要写回摘要缓存，这里的重扫因此
            // 能拿到一致指纹；重扫本身失败就不再遮盖原始的 session_changed。
            if self.index.refresh_tool(tool).is_err() {
                return Err(failure);
            }
            if let Some(hook) = &self.heal.hook {
                hook(attempt);
            }
        }
    }

    fn resolve_message_locator(
        &self,
        record: &OperationSession,
        locator: &str,
    ) -> DomainResult<ResolvedMessageLocator> {
        let message = self.index.resolve_message_locator_parts(
            &record.opaque_ref,
            &record.tool,
            &record.revision,
            locator,
        )?;
        Ok(ResolvedMessageLocator {
            native_locator: message.native_locator,
            editable: message.editable,
        })
    }

    fn evict(&self, tool: &str, canonical_ref: &str) -> DomainResult<()> {
        self.index.evict(tool, canonical_ref);
        Ok(())
    }

    fn read_indexed_session(
        &self,
        record: &OperationSession,
    ) -> DomainResult<crate::model::Session> {
        agent_read::read_indexed_session(&self.index, &self.full(record)?, true)
    }
}

// ---------------------------------------------------------------------------
// 门面
// ---------------------------------------------------------------------------

/// 能力门面：RPC 分发层之下、各能力包之上的那一层。
pub struct Engine {
    ports: Arc<EngineContext>,
    op_ports: Ports,
    index: Arc<AgentSessionIndex>,
    operations: OperationService,
    content_index: Option<Arc<ContentIndex>>,
    live: Mutex<Option<Arc<LiveIndexService>>>,
}

impl Engine {
    pub fn new(
        ports: Arc<EngineContext>,
        index: Arc<AgentSessionIndex>,
        operations: OperationService,
        content_index: Option<Arc<ContentIndex>>,
    ) -> Self {
        let op_ports: Ports = Arc::clone(&ports) as Ports;
        Self {
            ports,
            op_ports,
            index,
            operations,
            content_index,
            live: Mutex::new(None),
        }
    }

    pub fn index(&self) -> &Arc<AgentSessionIndex> {
        &self.index
    }

    pub fn content_index(&self) -> Option<&Arc<ContentIndex>> {
        self.content_index.as_ref()
    }

    /// 等价 `EngineService.close()`：live → operations → content_index。
    pub fn close(&self) {
        if let Some(live) = self.live_service() {
            live.stop();
        }
        self.operations.shutdown();
        if let Some(content_index) = &self.content_index {
            content_index.close();
        }
    }

    /// serve 模式专用：索引增量经 notifier 推送，并启动源变更轮询。
    pub fn enable_live_updates(&self, notifier: &Notifier) {
        let notifier = notifier.clone();
        self.index.set_on_delta(Some(Arc::new(move |delta: &Value| {
            // emit 只在事件未注册时报错，那是编译期就能排除的缺陷。
            let _ = notifier.emit("sessions.changed", delta.clone());
        })));
        let live = Arc::new(LiveIndexService::new(Arc::clone(&self.index)));
        live.start();
        *self
            .live
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(live);
    }

    /// serve 启动预热：先扫库，再把内容索引的缺口交给后台线程。
    pub fn warm_agent_search(&self) {
        let Some(content_index) = &self.content_index else {
            return;
        };
        let started = std::time::Instant::now();
        crate::server::serve::log_info("内容索引预热开始");
        // 预热失败不能影响 RPC 服务（Python 的 `except Exception: log.exception`）。
        match self.index.refresh() {
            Ok(records) => {
                crate::server::serve::log_info(&format!(
                    "预热扫库完成: {} 条会话 耗时={:.1}s",
                    records.len(),
                    started.elapsed().as_secs_f64()
                ));
                match content_index.sync(&self.index, &records, true) {
                    Ok(_) => crate::server::serve::log_info(&format!(
                        "内容索引预热完成 全程={:.1}s",
                        started.elapsed().as_secs_f64()
                    )),
                    Err(error) => crate::server::serve::log_warning(&format!(
                        "内容索引预热失败: {}",
                        error.message()
                    )),
                }
            }
            Err(error) => {
                crate::server::serve::log_warning(&format!("内容索引预热失败: {}", error.message()))
            }
        }
    }

    /// `daemon.status` 用的内容索引覆盖度：只读快照，不触发扫描也不入队。
    ///
    /// 没扫过库就如实说「还没扫」，而不是伪装成 ready——CLI 的 `scan --wait`
    /// 正是靠这个字段判断要不要继续等。
    pub fn content_index_status(&self) -> Value {
        let Some(content_index) = &self.content_index else {
            return json_status(false, "content_index_unavailable");
        };
        let Some((_tools, records, _generation)) = self.index.snapshot_with_status() else {
            return json_status(false, "not_scanned");
        };
        match content_index.coverage(&records) {
            Ok(status) => Value::Object(status),
            Err(error) => json_status(false, error.code),
        }
    }

    fn live_service(&self) -> Option<Arc<LiveIndexService>> {
        self.live
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// 等价 `_checked_query`：UI 只读浏览走宽松解析（`pin_content=False`）。
    ///
    /// 活跃会话随时被 CLI 追加写入，若像 Agent 路径那样把内容 pin 死，点开正在
    /// 进行的会话会稳定撞上 `agent.reference_invalid`。
    fn checked_query<T>(
        &self,
        tool: &Value,
        reference: &Value,
        query: impl FnOnce(&IndexedSession) -> EngineResult<T>,
    ) -> EngineResult<T> {
        let record = self
            .index
            .resolve(tool_name(tool)?, reference_name(reference), false)?;
        query(&record)
    }

    fn refresh_agent_prompt_ref(&self, tool: &str, session_id: &str) -> Option<String> {
        let records = match self.index.refresh() {
            Ok(records) => records,
            Err(error) => {
                // prompt 结果优先，刷新失败写进报告而不是掀桌。
                crate::server::serve::log_warning(&format!(
                    "Agent prompt 后刷新索引失败: {}",
                    error.message()
                ));
                return None;
            }
        };
        records
            .into_iter()
            .find(|record| {
                record.tool == tool
                    && record.row.get("id").and_then(Value::as_str) == Some(session_id)
            })
            .map(|record| record.opaque_ref)
    }

    fn validate_agent_prompt(
        &self,
        tool: &Value,
        reference: &Value,
        prompt: &Value,
        model: &Value,
        timeout_sec: &Value,
    ) -> DomainResult<()> {
        let known = tool.as_str().is_some_and(|name| {
            !name.is_empty() && self.op_ports.adapters().iter().any(|id| id == name)
        });
        if !known {
            let mut params = Map::new();
            params.insert("field".into(), Value::from("tool"));
            return Err(request_error("agent_prompt tool 无效", params));
        }
        if reference.as_str().is_none_or(str::is_empty) {
            let mut params = Map::new();
            params.insert("field".into(), Value::from("ref"));
            return Err(request_error("agent_prompt ref 无效", params));
        }
        if !prompt
            .as_str()
            .is_some_and(|text| (1..=100_000).contains(&text.chars().count()))
        {
            let mut params = Map::new();
            params.insert("field".into(), Value::from("prompt"));
            return Err(request_error(
                "agent_prompt prompt 长度必须为 1..100000",
                params,
            ));
        }
        if !model.is_null() {
            let ok = model.as_str().is_some_and(|text| {
                (1..=512).contains(&text.chars().count())
                    && !text.chars().any(|character| (character as u32) < 32)
            });
            if !ok {
                let mut params = Map::new();
                params.insert("field".into(), Value::from("model"));
                return Err(request_error(
                    "agent_prompt model 长度必须为 1..512",
                    params,
                ));
            }
        }
        let timeout_ok = !timeout_sec.is_boolean()
            && timeout_sec
                .as_i64()
                .is_some_and(|value| (1..=360).contains(&value));
        if !timeout_ok {
            let mut params = Map::new();
            params.insert("field".into(), Value::from("timeout_sec"));
            return Err(request_error(
                "agent_prompt timeout_sec 必须为 1..360 的整数",
                params,
            ));
        }
        Ok(())
    }
}

fn json_status(ready: bool, reason: &str) -> Value {
    let mut status = Map::new();
    status.insert("ready".into(), Value::Bool(ready));
    status.insert("reason".into(), Value::from(reason));
    Value::Object(status)
}

fn request_error(message: &str, params: Map<String, Value>) -> DomainError {
    DomainError::new(
        "agent.request_invalid",
        "AgentRequestError",
        message,
        params,
    )
}

/// 会话行里的原生 ID；缺失即 `agent.reference_invalid`。
fn native_session_id(record: &IndexedSession) -> DomainResult<&str> {
    record
        .row
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| DomainError::agent_reference_invalid("会话缺少原生 ID"))
}

/// 会话行里的工作目录；缺失回落 `"."`。
fn session_cwd(record: &IndexedSession) -> &str {
    record
        .row
        .get("dir")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or(".")
}

impl EngineService for Engine {
    fn health(&self) -> EngineResult<Value> {
        let mut payload = Map::new();
        payload.insert("status".into(), Value::from("ready"));
        payload.insert("service".into(), Value::from("engine"));
        payload.insert("contract_hash".into(), Value::from(FERRY_CONTRACT_HASH));
        Ok(Value::Object(payload))
    }

    fn version(&self) -> EngineResult<Value> {
        let mut payload = Map::new();
        payload.insert("version".into(), Value::from(self.ports.version()));
        Ok(Value::Object(payload))
    }

    fn scan(&self) -> EngineResult<Value> {
        let live = self.live_service();
        Ok(Value::Object(scanning::scan(&self.index, live.as_deref())?))
    }

    fn scan_progress(&self) -> EngineResult<Value> {
        Ok(Value::Object(scanning::scan_progress()))
    }

    fn environment(&self) -> EngineResult<Value> {
        Ok(Value::Object(environment::inspect(self.ports.registry())))
    }

    fn resume_command(&self, tool: &Value, reference: &Value) -> EngineResult<Value> {
        let adapter = SessionPorts::adapter(self.ports.as_ref(), tool_name(tool)?)?;
        let lifecycle = adapter.require_lifecycle("resume")?;
        self.checked_query(tool, reference, |record| {
            let session_id = native_session_id(record)?;
            let cwd = session_cwd(record);
            Ok(Value::Object(lifecycle.resume_descriptor(session_id, cwd)?))
        })
    }

    fn list_models(&self, tool: &Value) -> EngineResult<Value> {
        let name = tool_name(tool)?;
        let adapter = SessionPorts::adapter(self.ports.as_ref(), name)?;
        let catalog = adapter.require_models()?;
        Ok(models::list_models(name, catalog)?.to_value())
    }

    fn migration_history(&self) -> EngineResult<Value> {
        Ok(Value::Array(history::list_entries(&self.op_ports)?))
    }

    fn delete_migration_history(&self, id: &Value) -> EngineResult<Value> {
        // Python 直接把原值绑进 SQL；非字符串匹配不到任何行。
        Ok(history::delete(id.as_str().unwrap_or_default(), &self.op_ports)?.to_value())
    }

    fn pricing(&self, force: &Value) -> EngineResult<Value> {
        let result = pricing::pricing(python_truthy(force), false);
        let mut payload = Map::new();
        payload.insert("prices".into(), Value::Object(result.prices));
        payload.insert("fetched_at".into(), Value::from(result.fetched_at));
        payload.insert("source".into(), Value::from(result.source));
        payload.insert(
            "sources".into(),
            Value::Array(
                result
                    .sources
                    .iter()
                    .map(pricing::SourceStatus::to_value)
                    .collect(),
            ),
        );
        Ok(Value::Object(payload))
    }

    fn show_session(
        &self,
        tool: &Value,
        reference: &Value,
        from_message: &Value,
        limit: &Value,
    ) -> EngineResult<Value> {
        let name = tool_name(tool)?;
        let browser = SessionPorts::adapter(self.ports.as_ref(), name)?.require_browser()?;
        self.checked_query(tool, reference, |record| {
            // 浏览路径与 Agent 读取共用同一 locator 命名空间：详情里的
            // turn_locator 与 Agent preview 候选、operations 编辑通道对得上。
            let issuer = agent_read::browser_locator_issuer(&self.index, record);
            let session = browser.read_browser(&record.canonical_ref)?;
            let options = if equals_one(from_message) && limit.is_null() {
                session_read::SessionJsonOptions {
                    tree_count: record.row.get("tree_count").and_then(Value::as_i64),
                    child_count: record.row.get("child_count").and_then(Value::as_i64),
                    total_count: record.row.get("count").and_then(Value::as_i64),
                    ..session_read::show_options()
                }
            } else {
                session_read::SessionJsonOptions {
                    from_message: integer_param(from_message, "from_message")?,
                    message_limit: match limit {
                        Value::Null => None,
                        other => Some(integer_param(other, "limit")?),
                    },
                    include_messages: false,
                    include_tree: false,
                    tree_count: record.row.get("tree_count").and_then(Value::as_i64),
                    child_count: record.row.get("child_count").and_then(Value::as_i64),
                    total_count: record.row.get("count").and_then(Value::as_i64),
                }
            };
            Ok(Value::Object(session_read::show(
                &session,
                options,
                Some(&issuer),
            )?))
        })
    }

    fn session_asset(
        &self,
        tool: &Value,
        reference: &Value,
        asset_id: &Value,
    ) -> EngineResult<Value> {
        let name = tool_name(tool)?;
        SessionPorts::adapter(self.ports.as_ref(), name)?.require_browser()?;
        self.checked_query(tool, reference, |record| {
            let session =
                session_read::read_tree(name, &record.canonical_ref, self.ports.as_ref())?;
            Ok(Value::Object(session_read::session_asset(
                &session,
                asset_id.as_str().unwrap_or_default(),
            )?))
        })
    }

    fn list_session_metadata(&self) -> EngineResult<Value> {
        Ok(Value::Object(metadata::list_all(&self.op_ports)?))
    }

    fn search_sessions_for_ui(
        &self,
        query: &Value,
        tools: &Value,
        limit: &Value,
        scope: &Value,
    ) -> EngineResult<Value> {
        Ok(Value::Object(search::search_sessions_for_ui(
            Some(query),
            Some(tools),
            Some(limit),
            Some(scope),
            &self.index,
            self.content_index.as_ref(),
        )?))
    }

    fn load_runtime_sessions(&self) -> EngineResult<Value> {
        Ok(Value::Array(runtime_sessions::load_all(
            self.op_ports.state_dir(),
        )?))
    }

    fn commit_runtime_session(&self, update: &Value) -> EngineResult<Value> {
        runtime_sessions::commit(update, self.op_ports.state_dir())
    }

    fn delete_runtime_session(&self, session_id: &Value) -> EngineResult<Value> {
        runtime_sessions::delete(session_id, self.op_ports.state_dir())
    }

    fn truncate_runtime_session(
        &self,
        session_id: &Value,
        from_ordinal: &Value,
        from_seq: &Value,
    ) -> EngineResult<Value> {
        runtime_sessions::truncate(
            session_id,
            from_ordinal,
            from_seq,
            self.op_ports.state_dir(),
        )
    }

    fn content_search(&self, request: &ContentSearchRequest) -> EngineResult<Value> {
        Ok(Value::Object(search::search_sessions(
            &SearchRequest {
                query: Some(&request.query),
                agents: Some(&request.agents),
                projects: Some(&request.projects),
                session_ids: Some(&request.session_ids),
                time_range: Some(&request.time_range),
                limit: Some(&request.limit),
                scope: Some(&request.scope),
                include_tool_outputs: Some(&request.include_tool_outputs),
                patterns: Some(&request.patterns),
                regex: Some(&request.regex),
                exhaustive: Some(&request.exhaustive),
            },
            &self.index,
            self.content_index.as_ref(),
        )?))
    }

    fn session_read(&self, request: &SessionReadRequest) -> EngineResult<Value> {
        let name = tool_name(&request.tool)?;
        SessionPorts::adapter(self.ports.as_ref(), name)?.require_browser()?;
        Ok(Value::Object(agent_read::session_read(
            name,
            request.reference.as_str(),
            Some(&request.terms),
            Some(&request.roles),
            Some(&request.from_message),
            Some(&request.limit),
            Some(&request.include_tool_outputs),
            Some(&request.max_bytes),
            Some(&request.inert),
            &self.index,
        )?))
    }

    fn usage_stats(
        &self,
        agents: &Value,
        projects: &Value,
        time_range: &Value,
    ) -> EngineResult<Value> {
        Ok(Value::Object(usage::get_usage(
            Some(agents),
            Some(projects),
            Some(time_range),
            &self.index,
        )?))
    }

    fn agent_prompt(
        &self,
        tool: &Value,
        reference: &Value,
        prompt: &Value,
        model: &Value,
        timeout_sec: &Value,
    ) -> EngineResult<Value> {
        self.validate_agent_prompt(tool, reference, prompt, model, timeout_sec)?;
        let name = tool_name(tool)?;
        let record = self.index.resolve(name, reference_name(reference), true)?;
        let session_id = native_session_id(&record)?.to_string();
        let cwd = session_cwd(&record).to_string();
        let model_text = model.as_str();

        let outcome = verification::run_agent_prompt(
            name,
            &session_id,
            prompt.as_str().unwrap_or_default(),
            Some(&cwd),
            model_text,
            timeout_sec.as_i64().unwrap_or(360) as u64,
            &self.op_ports,
        );
        let mut report = match outcome {
            Ok(report) => report,
            Err(error) => {
                // 失败也要把 ref 刷新掉：会话很可能已经被 agent 写过了。
                self.refresh_agent_prompt_ref(name, &session_id);
                return Err(error);
            }
        };
        let next_ref = self.refresh_agent_prompt_ref(name, &session_id);

        if !report.contains_key("params") {
            report.insert("params".into(), Value::Object(Map::new()));
        }
        let Some(params) = report.get_mut("params").and_then(Value::as_object_mut) else {
            return Err(EngineError::runtime(
                "Agent prompt report params 必须是 object",
            ));
        };
        if !params.contains_key("tool") {
            params.insert("tool".into(), Value::from(name));
        }
        params.insert("session_id".into(), Value::from(session_id.as_str()));
        if let Some(model_text) = model_text {
            if !params.contains_key("model") {
                params.insert("model".into(), Value::from(model_text));
            }
        }
        match next_ref {
            Some(next_ref) => {
                report.insert("next_ref".into(), Value::from(next_ref));
            }
            None => {
                if let Some(params) = report.get_mut("params").and_then(Value::as_object_mut) {
                    params.insert("ref_refresh_failed".into(), Value::Bool(true));
                }
            }
        }
        Ok(Value::Object(report))
    }

    fn operation_plan(&self, input: &Value) -> EngineResult<Value> {
        self.operations.plan(input)
    }

    fn operation_apply(&self, plan_id: &Value) -> EngineResult<Value> {
        self.operations.apply(plan_id)
    }

    fn operation_status(&self, plan_id: &Value) -> EngineResult<Value> {
        self.operations.status(plan_id)
    }

    fn operation_cancel(&self, plan_id: &Value) -> EngineResult<Value> {
        self.operations.cancel(plan_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn from_message_one_covers_int_float_and_true() {
        assert!(equals_one(&json!(1)));
        assert!(equals_one(&json!(1.0)));
        assert!(equals_one(&json!(true)));
        assert!(!equals_one(&json!("1")));
        assert!(!equals_one(&json!(2)));
    }

    #[test]
    fn non_string_tool_reports_tool_unknown_with_python_text() {
        let error = tool_name(&json!(null)).unwrap_err();
        assert_eq!(error.code, "tool.unknown");
        assert_eq!(error.message(), "未知工具: None");
    }

    // -----------------------------------------------------------------------
    // §9.1 操作路径解析的 session_changed 自愈
    // -----------------------------------------------------------------------

    use crate::adapters::contracts::StorageKind;
    use crate::sessions::index::golden_tests::{harness, Harness};
    use std::sync::atomic::{AtomicU32, Ordering};

    /// 首扫后取一个文件型会话；它的 `canonical_ref` 是临时目录里的真实文件。
    fn file_session(harness: &Harness) -> IndexedSession {
        harness
            .index
            .refresh()
            .expect("首扫成功")
            .into_iter()
            .find(|record| record.storage_kind == StorageKind::File)
            .expect("有文件型会话")
    }

    /// 模拟 agent 追加写入：JSONL 会话的合法增量就是多一行。
    fn append_line(path: &str, line: &str) {
        use std::io::Write as _;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(path)
            .expect("会话文件可追加");
        writeln!(file, "{line}").expect("追加成功");
    }

    /// 计数用的观察点：返回 (hook, 计数器)。
    fn counting_hook() -> (HealHook, Arc<AtomicU32>) {
        let counter = Arc::new(AtomicU32::new(0));
        let sink = Arc::clone(&counter);
        let hook: HealHook = Arc::new(move |_attempt| {
            sink.fetch_add(1, Ordering::SeqCst);
        });
        (hook, counter)
    }

    #[test]
    fn session_changed_heals_by_rescanning_and_retrying_the_same_ref() {
        let harness = harness();
        let target = file_session(&harness);
        // 扫描之后 agent 又写了一笔：不自愈的话这里就是用户看到的
        // agent.reference_invalid。
        append_line(
            &target.canonical_ref,
            r#"{"type":"user","content":"扫描后追加"}"#,
        );

        let (hook, retries) = counting_hook();
        let resolver = IndexResolver::with_heal_policy(
            Arc::clone(&harness.index),
            HEAL_ATTEMPTS,
            Duration::ZERO,
            Some(hook),
        );
        let healed = resolver
            .resolve(&target.tool, &target.opaque_ref)
            .expect("自愈后解析成功");

        // ref 稳定、revision 跟到最新内容，且只用了一轮重试。
        assert_eq!(healed.opaque_ref, target.opaque_ref);
        assert_ne!(healed.revision, target.revision);
        assert_eq!(retries.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn non_transient_reasons_fail_immediately_without_a_rescan() {
        let harness = harness();
        let target = file_session(&harness);
        std::fs::remove_file(&target.canonical_ref).expect("删除会话文件");

        let (hook, retries) = counting_hook();
        let resolver = IndexResolver::with_heal_policy(
            Arc::clone(&harness.index),
            HEAL_ATTEMPTS,
            Duration::ZERO,
            Some(hook),
        );
        let error = resolver
            .resolve(&target.tool, &target.opaque_ref)
            .unwrap_err();
        assert_eq!(error.code, "agent.reference_invalid");
        assert_eq!(error.params()["reason"], Value::from("session_missing"));
        assert_eq!(retries.load(Ordering::SeqCst), 0);

        // tool 配错同样是终态。
        let error = resolver.resolve("nope", &target.opaque_ref).unwrap_err();
        assert_eq!(error.params()["reason"], Value::from("tool_mismatch"));
        assert_eq!(retries.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_session_that_keeps_changing_exhausts_the_retries_and_reports_session_changed() {
        let harness = harness();
        let target = file_session(&harness);
        append_line(&target.canonical_ref, r#"{"seq":0}"#);

        // 观察点在重扫之后、重试 resolve 之前触发：每轮都让文件再变一次，
        // 等价于一个从不安静的写入者，但完全确定性。
        let counter = Arc::new(AtomicU32::new(0));
        let sink = Arc::clone(&counter);
        let path = target.canonical_ref.clone();
        let hook: HealHook = Arc::new(move |attempt| {
            sink.fetch_add(1, Ordering::SeqCst);
            append_line(&path, &format!(r#"{{"seq":{attempt}}}"#));
        });
        let resolver = IndexResolver::with_heal_policy(
            Arc::clone(&harness.index),
            HEAL_ATTEMPTS,
            Duration::ZERO,
            Some(hook),
        );

        let error = resolver
            .resolve(&target.tool, &target.opaque_ref)
            .unwrap_err();
        assert_eq!(error.code, "agent.reference_invalid");
        assert_eq!(error.params()["reason"], Value::from("session_changed"));
        assert_eq!(error.message(), "ref 在扫描后已变化，请重新搜索");
        assert_eq!(counter.load(Ordering::SeqCst), HEAL_ATTEMPTS);
    }
}
