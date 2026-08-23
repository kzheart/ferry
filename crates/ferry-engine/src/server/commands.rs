//! `ferry` 薄客户端的子命令：命令行 → RPC 请求 → 原样打印。
//!
//! 输出契约（设计方案 §5.5 裁决 1/2）：
//!
//! - 成功：stdout 打引擎 `result` 的 pretty JSON。**不加工、不改词表**——
//!   impact 三分类以 `preview.differences.counts` 的 `exact/degraded/dropped`
//!   为准，摘要是 skill / agent 的职责，不是 CLI 的。
//! - 失败：stderr 打错误信封原样 JSON `{code, category, retryable, params}`；
//!   `unknown_ref` 之类是 `params.reason`，没有顶层 `error_type`。
//! - 退出码：成功 0 / 引擎业务错误 1 / 连接传输失败 2 / 等待超时 3。
//!
//! 多步命令（`scan --wait`、`migrate apply`）打印的是**它在等的那一步**的
//! 结果：等待的对象是内容索引状态与操作终态，不是触发调用的回执。
//!
//! 参数校验只做机械换算，业务规则交给引擎：`--exhaustive` 必须配 `--regex`、
//! `regex` 与 `query` 互斥这类判定，CLI 透传引擎的错误而不自建第二套词表。
//! 例外是引擎「合法但静默无效」的组合（`--roles` 不配 `--terms`）：引擎对
//! UI/runtime 的既有语义不动，CLI 侧把它判成用法错误，免得 agent 以为过滤生效了。
//!
//! 显示预算：`migrate plan` 的目标树与 `scan` 的全库 DTO 都是无界的，默认在
//! **打印层**收敛，`--full` 打未裁剪原文。裁剪只发生在这里，不是数据加工。

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

use crate::contracts::ipc::FERRY_CONTRACT_HASH;
use crate::contracts::operations::{OPERATION_SUCCESS_STATUS, OPERATION_TERMINAL_STATUSES};
use crate::server::args::{self, Parsed};
use crate::server::client::{self, Client, Failure};

/// `scan --wait` 的轮询间隔与默认超时。
const SCAN_POLL: Duration = Duration::from_secs(2);
const SCAN_TIMEOUT_SEC: u64 = 600;

/// `migrate apply` 的轮询间隔与超时。
const APPLY_POLL: Duration = Duration::from_secs(1);
const APPLY_TIMEOUT: Duration = Duration::from_secs(600);

/// 客户端子命令。既有的 `rpc` / `serve` / `show` / `extract-format` 不在此列，
/// 它们是维护者工具，继续走进程内路径。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClientCommand {
    Search,
    Read,
    Usage,
    Resume,
    Migrate,
    History,
    Scan,
    Daemon,
    Env,
    Health,
    Version,
}

impl ClientCommand {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "search" => Self::Search,
            "read" => Self::Read,
            "usage" => Self::Usage,
            "resume" => Self::Resume,
            "migrate" => Self::Migrate,
            "history" => Self::History,
            "scan" => Self::Scan,
            "daemon" => Self::Daemon,
            "env" => Self::Env,
            "health" => Self::Health,
            "version" | "--version" => Self::Version,
            _ => return None,
        })
    }
}

/// 一次子命令的结局。
enum Outcome {
    /// 打 result，退出 0。
    Done(Value),
    /// 结果拿到了，但业务上没成（如 apply 的终态是 failed）：打 result，退出 1。
    Unsuccessful(Value),
    /// 打错误信封，退出 1（引擎业务错误）或 2（连接/传输）。
    Failed(Failure),
    /// 等待超时：打最后一次状态，退出 3。
    TimedOut(Value),
}

/// 执行一条客户端子命令，返回进程退出码。
pub fn run(command: ClientCommand, argv: &[String]) -> Result<u8, String> {
    // version 是本地信息：不需要引擎，也就不该为了它拉起引擎。
    if command == ClientCommand::Version {
        return Ok(emit(Outcome::Done(local_version())));
    }
    let socket = client::default_socket();
    let outcome = match command {
        ClientCommand::Version => unreachable!("已在上面短路"),
        ClientCommand::Daemon => daemon(&socket, argv)?,
        ClientCommand::Search => call(&socket, "content_search", search_params(argv)?),
        ClientCommand::Read => call(&socket, "session_read", read_params(argv)?),
        ClientCommand::Usage => call(&socket, "usage_stats", usage_params(argv)?),
        ClientCommand::Resume => call(&socket, "resume", resume_params(argv)?),
        ClientCommand::History => call(&socket, "history", empty()),
        ClientCommand::Env => call(&socket, "env", empty()),
        ClientCommand::Health => call(&socket, "health", empty()),
        ClientCommand::Scan => scan(&socket, argv)?,
        ClientCommand::Migrate => migrate(&socket, argv)?,
    };
    Ok(emit(outcome))
}

fn emit(outcome: Outcome) -> u8 {
    match outcome {
        Outcome::Done(value) => {
            println!("{}", pretty(&value));
            0
        }
        // 终态本身是正常应答，走 stdout；退出码如实说「这次没成」。
        Outcome::Unsuccessful(value) => {
            println!("{}", pretty(&value));
            1
        }
        Outcome::Failed(failure) => {
            eprintln!("{}", pretty(&failure.payload()));
            failure.exit_code()
        }
        Outcome::TimedOut(value) => {
            println!("{}", pretty(&value));
            3
        }
    }
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn empty() -> Value {
    Value::Object(Map::new())
}

fn local_version() -> Value {
    let mut payload = Map::new();
    payload.insert(
        "version".into(),
        Value::from(crate::context::ENGINE_VERSION),
    );
    payload.insert("package".into(), Value::from(env!("CARGO_PKG_VERSION")));
    payload.insert("contract_hash".into(), Value::from(FERRY_CONTRACT_HASH));
    Value::Object(payload)
}

/// 连接（必要时自拉起）→ 发一条请求 → 断开。
fn call(socket: &Path, method: &str, params: Value) -> Outcome {
    with_client(socket, |client| client.call(method, params))
}

fn with_client(
    socket: &Path,
    action: impl FnOnce(&mut Client) -> Result<Value, Failure>,
) -> Outcome {
    match client::connect(socket).and_then(|mut client| action(&mut client)) {
        Ok(result) => Outcome::Done(result),
        // 超时带的是最后一次状态，走 stdout 而不是错误通道。
        Err(Failure::Timeout(status)) => Outcome::TimedOut(status),
        Err(failure) => Outcome::Failed(failure),
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

// ---------------------------------------------------------------------------
// 参数构造
// ---------------------------------------------------------------------------

fn insert_list(params: &mut Map<String, Value>, key: &str, items: Option<Vec<String>>) {
    if let Some(items) = items {
        params.insert(
            key.into(),
            Value::Array(items.into_iter().map(Value::from).collect()),
        );
    }
}

fn insert_int(params: &mut Map<String, Value>, key: &str, value: Option<i64>) {
    if let Some(value) = value {
        params.insert(key.into(), Value::from(value));
    }
}

/// `ferry search`：`--regex` 是开关，位置参数此时就是正则本体。
///
/// 引擎的 `regex` 参数收的是模式串且与 `query` 互斥，所以两者只能发一个。
fn search_params(argv: &[String]) -> Result<Value, String> {
    let parsed = args::parse(
        argv,
        &[
            "agent",
            "project",
            "session-id",
            "since",
            "until",
            "limit",
            "pattern",
            "scope",
        ],
        &["regex", "exhaustive", "tool-outputs"],
    )?;
    let mut params = Map::new();
    let query = parsed.positionals().join(" ");
    if parsed.has("regex") {
        if query.is_empty() {
            return Err("--regex 需要一个模式作为位置参数".to_string());
        }
        params.insert("regex".into(), Value::from(query));
    } else if !query.is_empty() {
        params.insert("query".into(), Value::from(query));
    }
    insert_list(&mut params, "agents", parsed.list("agent"));
    let projects = parsed.repeated("project");
    if !projects.is_empty() {
        insert_list(&mut params, "projects", Some(projects));
    }
    let session_ids = parsed.repeated("session-id");
    if !session_ids.is_empty() {
        insert_list(&mut params, "session_ids", Some(session_ids));
    }
    let patterns = parsed.repeated("pattern");
    if !patterns.is_empty() {
        insert_list(&mut params, "patterns", Some(patterns));
    }
    insert_int(&mut params, "limit", parsed.int("limit")?);
    if let Some(scope) = parsed.value("scope") {
        params.insert("scope".into(), Value::from(scope));
    }
    if parsed.has("tool-outputs") {
        params.insert("include_tool_outputs".into(), Value::Bool(true));
    }
    if parsed.has("exhaustive") {
        params.insert("exhaustive".into(), Value::Bool(true));
    }
    insert_time_range(&mut params, &parsed)?;
    Ok(Value::Object(params))
}

fn insert_time_range(params: &mut Map<String, Value>, parsed: &Parsed) -> Result<(), String> {
    if let Some(range) = args::time_range(parsed.value("since"), parsed.value("until"), now_ms())? {
        params.insert("time_range".into(), range);
    }
    Ok(())
}

fn read_params(argv: &[String]) -> Result<Value, String> {
    let parsed = args::parse(
        argv,
        &["from", "limit", "roles", "terms", "max-bytes"],
        &["tool-outputs", "inert"],
    )?;
    let roles = parsed.list("roles");
    let terms = parsed.list("terms");
    // 引擎只在 search 路径读 roles，context 路径拿到也不看：静默无效的过滤比
    // 报错危险得多（agent 会当成已经筛过），所以在这里就判死。
    if roles.is_some() && terms.is_none() {
        return Err("--roles 仅在 search 模式生效，需与 --terms 同用".to_string());
    }
    let mut params = Map::new();
    params.insert("tool".into(), Value::from(parsed.positional(0, "tool")?));
    params.insert("ref".into(), Value::from(parsed.positional(1, "ref")?));
    insert_int(&mut params, "from_message", parsed.int("from")?);
    insert_int(&mut params, "limit", parsed.int("limit")?);
    insert_int(&mut params, "max_bytes", parsed.int("max-bytes")?);
    insert_list(&mut params, "roles", roles);
    insert_list(&mut params, "terms", terms);
    if parsed.has("tool-outputs") {
        params.insert("include_tool_outputs".into(), Value::Bool(true));
    }
    if parsed.has("inert") {
        params.insert("inert".into(), Value::Bool(true));
    }
    Ok(Value::Object(params))
}

fn usage_params(argv: &[String]) -> Result<Value, String> {
    let parsed = args::parse(argv, &["agent", "project", "since", "until"], &[])?;
    let mut params = Map::new();
    insert_list(&mut params, "agents", parsed.list("agent"));
    let projects = parsed.repeated("project");
    if !projects.is_empty() {
        insert_list(&mut params, "projects", Some(projects));
    }
    insert_time_range(&mut params, &parsed)?;
    Ok(Value::Object(params))
}

fn resume_params(argv: &[String]) -> Result<Value, String> {
    let parsed = args::parse(argv, &[], &[])?;
    let mut params = Map::new();
    params.insert("tool".into(), Value::from(parsed.positional(0, "tool")?));
    params.insert("ref".into(), Value::from(parsed.positional(1, "ref")?));
    Ok(Value::Object(params))
}

/// `operation.plan(kind=migration)` 的 input：字段严格按
/// `contracts/operations.json` 的 `input_fields.migration`，不发多余字段。
/// 目标工作目录由引擎从源会话推导，所以没有 `--cwd`（§5.5 裁决 4）。
///
/// 返回值第二项是 `--full`：只影响打印，不进 input。
fn migrate_plan_params(argv: &[String]) -> Result<(Value, bool), String> {
    let parsed = args::parse(argv, &["to", "max-turn"], &["full"])?;
    let mut input = Map::new();
    input.insert("kind".into(), Value::from("migration"));
    input.insert(
        "source_tool".into(),
        Value::from(parsed.positional(0, "tool")?),
    );
    input.insert("ref".into(), Value::from(parsed.positional(1, "ref")?));
    input.insert(
        "target_tool".into(),
        Value::from(parsed.value("to").ok_or("缺少 --to <target>")?),
    );
    insert_int(&mut input, "max_turn", parsed.int("max-turn")?);
    let mut params = Map::new();
    params.insert("input".into(), Value::Object(input));
    Ok((Value::Object(params), parsed.has("full")))
}

fn plan_id_params(argv: &[String]) -> Result<Value, String> {
    let parsed = args::parse(argv, &[], &[])?;
    let mut params = Map::new();
    params.insert(
        "plan_id".into(),
        Value::from(parsed.positional(0, "plan_id")?),
    );
    Ok(Value::Object(params))
}

// ---------------------------------------------------------------------------
// 显示预算
//
// 这一节只改**打印形态**，不动引擎给的数据语义：被裁掉的字段都能用 `--full`
// 原样取回。裁剪的理由是 agent 上下文有限，而这两处结构在长会话/大库下无界。
// ---------------------------------------------------------------------------

const OMITTED_MARKER: &str = "[omitted: rerun with --full]";

/// plan 输出里两处无界结构：`preview.preview.root` 是整棵渲染后的目标树，
/// `preview.preview.differences.items` 每条都带完整 `source`/`target` 渲染文本
/// （真实库实测 229 条约 530KB）。默认各换成一句占位；影响转述所需的
/// `differences.counts` 与 `loss`（含 degrade/drop details）原样保留。
fn omit_plan_root(mut result: Value) -> Value {
    if let Some(render) = result
        .get_mut("preview")
        .and_then(|base| base.get_mut("preview"))
    {
        if let Some(root) = render.get_mut("root") {
            *root = Value::from(OMITTED_MARKER);
        }
        if let Some(items) = render
            .get_mut("differences")
            .and_then(|diff| diff.get_mut("items"))
        {
            let count = items.as_array().map(Vec::len).unwrap_or_default();
            *items = Value::from(format!("[omitted: {count} items; rerun with --full]"));
        }
    }
    result
}

/// `scan` 的 result 带全库会话 DTO。默认只交出可数的摘要：
/// `{tools, generation, session_count, sessions_by_tool}`。
fn scan_summary(result: &Value) -> Value {
    let sessions = result.get("sessions").and_then(Value::as_array);
    let mut by_tool: BTreeMap<&str, i64> = BTreeMap::new();
    for session in sessions.map(Vec::as_slice).unwrap_or_default() {
        let tool = session
            .get("tool")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        *by_tool.entry(tool).or_default() += 1;
    }
    let mut summary = Map::new();
    if let Some(tools) = result.get("tools") {
        summary.insert("tools".into(), tools.clone());
    }
    if let Some(generation) = result.get("generation") {
        summary.insert("generation".into(), generation.clone());
    }
    summary.insert(
        "session_count".into(),
        Value::from(sessions.map(Vec::len).unwrap_or(0)),
    );
    let counts: Map<String, Value> = by_tool
        .into_iter()
        .map(|(tool, count)| (tool.to_string(), Value::from(count)))
        .collect();
    summary.insert("sessions_by_tool".into(), Value::Object(counts));
    Value::Object(summary)
}

// ---------------------------------------------------------------------------
// 多步命令
// ---------------------------------------------------------------------------

/// `ferry scan [--wait] [--timeout SEC]`。
///
/// 先触发刷新；`--wait` 再每 2s 查一次内容索引状态直到 ready。打印的是
/// 「等待对象」——不带 `--wait` 打 `scan` 的结果，带 `--wait` 打最后一次
/// `daemon.status` 的结果（含 `content_index`）。
///
/// 不带 `--wait` 时默认只打摘要（见 [`scan_summary`]），`--full` 打原文；
/// `--wait` 的打印对象是 daemon 状态，本来就有界，不受影响。
fn scan(socket: &Path, argv: &[String]) -> Result<Outcome, String> {
    let parsed = args::parse(argv, &["timeout"], &["wait", "full"])?;
    let timeout = Duration::from_secs(
        parsed
            .int("timeout")?
            .filter(|value| *value > 0)
            .map(|value| value as u64)
            .unwrap_or(SCAN_TIMEOUT_SEC),
    );
    let wait = parsed.has("wait");
    let full = parsed.has("full");
    Ok(with_client(socket, |client| {
        let refreshed = client.call("scan", empty())?;
        if !wait {
            return Ok(if full {
                refreshed
            } else {
                scan_summary(&refreshed)
            });
        }
        let deadline = Instant::now() + timeout;
        loop {
            let status = client.call("daemon.status", empty())?;
            if status
                .get("content_index")
                .and_then(|index| index.get("ready"))
                == Some(&Value::Bool(true))
            {
                return Ok(status);
            }
            if Instant::now() >= deadline {
                // 超时不是错误信封：把最后状态照实交出去，由调用方决定。
                return Err(Failure::Timeout(status));
            }
            std::thread::sleep(SCAN_POLL);
        }
    }))
}

/// `ferry migrate plan|apply|status|cancel`。
fn migrate(socket: &Path, argv: &[String]) -> Result<Outcome, String> {
    let action = argv
        .first()
        .map(String::as_str)
        .ok_or("用法: ferry migrate plan|apply|status|cancel ...")?;
    let rest = &argv[1..];
    Ok(match action {
        "plan" => {
            let (params, full) = migrate_plan_params(rest)?;
            let outcome = call(socket, "operation.plan", params);
            match outcome {
                Outcome::Done(result) if !full => Outcome::Done(omit_plan_root(result)),
                other => other,
            }
        }
        "status" => call(socket, "operation.status", plan_id_params(rest)?),
        "cancel" => call(socket, "operation.cancel", plan_id_params(rest)?),
        "apply" => migrate_apply(socket, plan_id_params(rest)?),
        other => return Err(format!("未知的 migrate 子命令: {other}")),
    })
}

/// apply 只带 plan_id：plan 不可变，二传业务参数会被引擎拒绝。
/// 提交后轮询 `operation.status` 至终态，打印终态本身。
fn migrate_apply(socket: &Path, params: Value) -> Outcome {
    let outcome = with_client(socket, |client| {
        client.call("operation.apply", params.clone())?;
        let deadline = Instant::now() + APPLY_TIMEOUT;
        loop {
            let status = client.call("operation.status", params.clone())?;
            let terminal = status
                .get("status")
                .and_then(Value::as_str)
                .is_some_and(|state| OPERATION_TERMINAL_STATUSES.contains(&state));
            if terminal {
                return Ok(status);
            }
            if Instant::now() >= deadline {
                return Err(Failure::Timeout(status));
            }
            std::thread::sleep(APPLY_POLL);
        }
    });
    match outcome {
        Outcome::Done(status)
            if status.get("status").and_then(Value::as_str) != Some(OPERATION_SUCCESS_STATUS) =>
        {
            Outcome::Unsuccessful(status)
        }
        other => other,
    }
}

/// `ferry daemon status|stop`：都不自拉起（为报告现状而拉起一个是荒谬的）。
fn daemon(socket: &Path, argv: &[String]) -> Result<Outcome, String> {
    let action = argv
        .first()
        .map(String::as_str)
        .ok_or("用法: ferry daemon status|stop")?;
    let method = match action {
        "status" => "daemon.status",
        "stop" => "daemon.shutdown",
        other => return Err(format!("未知的 daemon 子命令: {other}")),
    };
    Ok(
        match client::attach(socket).and_then(|mut client| client.call(method, empty())) {
            Ok(result) => Outcome::Done(result),
            Err(failure) => Outcome::Failed(failure),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn command_table_covers_the_documented_surface() {
        for (raw, expected) in [
            ("search", ClientCommand::Search),
            ("read", ClientCommand::Read),
            ("usage", ClientCommand::Usage),
            ("resume", ClientCommand::Resume),
            ("migrate", ClientCommand::Migrate),
            ("history", ClientCommand::History),
            ("scan", ClientCommand::Scan),
            ("daemon", ClientCommand::Daemon),
            ("env", ClientCommand::Env),
            ("health", ClientCommand::Health),
            ("version", ClientCommand::Version),
            ("--version", ClientCommand::Version),
        ] {
            assert_eq!(ClientCommand::parse(raw), Some(expected), "{raw}");
        }
        assert_eq!(ClientCommand::parse("meta"), None, "meta 归 P2");
        assert_eq!(ClientCommand::parse("show"), None, "show 不对 CLI 暴露");
    }

    #[test]
    fn search_maps_flags_onto_content_search_params() {
        let params = search_params(&argv(&[
            "sqlite",
            "锁",
            "--agent",
            "claude,codex",
            "--project",
            "/tmp/a",
            "--limit",
            "5",
            "--scope",
            "content",
            "--pattern",
            "WAL",
            "--tool-outputs",
        ]))
        .expect("可解析");
        assert_eq!(params["query"], Value::from("sqlite 锁"));
        assert_eq!(params["agents"], serde_json::json!(["claude", "codex"]));
        assert_eq!(params["projects"], serde_json::json!(["/tmp/a"]));
        assert_eq!(params["limit"], Value::from(5));
        assert_eq!(params["scope"], Value::from("content"));
        assert_eq!(params["patterns"], serde_json::json!(["WAL"]));
        assert_eq!(params["include_tool_outputs"], Value::Bool(true));
        // 没给的参数一律不下发，默认值由引擎的分发层说了算。
        assert!(!params.as_object().unwrap().contains_key("time_range"));
        assert!(!params.as_object().unwrap().contains_key("exhaustive"));
        assert!(!params.as_object().unwrap().contains_key("regex"));
        assert!(!params.as_object().unwrap().contains_key("session_ids"));
    }

    /// `--session-id` 可重复，映射到 `session_ids`；可以不带任何 query。
    #[test]
    fn search_maps_repeated_session_id_flags() {
        let params = search_params(&argv(&[
            "--agent",
            "codex",
            "--session-id",
            "01a02803-9a5f-7b91-8610-37945d3b9478",
            "--session-id",
            "b2c3",
        ]))
        .expect("可解析");
        assert_eq!(
            params["session_ids"],
            serde_json::json!(["01a02803-9a5f-7b91-8610-37945d3b9478", "b2c3"])
        );
        assert_eq!(params["agents"], serde_json::json!(["codex"]));
        assert!(!params.as_object().unwrap().contains_key("query"));
    }

    #[test]
    fn regex_search_sends_the_pattern_instead_of_a_query() {
        let params = search_params(&argv(&["fo+bar", "--regex", "--exhaustive"])).expect("可解析");
        assert_eq!(params["regex"], Value::from("fo+bar"));
        assert_eq!(params["exhaustive"], Value::Bool(true));
        assert!(
            !params.as_object().unwrap().contains_key("query"),
            "regex 与 query 互斥，不能同时下发"
        );
        assert!(search_params(&argv(&["--regex"])).is_err());
    }

    #[test]
    fn time_flags_become_epoch_millisecond_ranges() {
        let params = search_params(&argv(&[
            "x",
            "--since",
            "1970-01-02",
            "--until",
            "1970-01-03",
        ]))
        .expect("可解析");
        assert_eq!(params["time_range"]["from"], Value::from(86_400_000));
        assert_eq!(params["time_range"]["to"], Value::from(172_800_000));
        assert!(search_params(&argv(&["x", "--since", "昨天"])).is_err());
    }

    #[test]
    fn read_maps_positionals_and_flags() {
        let params = read_params(&argv(&[
            "claude",
            "fsr_1",
            "--from",
            "10",
            "--limit",
            "5",
            "--roles",
            "user,assistant",
            "--terms",
            "a,b",
            "--max-bytes",
            "4096",
            "--tool-outputs",
        ]))
        .expect("可解析");
        assert_eq!(params["tool"], Value::from("claude"));
        assert_eq!(params["ref"], Value::from("fsr_1"));
        assert_eq!(params["from_message"], Value::from(10));
        assert_eq!(params["max_bytes"], Value::from(4096));
        assert_eq!(params["roles"], serde_json::json!(["user", "assistant"]));
        assert_eq!(params["terms"], serde_json::json!(["a", "b"]));
        assert!(read_params(&argv(&["claude"])).is_err());
    }

    #[test]
    fn inert_is_a_switch_that_only_appears_when_asked_for() {
        let plain = read_params(&argv(&["claude", "fsr_1"])).expect("可解析");
        assert!(
            !plain.as_object().unwrap().contains_key("inert"),
            "没给就不下发，默认值由引擎分发层说了算"
        );
        let lazy = read_params(&argv(&["claude", "fsr_1", "--inert"])).expect("可解析");
        assert_eq!(lazy["inert"], Value::Bool(true));
        // `--inert=1` 这种写法不是开关，应报未知用法而不是被静默当成 true。
        assert!(read_params(&argv(&["claude", "fsr_1", "--inert=1"])).is_err());
    }

    #[test]
    fn roles_without_terms_is_a_usage_error() {
        let error = read_params(&argv(&["claude", "fsr_1", "--roles", "user"]))
            .expect_err("context 模式下 roles 无效，不能静默放过");
        assert!(error.contains("--roles"), "{error}");
        assert!(error.contains("--terms"), "{error}");
        // 与 --terms 同用则照常下发。
        let params = read_params(&argv(&[
            "claude", "fsr_1", "--roles", "user", "--terms", "a",
        ]))
        .expect("可解析");
        assert_eq!(params["roles"], serde_json::json!(["user"]));
    }

    #[test]
    fn migration_input_matches_the_operations_contract() {
        let (params, full) = migrate_plan_params(&argv(&[
            "claude",
            "fsr_1",
            "--to",
            "codex",
            "--max-turn",
            "12",
        ]))
        .expect("可解析");
        assert!(!full, "--full 默认关");
        let input = params["input"].as_object().expect("input 是对象");
        assert_eq!(input["kind"], Value::from("migration"));
        assert_eq!(input["source_tool"], Value::from("claude"));
        assert_eq!(input["ref"], Value::from("fsr_1"));
        assert_eq!(input["target_tool"], Value::from("codex"));
        assert_eq!(input["max_turn"], Value::from(12));
        // 契约里 migration 没有 cwd；多发字段等于自建第二套契约。
        let allowed = ["kind", "source_tool", "ref", "target_tool", "max_turn"];
        for key in input.keys() {
            assert!(allowed.contains(&key.as_str()), "多余字段: {key}");
        }
        assert!(migrate_plan_params(&argv(&["claude", "fsr_1"])).is_err());
        let (_, full) = migrate_plan_params(&argv(&["claude", "fsr_1", "--to", "codex", "--full"]))
            .expect("可解析");
        assert!(full);
    }

    fn plan_result() -> Value {
        serde_json::json!({
            "plan_id": "op_1",
            "preview": {
                "src": "claude",
                "loss": {"drop": 1},
                "preview": {
                    "target_tool": "codex",
                    "root": {"messages": [{"text": "很长的一棵树"}]},
                    "differences": {
                        "counts": {"exact": 3, "degraded": 1, "dropped": 1},
                        "items": [{"kind": "dropped"}]
                    }
                }
            }
        })
    }

    #[test]
    fn plan_root_and_items_are_omitted_unless_full_is_asked_for() {
        let trimmed = omit_plan_root(plan_result());
        assert_eq!(
            trimmed["preview"]["preview"]["root"],
            Value::from(OMITTED_MARKER)
        );
        // items 换成带条数的占位；counts / loss 一个字节不动。
        assert_eq!(
            trimmed["preview"]["preview"]["differences"]["items"],
            Value::from("[omitted: 1 items; rerun with --full]")
        );
        assert_eq!(
            trimmed["preview"]["preview"]["differences"]["counts"],
            plan_result()["preview"]["preview"]["differences"]["counts"]
        );
        assert_eq!(trimmed["preview"]["loss"], plan_result()["preview"]["loss"]);
        assert_eq!(trimmed["plan_id"], Value::from("op_1"));
    }

    #[test]
    fn plan_trimming_tolerates_a_missing_root() {
        let bare = serde_json::json!({"plan_id": "op_1"});
        assert_eq!(omit_plan_root(bare.clone()), bare);
    }

    #[test]
    fn scan_defaults_to_a_countable_summary() {
        let result = serde_json::json!({
            "tools": {"claude": {"installed": true}},
            "generation": 7,
            "sessions": [
                {"tool": "claude", "ref": "fsr_1"},
                {"tool": "claude", "ref": "fsr_2"},
                {"tool": "codex", "ref": "fsr_3"},
                {"ref": "fsr_4"}
            ]
        });
        let summary = scan_summary(&result);
        assert_eq!(summary["tools"], result["tools"]);
        assert_eq!(summary["generation"], Value::from(7));
        assert_eq!(summary["session_count"], Value::from(4));
        assert_eq!(
            summary["sessions_by_tool"],
            serde_json::json!({"claude": 2, "codex": 1, "unknown": 1})
        );
        assert!(
            summary.get("sessions").is_none(),
            "全库 DTO 不进默认输出，要原文得 --full"
        );
    }

    #[test]
    fn scan_summary_survives_an_empty_library() {
        let summary = scan_summary(&serde_json::json!({"tools": {}, "generation": 0}));
        assert_eq!(summary["session_count"], Value::from(0));
        assert_eq!(summary["sessions_by_tool"], serde_json::json!({}));
    }

    #[test]
    fn apply_only_carries_the_plan_id() {
        let params = plan_id_params(&argv(&["op_abc"])).expect("可解析");
        let object = params.as_object().expect("是对象");
        assert_eq!(object["plan_id"], Value::from("op_abc"));
        assert_eq!(object.len(), 1, "plan 不可变，不得二传业务参数");
    }

    #[test]
    fn exit_codes_follow_the_output_contract() {
        assert_eq!(emit(Outcome::Done(Value::Null)), 0);
        assert_eq!(
            emit(Outcome::Failed(Failure::Engine(Value::Null))),
            1,
            "引擎业务错误"
        );
        assert_eq!(
            emit(Outcome::Failed(Failure::Transport(
                crate::errors::DomainError::engine_unavailable("connect_failed", "x", "y")
            ))),
            2,
            "连接/传输失败"
        );
        assert_eq!(emit(Outcome::TimedOut(Value::Null)), 3, "等待超时");
        assert_eq!(
            emit(Outcome::Unsuccessful(
                serde_json::json!({"status": "failed"})
            )),
            1,
            "终态不是 applied 就不算成功"
        );
    }
}
