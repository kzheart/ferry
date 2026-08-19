//! 引擎命令行入口。
//!
//! 语义事实源：`engine/server/cli.py` 的 `main()`（86-131 行）。
//!
//! 子命令：`serve` / `rpc` / `health` / `version`(`--version`) / `scan` / `show` /
//! `history` / `env` / `extract-format`。除 `serve` 与 `rpc` 外都直接打印
//! `indent=2` 的 JSON。
//!
//! `extract-format` 是维护者工具，不在 `ferry-ipc/1` 方法表里：它只读一份原生
//! capture，产出 `native_schema` 的结构模板，用来核对 fixture 是否还是当前格式。
//!
//! `serve` 分支的三件事顺序固定：stderr logging → 后台预热线程 → notifier 绑定
//! （`enable_live_updates`），然后才进 [`crate::server::serve::serve`] 主循环。

use std::io::{Read, Write};
use std::sync::Arc;

use serde_json::Value;

use crate::operations::types::EngineError;
use crate::server::notify::Notifier;
use crate::server::rpc::{EngineService, RpcDispatcher};
use crate::server::serve::{enable_stderr_logging, serve, ServeHandler};

/// CLI 子命令；与 Python `main()` 的分支集合逐字对齐。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Command {
    Rpc,
    Serve,
    Health,
    Version,
    Scan,
    Show,
    History,
    Env,
    ExtractFormat,
}

impl Command {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "rpc" => Self::Rpc,
            "serve" => Self::Serve,
            "health" => Self::Health,
            "version" | "--version" => Self::Version,
            "scan" => Self::Scan,
            "show" => Self::Show,
            "history" => Self::History,
            "env" => Self::Env,
            "extract-format" => Self::ExtractFormat,
            _ => return None,
        })
    }
}

/// `application.warm_agent_search` / `application.close` 这类无参钩子。
pub type CliHook = Arc<dyn Fn() + Send + Sync>;
/// `application.enable_live_updates(notifier)`。
pub type LiveUpdateHook = Arc<dyn Fn(&Notifier) + Send + Sync>;

/// 组合根交给 CLI 的东西。WP-E 在 `bootstrap` 里装配它。
#[derive(Clone)]
pub struct CliDeps {
    pub service: Arc<dyn EngineService>,
    /// `application.warm_agent_search`：内容索引后台预热。
    pub warm_agent_search: Option<CliHook>,
    /// `application.enable_live_updates(notifier)`：活索引接管刷新。
    pub enable_live_updates: Option<LiveUpdateHook>,
    /// `application.close()`：所有分支的 finally。
    pub close: Option<CliHook>,
}

impl CliDeps {
    pub fn new(service: Arc<dyn EngineService>) -> Self {
        Self {
            service,
            warm_agent_search: None,
            enable_live_updates: None,
            close: None,
        }
    }
}

impl std::fmt::Debug for CliDeps {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("CliDeps").finish_non_exhaustive()
    }
}

/// 等价 `engine.server.cli.main(argv)`。
///
/// `build` 对应 Python 的 `build_engine()`；放在参数上是为了让 WP-E 在不改本文件
/// 的前提下接线真实组合根。
pub fn main(
    argv: &[String],
    build: impl FnOnce() -> Result<CliDeps, String>,
) -> Result<(), String> {
    let Some(raw) = argv.first() else {
        return Err("缺少命令".to_string());
    };
    // 未知命令要在 build_engine 之前就否掉？——不，Python 是先 build 再分支，
    // 这里保持一致：环境探测失败的报错顺序也是 wire 行为的一部分。
    let deps = build()?;
    let outcome = run(Command::parse(raw), raw, &argv[1..], &deps);
    if let Some(close) = &deps.close {
        close();
    }
    outcome
}

fn run(command: Option<Command>, raw: &str, rest: &[String], deps: &CliDeps) -> Result<(), String> {
    let Some(command) = command else {
        return Err(format!("未知命令: {raw}"));
    };
    let dispatcher = RpcDispatcher::new(Arc::clone(&deps.service))?;
    match command {
        Command::Rpc => {
            let request = match rest.first() {
                Some(request) => request.clone(),
                None => read_stdin()?,
            };
            print_line(&dispatcher.handle(&request).to_string())
        }
        Command::Serve => serve_forever(dispatcher, deps),
        Command::Health => print_pretty(&deps.service.health().map_err(cli_error)?),
        Command::Version => print_pretty(&deps.service.version().map_err(cli_error)?),
        Command::Scan => print_pretty(&deps.service.scan().map_err(cli_error)?),
        Command::Show => {
            let tool = positional(rest, 0)?;
            let reference = positional(rest, 1)?;
            // CLI 走的是能力门面而不是分发层，所以没有 from_message 默认值。
            print_pretty(
                &deps
                    .service
                    .show_session(
                        &Value::from(tool),
                        &Value::from(reference),
                        &Value::from(1),
                        &Value::Null,
                    )
                    .map_err(cli_error)?,
            )
        }
        Command::History => print_pretty(&deps.service.migration_history().map_err(cli_error)?),
        Command::Env => print_pretty(&deps.service.environment().map_err(cli_error)?),
        Command::ExtractFormat => {
            let agent = positional(rest, 0)?;
            let capture = positional(rest, 1)?;
            print_pretty(&extract_format(agent, std::path::Path::new(capture))?)
        }
    }
}

/// 从一份原生 capture 抽取结构模板。
///
/// 各家的 capture 形态不同：claude / codex / pi 是 JSONL 记录流，opencode 是单个
/// JSON（三张表的导出），grok 是一个 bundle 目录。
fn extract_format(agent: &str, capture: &std::path::Path) -> Result<Value, String> {
    use crate::adapters::{claude, codex, grok, opencode, pi};

    if !capture.exists() {
        return Err(format!("capture 不存在: {}", capture.display()));
    }
    let read_json = |path: &std::path::Path| -> Result<Value, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("读取 {} 失败: {error}", path.display()))?;
        serde_json::from_str(&text)
            .map_err(|error| format!("{} 不是合法 JSON: {error}", path.display()))
    };
    let read_jsonl = |path: &std::path::Path| -> Result<Vec<Value>, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("读取 {} 失败: {error}", path.display()))?;
        crate::adapters::shared::scanner::split_jsonl_lines(&text)
            .into_iter()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                serde_json::from_str(line).map_err(|error| format!("JSONL 行非法: {error}"))
            })
            .collect()
    };
    let templates = match agent {
        "claude" => claude::native_schema::extract_templates(&read_jsonl(capture)?)
            .map_err(|error| error.message().to_string())?,
        "codex" => codex::native_schema::extract_templates(&read_jsonl(capture)?)?,
        "pi" => pi::native_schema::extract_templates(&read_jsonl(capture)?)?,
        "opencode" => opencode::native_schema::extract_templates(&read_json(capture)?)
            .map_err(|error| error.message().to_string())?,
        "grok" => {
            let mut bundle = serde_json::Map::new();
            bundle.insert("summary".into(), read_json(&capture.join("summary.json"))?);
            bundle.insert(
                "updates".into(),
                Value::Array(read_jsonl(&capture.join("updates.jsonl"))?),
            );
            bundle.insert(
                "chat".into(),
                Value::Array(read_jsonl(&capture.join("chat_history.jsonl"))?),
            );
            return Ok(Value::Object(
                grok::native_schema::extract_templates(&Value::Object(bundle))?
                    .into_iter()
                    .collect(),
            ));
        }
        other => return Err(format!("不支持的 agent: {other}")),
    };
    Ok(Value::Object(templates))
}

fn serve_forever(dispatcher: RpcDispatcher, deps: &CliDeps) -> Result<(), String> {
    // 常驻模式：日志只能走 stderr（stdout 是 RPC 通道），宿主会把它接到日志文件。
    enable_stderr_logging();
    // 内容索引在后台预热，首个内容搜索到来时通常已就绪。
    if let Some(warm) = &deps.warm_agent_search {
        let warm = Arc::clone(warm);
        let _ = std::thread::Builder::new()
            .name("content-index-warmup".into())
            .spawn(move || warm());
    }
    // 活索引：源变更轮询 + 增量推送，预热完成后开始接管刷新。
    let notifier = Notifier::new();
    if let Some(enable) = &deps.enable_live_updates {
        enable(&notifier);
    }
    let dispatcher = Arc::new(dispatcher);
    let handler: ServeHandler = Arc::new(move |request: &str| Ok(dispatcher.handle(request)));
    serve(
        std::io::BufReader::new(std::io::stdin()),
        Box::new(std::io::stdout()),
        handler,
        Some(&notifier),
    )
}

/// 能力门面的异常在 CLI 直接落成退出信息（Python 侧是未捕获异常的 traceback）。
fn cli_error(error: EngineError) -> String {
    format!("{}: {}", error.error_type(), error.message())
}

fn positional(rest: &[String], index: usize) -> Result<&str, String> {
    rest.get(index)
        .map(String::as_str)
        .ok_or_else(|| format!("缺少第 {} 个参数", index + 1))
}

fn read_stdin() -> Result<String, String> {
    let mut buffer = String::new();
    std::io::stdin()
        .read_to_string(&mut buffer)
        .map_err(|error| format!("读取 stdin 失败: {error}"))?;
    Ok(buffer)
}

fn print_line(line: &str) -> Result<(), String> {
    writeln!(std::io::stdout(), "{line}").map_err(|error| format!("写入 stdout 失败: {error}"))
}

/// `json.dumps(result, ensure_ascii=False, indent=2)`。
fn print_pretty(value: &Value) -> Result<(), String> {
    let rendered =
        serde_json::to_string_pretty(value).map_err(|error| format!("序列化失败: {error}"))?;
    print_line(&rendered)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_table_matches_the_python_branches() {
        let cases = [
            ("rpc", Command::Rpc),
            ("serve", Command::Serve),
            ("health", Command::Health),
            ("version", Command::Version),
            ("--version", Command::Version),
            ("scan", Command::Scan),
            ("show", Command::Show),
            ("history", Command::History),
            ("env", Command::Env),
            ("extract-format", Command::ExtractFormat),
        ];
        for (raw, expected) in cases {
            assert_eq!(Command::parse(raw), Some(expected), "raw={raw}");
        }
        assert_eq!(Command::parse("nope"), None);
        assert_eq!(Command::parse("-v"), None);
    }

    /// `extract-format` 只是维护者工具，不得混进 `ferry-ipc/1` 的方法表。
    #[test]
    fn extract_format_is_not_an_rpc_method() {
        use crate::contracts::engine_methods::ENGINE_METHOD_NAMES;
        assert!(!ENGINE_METHOD_NAMES.contains(&"extract-format"));
        assert!(!ENGINE_METHOD_NAMES.contains(&"extract_format"));
    }

    #[test]
    fn extract_format_reads_each_native_capture_shape() {
        let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/agent_formats");
        for (agent, case, member) in [
            ("claude", "case-02-tools", "session.jsonl"),
            ("codex", "case-02-tools", "session.jsonl"),
            ("pi", "case-02-tools", "session.jsonl"),
            ("opencode", "case-02-tools", "session.json"),
            ("grok", "case-02-tools", ""),
        ] {
            let capture = fixtures.join(agent).join(case).join(member);
            let templates = extract_format(agent, &capture)
                .unwrap_or_else(|error| panic!("{agent} 抽取失败: {error}"));
            assert!(
                templates.as_object().is_some_and(|map| !map.is_empty()),
                "{agent} 的模板不应为空"
            );
        }
        assert!(extract_format("nope", &fixtures).is_err());
    }

    #[test]
    fn missing_command_reports_the_python_message() {
        let error = main(&[], || Err("不该走到这里".to_string())).unwrap_err();
        assert_eq!(error, "缺少命令");
    }
}
