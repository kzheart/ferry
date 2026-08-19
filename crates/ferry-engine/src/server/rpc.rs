//! `ferry-ipc/1` RPC 契约与调度：结构化错误 envelope。
//!
//! 硬约束（§2.1）：
//! 1. 请求信封字段集合**精确等于** `{protocol,id,method,params}`，多一个即
//!    `rpc.invalid_request`；`id` ≤128；`request_id` 提取先于 protocol 校验，
//!    取不到回落 `"unknown"`；
//! 2. 任何响应（含错误）必须带 string 型 `id`；
//! 4. 错误信封 `{code, params(+message=str(err)[:500] setdefault), category,
//!    retryable}`；`probe.timeout` 是幽灵码；未捕获异常一律 `internal.unexpected`
//!    且不泄漏异常文本（除非 `FERRY_DEBUG`）；
//! 5. 分发层默认参数是 wire 语义的一部分；
//! 6. 只有「分发层直接取参」的缺键才算 `rpc.missing_param`，深层缺键是内部缺陷；
//! 9. 启动自检：方法表 == 生成契约的方法集合，不一致直接失败。

use std::sync::Arc;
use std::time::Instant;

use serde_json::{Map, Value};

use crate::contracts::engine_methods::ENGINE_METHOD_NAMES;
use crate::contracts::ipc::FERRY_IPC_PROTOCOL;
use crate::errors::DomainError;
use crate::operations::types::{EngineError, EngineResult};
use crate::server::serve::{log_info, log_warning};

pub const PROTOCOL: &str = FERRY_IPC_PROTOCOL;

/// `agent_session_read` 的 `max_bytes` 默认值。
///
/// 分发层在这里兜底，`sessions::agent_read` 自己也认这个上限。
pub const DEFAULT_CONTEXT_BYTES: i64 = 24 * 1024;

/// `agent_prompt` 的默认超时（秒）。
pub const DEFAULT_AGENT_PROMPT_TIMEOUT_SEC: i64 = 360;

/// `agent_search_sessions` 的分发层参数包。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AgentSearchRequest {
    pub query: Value,
    pub agents: Value,
    pub projects: Value,
    pub time_range: Value,
    pub limit: Value,
    pub scope: Value,
    pub include_tool_outputs: Value,
    pub patterns: Value,
    pub regex: Value,
    pub exhaustive: Value,
}

/// `agent_session_read` 的分发层参数包。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AgentSessionReadRequest {
    pub tool: Value,
    pub reference: Value,
    pub terms: Value,
    pub roles: Value,
    pub from_message: Value,
    pub limit: Value,
    pub include_tool_outputs: Value,
    pub max_bytes: Value,
}

/// Engine 能力门面。等价 Python 的 `EngineService`。
///
/// 参数一律是 `&Value`：Python 分发层不做类型收窄，类型校验属于下游能力包，
/// 提前收窄会把「参数类型错误」误报成 `rpc.missing_param`。
pub trait EngineService: Send + Sync {
    fn health(&self) -> EngineResult<Value>;
    fn version(&self) -> EngineResult<Value>;
    fn scan(&self) -> EngineResult<Value>;
    fn scan_progress(&self) -> EngineResult<Value>;
    fn environment(&self) -> EngineResult<Value>;
    fn resume_command(&self, tool: &Value, reference: &Value) -> EngineResult<Value>;
    fn list_models(&self, tool: &Value) -> EngineResult<Value>;
    fn migration_history(&self) -> EngineResult<Value>;
    fn delete_migration_history(&self, id: &Value) -> EngineResult<Value>;
    fn pricing(&self, force: &Value) -> EngineResult<Value>;
    fn show_session(
        &self,
        tool: &Value,
        reference: &Value,
        from_message: &Value,
        limit: &Value,
    ) -> EngineResult<Value>;
    fn session_asset(
        &self,
        tool: &Value,
        reference: &Value,
        asset_id: &Value,
    ) -> EngineResult<Value>;
    fn list_session_metadata(&self) -> EngineResult<Value>;
    fn search_sessions_for_ui(
        &self,
        query: &Value,
        tools: &Value,
        limit: &Value,
        scope: &Value,
    ) -> EngineResult<Value>;
    fn load_runtime_sessions(&self) -> EngineResult<Value>;
    fn commit_runtime_session(&self, update: &Value) -> EngineResult<Value>;
    fn delete_runtime_session(&self, session_id: &Value) -> EngineResult<Value>;
    fn truncate_runtime_session(
        &self,
        session_id: &Value,
        from_ordinal: &Value,
        from_seq: &Value,
    ) -> EngineResult<Value>;
    fn agent_search_sessions(&self, request: &AgentSearchRequest) -> EngineResult<Value>;
    fn agent_session_read(&self, request: &AgentSessionReadRequest) -> EngineResult<Value>;
    fn agent_get_usage(
        &self,
        agents: &Value,
        projects: &Value,
        time_range: &Value,
    ) -> EngineResult<Value>;
    fn agent_prompt(
        &self,
        tool: &Value,
        reference: &Value,
        prompt: &Value,
        model: &Value,
        timeout_sec: &Value,
    ) -> EngineResult<Value>;
    fn operation_plan(&self, input: &Value) -> EngineResult<Value>;
    fn operation_apply(&self, plan_id: &Value) -> EngineResult<Value>;
    fn operation_status(&self, plan_id: &Value) -> EngineResult<Value>;
    fn operation_cancel(&self, plan_id: &Value) -> EngineResult<Value>;
}

/// 分发表覆盖的方法名，顺序与 `ENGINE_METHOD_NAMES` 一致。
const DISPATCH_METHOD_NAMES: &[&str] = &[
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
    "agent_search_sessions",
    "agent_session_read",
    "agent_get_usage",
    "agent_prompt",
    "operation.plan",
    "operation.apply",
    "operation.status",
    "operation.cancel",
];

/// 绑定一个 `EngineService` 的 RPC 调度器。
pub struct RpcDispatcher {
    service: Arc<dyn EngineService>,
}

impl RpcDispatcher {
    /// 启动自检：分发表必须与生成契约的方法集合逐字相等。
    pub fn new(service: Arc<dyn EngineService>) -> Result<Self, String> {
        let mut declared: Vec<&str> = DISPATCH_METHOD_NAMES.to_vec();
        let mut contract: Vec<&str> = ENGINE_METHOD_NAMES.to_vec();
        declared.sort_unstable();
        contract.sort_unstable();
        if declared != contract {
            return Err("Engine RPC 与生成方法契约不一致".to_string());
        }
        Ok(Self { service })
    }

    pub fn handle(&self, request: &str) -> Value {
        let started = Instant::now();
        let response = self.handle_inner(request);
        let elapsed = started.elapsed();
        // 高频轮询（scan_progress 等）不刷屏，只记慢请求；串行池被谁占住靠它还原。
        if elapsed.as_secs_f64() >= 1.0 {
            let method = serde_json::from_str::<Value>(request)
                .ok()
                .and_then(|value| {
                    value
                        .get("method")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_else(|| "None".to_string());
            log_info(&format!(
                "RPC {method} 耗时={:.1}s ok={}",
                elapsed.as_secs_f64(),
                response.get("ok").and_then(Value::as_bool).unwrap_or(false)
            ));
        }
        response
    }

    fn handle_inner(&self, request: &str) -> Value {
        let mut request_id = "unknown".to_string();
        match self.route(request, &mut request_id) {
            Ok(result) => {
                let mut envelope = Map::new();
                envelope.insert("protocol".into(), Value::from(PROTOCOL));
                envelope.insert("id".into(), Value::from(request_id));
                envelope.insert("ok".into(), Value::Bool(true));
                envelope.insert("result".into(), result);
                Value::Object(envelope)
            }
            Err(EngineError::Domain(error)) => {
                log_warning(&format!("RPC domain error: {}", error.message()));
                error_envelope(&error, &request_id)
            }
            Err(EngineError::Internal {
                error_type,
                message,
            }) => {
                // 生产 RPC 不暴露任意异常文本；完整异常链只进日志。
                log_warning(&format!("RPC internal error: {error_type}: {message}"));
                let internal = if std::env::var_os("FERRY_DEBUG").is_some() {
                    DomainError::internal(format!("{error_type}: {message}"))
                } else {
                    DomainError::internal("internal")
                };
                error_envelope(&internal, &request_id)
            }
        }
    }

    fn route(&self, request: &str, request_id: &mut String) -> EngineResult<Value> {
        let value: Value = serde_json::from_str(request)
            .map_err(|error| DomainError::invalid_json(error.to_string()))?;
        let Some(object) = value.as_object() else {
            return Err(DomainError::invalid_json("请求必须是 JSON object").into());
        };
        // request_id 的提取先于 protocol 校验：协议不匹配的响应也要能对上号。
        if let Some(id) = object.get("id").and_then(Value::as_str) {
            if !id.is_empty() {
                *request_id = id.to_string();
            }
        }
        if object.get("protocol") != Some(&Value::from(PROTOCOL)) {
            return Err(DomainError::unsupported_protocol(
                PROTOCOL,
                object.get("protocol").cloned().unwrap_or(Value::Null),
            )
            .into());
        }
        let expected = ["protocol", "id", "method", "params"];
        if object.len() != expected.len() || !expected.iter().all(|key| object.contains_key(*key)) {
            return Err(DomainError::invalid_request("请求 envelope 字段不匹配").into());
        }
        if request_id.chars().count() > 128 {
            return Err(DomainError::invalid_request("请求 id 超出长度限制").into());
        }
        let Some(params) = object["params"].as_object() else {
            return Err(DomainError::invalid_request("params 必须是 JSON object").into());
        };
        let method = object["method"]
            .as_str()
            .filter(|method| !method.is_empty())
            .ok_or_else(|| DomainError::invalid_request("method 必须是非空字符串"))?;
        if !DISPATCH_METHOD_NAMES.contains(&method) {
            return Err(DomainError::unknown_method(method).into());
        }
        self.dispatch(method, params)
    }

    fn dispatch(&self, method: &str, params: &Map<String, Value>) -> EngineResult<Value> {
        let service = self.service.as_ref();
        match method {
            "health" => service.health(),
            "version" => service.version(),
            "scan" => service.scan(),
            "scan_progress" => service.scan_progress(),
            "env" => service.environment(),
            "resume" => service.resume_command(required(params, "tool")?, required(params, "ref")?),
            "models" => service.list_models(required(params, "tool")?),
            "history" => service.migration_history(),
            "history_delete" => service.delete_migration_history(required(params, "id")?),
            "pricing" => service.pricing(&default_of(params, "force", Value::Bool(false))),
            "show" => service.show_session(
                required(params, "tool")?,
                required(params, "ref")?,
                &default_of(params, "from_message", Value::from(1)),
                optional(params, "limit"),
            ),
            "session_asset" => service.session_asset(
                required(params, "tool")?,
                required(params, "ref")?,
                required(params, "asset_id")?,
            ),
            "session_meta_list" => service.list_session_metadata(),
            "session_search" => service.search_sessions_for_ui(
                required(params, "query")?,
                optional(params, "tools"),
                optional(params, "limit"),
                &default_of(params, "scope", Value::from("any")),
            ),
            "runtime_sessions.load_all" => service.load_runtime_sessions(),
            "runtime_sessions.commit" => {
                service.commit_runtime_session(required(params, "update")?)
            }
            "runtime_sessions.delete" => {
                service.delete_runtime_session(required(params, "session_id")?)
            }
            "runtime_sessions.truncate" => service.truncate_runtime_session(
                required(params, "session_id")?,
                required(params, "from_ordinal")?,
                required(params, "from_seq")?,
            ),
            "agent_search_sessions" => service.agent_search_sessions(&AgentSearchRequest {
                query: default_of(params, "query", Value::from("")),
                agents: optional(params, "agents").clone(),
                projects: optional(params, "projects").clone(),
                time_range: optional(params, "time_range").clone(),
                limit: default_of(params, "limit", Value::from(20)),
                scope: default_of(params, "scope", Value::from("any")),
                include_tool_outputs: default_of(
                    params,
                    "include_tool_outputs",
                    Value::Bool(false),
                ),
                patterns: optional(params, "patterns").clone(),
                regex: optional(params, "regex").clone(),
                exhaustive: default_of(params, "exhaustive", Value::Bool(false)),
            }),
            "agent_session_read" => service.agent_session_read(&AgentSessionReadRequest {
                tool: required(params, "tool")?.clone(),
                reference: required(params, "ref")?.clone(),
                terms: optional(params, "terms").clone(),
                roles: optional(params, "roles").clone(),
                from_message: default_of(params, "from_message", Value::from(1)),
                limit: default_of(params, "limit", Value::from(20)),
                include_tool_outputs: default_of(
                    params,
                    "include_tool_outputs",
                    Value::Bool(false),
                ),
                max_bytes: default_of(params, "max_bytes", Value::from(DEFAULT_CONTEXT_BYTES)),
            }),
            "agent_get_usage" => service.agent_get_usage(
                optional(params, "agents"),
                optional(params, "projects"),
                optional(params, "time_range"),
            ),
            "agent_prompt" => service.agent_prompt(
                required(params, "tool")?,
                required(params, "ref")?,
                required(params, "prompt")?,
                optional(params, "model"),
                &default_of(
                    params,
                    "timeout_sec",
                    Value::from(DEFAULT_AGENT_PROMPT_TIMEOUT_SEC),
                ),
            ),
            "operation.plan" => service.operation_plan(required(params, "input")?),
            "operation.apply" => service.operation_apply(required(params, "plan_id")?),
            "operation.status" => service.operation_status(required(params, "plan_id")?),
            "operation.cancel" => service.operation_cancel(required(params, "plan_id")?),
            // route() 已经拿 DISPATCH_METHOD_NAMES 过滤过。
            other => Err(DomainError::unknown_method(other).into()),
        }
    }
}

/// 分发层直接取参：缺键即 `rpc.missing_param`（这是**唯一**产出该码的地方）。
fn required<'a>(params: &'a Map<String, Value>, key: &str) -> EngineResult<&'a Value> {
    params
        .get(key)
        .ok_or_else(|| DomainError::missing_param(key).into())
}

/// `p.get(key)`：缺键即 JSON null，与 Python 的 `None` 等价。
fn optional<'a>(params: &'a Map<String, Value>, key: &str) -> &'a Value {
    params.get(key).unwrap_or(&Value::Null)
}

/// `p.get(key, default)`：键存在就用它（哪怕是 null），否则用默认值。
fn default_of(params: &Map<String, Value>, key: &str, default: Value) -> Value {
    params.get(key).cloned().unwrap_or(default)
}

/// `{code, params(+message), category, retryable}`。
///
/// `message` 是 agent 自救指引的一部分：光有 code 模型只能盲猜，
/// 与 runtime `ProtocolError.toEnvelope` 一致，借 `params.message` 下发。
pub fn error_envelope(error: &DomainError, request_id: &str) -> Value {
    let mut params = error.params().clone();
    if !params.contains_key("message") {
        params.insert(
            "message".into(),
            Value::from(error.message().chars().take(500).collect::<String>()),
        );
    }
    let mut payload = Map::new();
    payload.insert("code".into(), Value::from(error.code));
    payload.insert("params".into(), Value::Object(params));
    payload.insert("category".into(), Value::from(error.category));
    payload.insert("retryable".into(), Value::Bool(error.retryable));

    let mut envelope = Map::new();
    envelope.insert("protocol".into(), Value::from(PROTOCOL));
    envelope.insert("id".into(), Value::from(request_id));
    envelope.insert("ok".into(), Value::Bool(false));
    envelope.insert("error".into(), Value::Object(payload));
    Value::Object(envelope)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    /// 记录调用的假 EngineService；未被断言的方法一律回 null。
    #[derive(Default)]
    struct Recorder {
        calls: Mutex<Vec<(String, Value)>>,
        health: Value,
        failure: Option<EngineError>,
    }

    impl Recorder {
        fn record(&self, name: &str, payload: Value) -> EngineResult<Value> {
            self.calls
                .lock()
                .unwrap()
                .push((name.to_string(), payload.clone()));
            match &self.failure {
                Some(error) => Err(error.clone()),
                None => Ok(payload),
            }
        }

        fn last(&self) -> (String, Value) {
            self.calls.lock().unwrap().last().cloned().unwrap()
        }
    }

    macro_rules! recorded {
        ($name:literal, $($key:literal => $value:expr),* $(,)?) => {{
            let mut payload = Map::new();
            $(payload.insert($key.into(), $value.clone());)*
            Value::Object(payload)
        }};
    }

    impl EngineService for Recorder {
        fn health(&self) -> EngineResult<Value> {
            self.calls
                .lock()
                .unwrap()
                .push(("health".into(), Value::Null));
            match &self.failure {
                Some(error) => Err(error.clone()),
                None => Ok(self.health.clone()),
            }
        }
        fn version(&self) -> EngineResult<Value> {
            self.record("version", Value::Null)
        }
        fn scan(&self) -> EngineResult<Value> {
            self.record("scan", Value::Null)
        }
        fn scan_progress(&self) -> EngineResult<Value> {
            self.record("scan_progress", Value::Null)
        }
        fn environment(&self) -> EngineResult<Value> {
            self.record("env", Value::Null)
        }
        fn resume_command(&self, tool: &Value, reference: &Value) -> EngineResult<Value> {
            self.record(
                "resume",
                recorded!("resume", "tool" => tool, "ref" => reference),
            )
        }
        fn list_models(&self, tool: &Value) -> EngineResult<Value> {
            self.record("models", recorded!("models", "tool" => tool))
        }
        fn migration_history(&self) -> EngineResult<Value> {
            self.record("history", Value::Null)
        }
        fn delete_migration_history(&self, id: &Value) -> EngineResult<Value> {
            self.record("history_delete", recorded!("h", "id" => id))
        }
        fn pricing(&self, force: &Value) -> EngineResult<Value> {
            self.record("pricing", recorded!("p", "force" => force))
        }
        fn show_session(
            &self,
            tool: &Value,
            reference: &Value,
            from_message: &Value,
            limit: &Value,
        ) -> EngineResult<Value> {
            self.record(
                "show",
                recorded!("s", "tool" => tool, "ref" => reference,
                          "from_message" => from_message, "limit" => limit),
            )
        }
        fn session_asset(
            &self,
            tool: &Value,
            reference: &Value,
            asset_id: &Value,
        ) -> EngineResult<Value> {
            self.record(
                "session_asset",
                recorded!("a", "tool" => tool, "ref" => reference, "asset_id" => asset_id),
            )
        }
        fn list_session_metadata(&self) -> EngineResult<Value> {
            self.record("session_meta_list", Value::Null)
        }
        fn search_sessions_for_ui(
            &self,
            query: &Value,
            tools: &Value,
            limit: &Value,
            scope: &Value,
        ) -> EngineResult<Value> {
            self.record(
                "session_search",
                recorded!("q", "query" => query, "tools" => tools,
                          "limit" => limit, "scope" => scope),
            )
        }
        fn load_runtime_sessions(&self) -> EngineResult<Value> {
            self.record("runtime_sessions.load_all", Value::Null)
        }
        fn commit_runtime_session(&self, update: &Value) -> EngineResult<Value> {
            self.record(
                "runtime_sessions.commit",
                recorded!("c", "update" => update),
            )
        }
        fn delete_runtime_session(&self, session_id: &Value) -> EngineResult<Value> {
            self.record(
                "runtime_sessions.delete",
                recorded!("d", "session_id" => session_id),
            )
        }
        fn truncate_runtime_session(
            &self,
            session_id: &Value,
            from_ordinal: &Value,
            from_seq: &Value,
        ) -> EngineResult<Value> {
            self.record(
                "runtime_sessions.truncate",
                recorded!("t", "session_id" => session_id,
                          "from_ordinal" => from_ordinal, "from_seq" => from_seq),
            )
        }
        fn agent_search_sessions(&self, request: &AgentSearchRequest) -> EngineResult<Value> {
            self.record(
                "agent_search_sessions",
                recorded!("s",
                    "query" => request.query, "agents" => request.agents,
                    "projects" => request.projects, "time_range" => request.time_range,
                    "limit" => request.limit, "scope" => request.scope,
                    "include_tool_outputs" => request.include_tool_outputs,
                    "patterns" => request.patterns, "regex" => request.regex,
                    "exhaustive" => request.exhaustive),
            )
        }
        fn agent_session_read(&self, request: &AgentSessionReadRequest) -> EngineResult<Value> {
            self.record(
                "agent_session_read",
                recorded!("r",
                    "tool" => request.tool, "ref" => request.reference,
                    "terms" => request.terms, "roles" => request.roles,
                    "from_message" => request.from_message, "limit" => request.limit,
                    "include_tool_outputs" => request.include_tool_outputs,
                    "max_bytes" => request.max_bytes),
            )
        }
        fn agent_get_usage(
            &self,
            agents: &Value,
            projects: &Value,
            time_range: &Value,
        ) -> EngineResult<Value> {
            self.record(
                "agent_get_usage",
                recorded!("u", "agents" => agents, "projects" => projects,
                          "time_range" => time_range),
            )
        }
        fn agent_prompt(
            &self,
            tool: &Value,
            reference: &Value,
            prompt: &Value,
            model: &Value,
            timeout_sec: &Value,
        ) -> EngineResult<Value> {
            self.record(
                "agent_prompt",
                recorded!("p", "tool" => tool, "ref" => reference, "prompt" => prompt,
                          "model" => model, "timeout_sec" => timeout_sec),
            )
        }
        fn operation_plan(&self, input: &Value) -> EngineResult<Value> {
            self.record("operation.plan", recorded!("o", "input" => input))
        }
        fn operation_apply(&self, plan_id: &Value) -> EngineResult<Value> {
            self.record("operation.apply", recorded!("o", "plan_id" => plan_id))
        }
        fn operation_status(&self, plan_id: &Value) -> EngineResult<Value> {
            self.record("operation.status", recorded!("o", "plan_id" => plan_id))
        }
        fn operation_cancel(&self, plan_id: &Value) -> EngineResult<Value> {
            self.record("operation.cancel", recorded!("o", "plan_id" => plan_id))
        }
    }

    fn dispatcher(service: Arc<Recorder>) -> RpcDispatcher {
        RpcDispatcher::new(service).unwrap()
    }

    fn call(dispatcher: &RpcDispatcher, method: &str, params: Value, id: &str) -> Value {
        dispatcher.handle(
            &json!({"protocol": PROTOCOL, "id": id, "method": method, "params": params})
                .to_string(),
        )
    }

    #[test]
    fn dispatch_table_matches_the_generated_contract() {
        assert_eq!(DISPATCH_METHOD_NAMES, ENGINE_METHOD_NAMES);
    }

    #[test]
    fn success_envelope_carries_protocol_and_id() {
        let service = Arc::new(Recorder {
            health: json!({"status": "ready", "service": "engine"}),
            ..Recorder::default()
        });
        let response = call(&dispatcher(service), "health", json!({}), "req-1");
        assert_eq!(response["protocol"], json!("ferry-ipc/1"));
        assert_eq!(response["ok"], json!(true));
        assert_eq!(response["id"], json!("req-1"));
        assert_eq!(response["result"]["status"], json!("ready"));
        // 成功信封只有这四个字段。
        let keys: Vec<&String> = response.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["protocol", "id", "ok", "result"]);
    }

    #[test]
    fn invalid_json_is_structured_and_falls_back_to_unknown() {
        let response = dispatcher(Arc::new(Recorder::default())).handle("{not json");
        assert_eq!(response["ok"], json!(false));
        assert_eq!(response["protocol"], json!("ferry-ipc/1"));
        assert_eq!(response["id"], json!("unknown"));
        assert_eq!(response["error"]["code"], json!("rpc.invalid_json"));
        assert_eq!(response["error"]["category"], json!("validation"));

        let not_object = dispatcher(Arc::new(Recorder::default())).handle("[]");
        assert_eq!(not_object["error"]["code"], json!("rpc.invalid_json"));
        assert_eq!(
            not_object["error"]["params"]["message"],
            json!("请求必须是 JSON object")
        );
    }

    #[test]
    fn old_or_extended_envelopes_are_rejected() {
        let engine = dispatcher(Arc::new(Recorder::default()));
        let old = engine.handle(
            &json!({"protocol": 2, "id": "old", "method": "health", "params": {}}).to_string(),
        );
        assert_eq!(old["id"], json!("old"));
        assert_eq!(old["error"]["code"], json!("rpc.unsupported_protocol"));
        assert_eq!(old["error"]["params"]["expected"], json!(PROTOCOL));
        assert_eq!(old["error"]["params"]["actual"], json!(2));

        let extended = engine.handle(
            &json!({
                "protocol": PROTOCOL, "id": "extended", "method": "health",
                "params": {}, "request_id": "legacy",
            })
            .to_string(),
        );
        assert_eq!(extended["id"], json!("extended"));
        assert_eq!(extended["error"]["code"], json!("rpc.invalid_request"));

        // 少一个字段同样拒绝。
        let short = engine
            .handle(&json!({"protocol": PROTOCOL, "id": "short", "method": "health"}).to_string());
        assert_eq!(short["error"]["code"], json!("rpc.invalid_request"));
    }

    #[test]
    fn request_id_is_capped_at_128_characters() {
        let engine = dispatcher(Arc::new(Recorder::default()));
        let long = "x".repeat(129);
        let response = call(&engine, "health", json!({}), &long);
        assert_eq!(response["error"]["code"], json!("rpc.invalid_request"));
        assert_eq!(response["id"], json!(long));

        let boundary = "x".repeat(128);
        assert_eq!(
            call(&engine, "health", json!({}), &boundary)["ok"],
            json!(true)
        );
    }

    #[test]
    fn non_string_id_falls_back_to_unknown() {
        let engine = dispatcher(Arc::new(Recorder::default()));
        let response = engine.handle(
            &json!({"protocol": PROTOCOL, "id": 7, "method": "health", "params": {}}).to_string(),
        );
        assert_eq!(response["ok"], json!(true));
        assert_eq!(response["id"], json!("unknown"));
    }

    #[test]
    fn params_and_method_shape_are_validated() {
        let engine = dispatcher(Arc::new(Recorder::default()));
        let bad_params = engine.handle(
            &json!({"protocol": PROTOCOL, "id": "a", "method": "health", "params": []}).to_string(),
        );
        assert_eq!(
            bad_params["error"]["params"]["message"],
            json!("params 必须是 JSON object")
        );
        let bad_method = engine.handle(
            &json!({"protocol": PROTOCOL, "id": "a", "method": "", "params": {}}).to_string(),
        );
        assert_eq!(
            bad_method["error"]["params"]["message"],
            json!("method 必须是非空字符串")
        );
    }

    #[test]
    fn unknown_method_is_structured() {
        let response = call(
            &dispatcher(Arc::new(Recorder::default())),
            "nope",
            json!({}),
            "x",
        );
        assert_eq!(response["error"]["code"], json!("rpc.unknown_method"));
        assert_eq!(
            response["error"]["params"],
            json!({"method": "nope", "message": "未知 method: nope"})
        );
    }

    #[test]
    fn metadata_write_is_not_a_generic_rpc_method() {
        let response = call(
            &dispatcher(Arc::new(Recorder::default())),
            "session_meta_set",
            json!({"id": "session", "patch": {"name": "direct write"}}),
            "x",
        );
        assert_eq!(response["error"]["code"], json!("rpc.unknown_method"));
    }

    #[test]
    fn missing_dispatch_param_is_structured() {
        let response = call(
            &dispatcher(Arc::new(Recorder::default())),
            "models",
            json!({}),
            "x",
        );
        assert_eq!(response["error"]["code"], json!("rpc.missing_param"));
        assert_eq!(
            response["error"]["params"],
            json!({"param": "tool", "message": "缺少参数: tool"})
        );
    }

    #[test]
    fn deep_key_errors_are_internal_not_missing_param() {
        let service = Arc::new(Recorder {
            failure: Some(EngineError::key_error("session_id")),
            ..Recorder::default()
        });
        let response = call(
            &dispatcher(service),
            "models",
            json!({"tool": "claude"}),
            "x",
        );
        assert_eq!(response["error"]["code"], json!("internal.unexpected"));
        assert_eq!(response["error"]["params"]["message"], json!("internal"));
    }

    #[test]
    fn domain_errors_keep_their_code_and_params() {
        let service = Arc::new(Recorder {
            failure: Some(DomainError::tool_unknown("nope").into()),
            ..Recorder::default()
        });
        let response = call(&dispatcher(service), "models", json!({"tool": "nope"}), "x");
        assert_eq!(response["error"]["code"], json!("tool.unknown"));
        assert_eq!(response["error"]["category"], json!("not-found"));
        assert_eq!(
            response["error"]["params"],
            json!({"tool": "nope", "message": "未知工具: nope"})
        );
    }

    #[test]
    fn probe_timeout_stays_a_ghost_code_on_the_wire() {
        let service = Arc::new(Recorder {
            failure: Some(DomainError::probe_timeout("探针超时: claude --version").into()),
            ..Recorder::default()
        });
        let response = call(
            &dispatcher(service),
            "models",
            json!({"tool": "claude"}),
            "x",
        );
        assert_eq!(
            response["error"],
            json!({
                "code": "probe.timeout",
                "params": {"message": "探针超时: claude --version"},
                "category": "internal",
                "retryable": true,
            })
        );
    }

    #[test]
    fn error_message_is_truncated_at_500_characters() {
        let service = Arc::new(Recorder {
            failure: Some(DomainError::agent_request_invalid("中".repeat(600)).into()),
            ..Recorder::default()
        });
        let response = call(&dispatcher(service), "models", json!({"tool": "x"}), "x");
        let message = response["error"]["params"]["message"].as_str().unwrap();
        assert_eq!(message.chars().count(), 500);
    }

    #[test]
    fn explicit_message_param_is_not_overwritten() {
        let mut params = Map::new();
        params.insert("message".into(), Value::from("自定义"));
        let error = DomainError::new(
            "agent.request_invalid",
            "AgentRequestError",
            "原始文案",
            params,
        );
        let envelope = error_envelope(&error, "x");
        assert_eq!(envelope["error"]["params"]["message"], json!("自定义"));
    }

    #[test]
    fn dispatch_defaults_are_part_of_the_wire_contract() {
        let service = Arc::new(Recorder::default());
        let engine = dispatcher(Arc::clone(&service));

        call(
            &engine,
            "show",
            json!({"tool": "claude", "ref": "fsr_a"}),
            "x",
        );
        let (_, payload) = service.last();
        assert_eq!(payload["from_message"], json!(1));
        assert_eq!(payload["limit"], json!(null));

        call(&engine, "pricing", json!({}), "x");
        assert_eq!(service.last().1["force"], json!(false));

        call(&engine, "agent_search_sessions", json!({}), "x");
        let (_, payload) = service.last();
        assert_eq!(payload["query"], json!(""));
        assert_eq!(payload["limit"], json!(20));
        assert_eq!(payload["scope"], json!("any"));
        assert_eq!(payload["include_tool_outputs"], json!(false));
        assert_eq!(payload["exhaustive"], json!(false));
        assert_eq!(payload["agents"], json!(null));

        call(
            &engine,
            "agent_session_read",
            json!({"tool": "claude", "ref": "fsr_a"}),
            "x",
        );
        let (_, payload) = service.last();
        assert_eq!(payload["from_message"], json!(1));
        assert_eq!(payload["limit"], json!(20));
        assert_eq!(payload["max_bytes"], json!(DEFAULT_CONTEXT_BYTES));

        call(
            &engine,
            "agent_prompt",
            json!({"tool": "claude", "ref": "fsr_a", "prompt": "hi"}),
            "x",
        );
        assert_eq!(service.last().1["timeout_sec"], json!(360));

        call(&engine, "session_search", json!({"query": "q"}), "x");
        assert_eq!(service.last().1["scope"], json!("any"));
    }

    #[test]
    fn every_contract_method_is_reachable() {
        let service = Arc::new(Recorder::default());
        let engine = dispatcher(Arc::clone(&service));
        // 每个方法都带齐必需参数，确认没有一个落进 unknown_method。
        let params: &[(&str, Value)] = &[
            ("health", json!({})),
            ("version", json!({})),
            ("scan", json!({})),
            ("scan_progress", json!({})),
            ("env", json!({})),
            ("resume", json!({"tool": "claude", "ref": "r"})),
            ("models", json!({"tool": "claude"})),
            ("history", json!({})),
            ("history_delete", json!({"id": "history_x"})),
            ("pricing", json!({})),
            ("show", json!({"tool": "claude", "ref": "r"})),
            (
                "session_asset",
                json!({"tool": "c", "ref": "r", "asset_id": "a"}),
            ),
            ("session_meta_list", json!({})),
            ("session_search", json!({"query": "q"})),
            ("runtime_sessions.load_all", json!({})),
            ("runtime_sessions.commit", json!({"update": {}})),
            ("runtime_sessions.delete", json!({"session_id": "s"})),
            (
                "runtime_sessions.truncate",
                json!({"session_id": "s", "from_ordinal": 0, "from_seq": 0}),
            ),
            ("agent_search_sessions", json!({})),
            ("agent_session_read", json!({"tool": "c", "ref": "r"})),
            ("agent_get_usage", json!({})),
            (
                "agent_prompt",
                json!({"tool": "c", "ref": "r", "prompt": "p"}),
            ),
            ("operation.plan", json!({"input": {}})),
            ("operation.apply", json!({"plan_id": "op_x"})),
            ("operation.status", json!({"plan_id": "op_x"})),
            ("operation.cancel", json!({"plan_id": "op_x"})),
        ];
        assert_eq!(params.len(), ENGINE_METHOD_NAMES.len());
        for (method, value) in params {
            let response = call(&engine, method, value.clone(), "x");
            assert_eq!(response["ok"], json!(true), "method={method} -> {response}");
        }
    }
}
