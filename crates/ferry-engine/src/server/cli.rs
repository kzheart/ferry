//! 引擎命令行入口：一个二进制、两种角色。
//!
//! **维护者工具**（进程内直接跑，装配自己的引擎实例）：`serve` / `rpc` /
//! `show` / `extract-format`。
//!
//! **薄客户端**（连 socket 上的常驻引擎，见 [`crate::server::commands`]）：
//! `search` / `read` / `usage` / `resume` / `migrate` / `history` / `scan` /
//! `daemon` / `env` / `health` / `version`。
//!
//! 语义升级（本轮）：`health` / `scan` / `history` / `env` 从「一次性进程内
//! 调用」改为「走客户端路径」。原因不是形式统一，而是一次性进程在功能上不
//! 成立——`fsr_` ref 由进程内存签发、跨进程必然失效，内容索引首建也只在常驻
//! 后台线程里发生。改走客户端之后，这些命令看到的是与桌面 App 同一个引擎实例
//! 的状态；代价是它们会按需拉起 daemon。`rpc` 保留进程内语义，CI 的
//! frozen-sidecar smoke 依赖它。
//!
//! `extract-format` 是维护者工具，不在 `ferry-ipc/1` 方法表里：它只读一份原生
//! capture，产出 `native_schema` 的结构模板，用来核对 fixture 是否还是当前格式。
//!
//! `serve` 分支的顺序固定：stderr logging → socket 绑定（可选）→ 后台预热线程
//! → notifier 绑定（`enable_live_updates`），然后才进主循环。daemon 模式下
//! stdin **不是** RPC 通道，主线程改为等退出信号。

use std::io::{Read, Write};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::operations::types::EngineError;
use crate::server::commands::{self, ClientCommand};
use crate::server::notify::Notifier;
use crate::server::rpc::{EngineService, RpcDispatcher};
use crate::server::serve::{enable_stderr_logging, serve_on, Lanes, ServeHandler};
use crate::server::socket::{self, EngineMode, SocketConfig};

/// daemon 模式的默认空闲退出时长。
const DEFAULT_IDLE_EXIT: Duration = Duration::from_secs(600);

/// 维护者子命令。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Command {
    Rpc,
    Serve,
    Show,
    ExtractFormat,
}

impl Command {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "rpc" => Self::Rpc,
            "serve" => Self::Serve,
            "show" => Self::Show,
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
    /// `daemon.status` 里的内容索引覆盖度：只读，不触发扫描。
    pub content_index_status: Option<socket::ContentIndexStatus>,
}

impl CliDeps {
    pub fn new(service: Arc<dyn EngineService>) -> Self {
        Self {
            service,
            warm_agent_search: None,
            enable_live_updates: None,
            close: None,
            content_index_status: None,
        }
    }
}

impl std::fmt::Debug for CliDeps {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("CliDeps").finish_non_exhaustive()
    }
}

/// CLI 入口：解析子命令、装配组合根、执行、收尾。返回进程退出码。
///
/// `build` 放在参数上，测试可以在不拉起真实环境探测的前提下驱动整条分支。
pub fn main(
    argv: &[String],
    build: impl FnOnce() -> Result<CliDeps, String>,
) -> Result<u8, String> {
    let Some(raw) = argv.first() else {
        return Err("缺少命令".to_string());
    };
    // 客户端子命令不装配引擎：它连的是常驻实例，自己再起一份既慢又会双写
    // ferry-state.sqlite3。
    if let Some(command) = ClientCommand::parse(raw) {
        return commands::run(command, &argv[1..]);
    }
    // 未知命令刻意不在装配之前否掉：环境探测失败要先于「未知命令」报出来，
    // 报错顺序本身是宿主依赖的行为。
    let deps = build()?;
    let outcome = run(Command::parse(raw), raw, &argv[1..], &deps);
    if let Some(close) = &deps.close {
        close();
    }
    outcome.map(|()| 0)
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
        Command::Serve => serve_forever(dispatcher, deps, parse_serve_options(rest)?),
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

/// `serve [--socket [path]] [--mode app|daemon] [--idle-exit SEC]`。
///
/// 三种形态：不带 `--socket` 就是纯 stdio（现状）；`--socket` 是 App sidecar
/// 兼听；`--mode daemon` 是 CLI 自拉起的独立实例。`--mode daemon` 隐含开
/// socket——没有 socket 的 daemon 没有任何人能调用它。
///
/// 这里不复用 [`crate::server::args`]：`--socket` 允许省略取值（回落到
/// `FERRY_ENGINE_SOCKET` 或 `~/.ferry/engine.sock`），通用解析器为了不把
/// `--pattern --foo` 这类写法解错，不支持可选取值。
fn parse_serve_options(rest: &[String]) -> Result<Option<SocketConfig>, String> {
    let mut socket_requested = false;
    let mut path: Option<std::path::PathBuf> = None;
    let mut mode = EngineMode::App;
    let mut idle_exit: Option<Duration> = None;
    let mut index = 0;
    while index < rest.len() {
        let argument = rest[index].as_str();
        let (name, inline) = match argument.split_once('=') {
            Some((name, value)) => (name, Some(value.to_string())),
            None => (argument, None),
        };
        match name {
            "--socket" => {
                socket_requested = true;
                match inline {
                    Some(value) => path = Some(std::path::PathBuf::from(value)),
                    None => {
                        // 下一个 token 以 `--` 开头就说明取值被省略了。
                        if let Some(next) = rest.get(index + 1) {
                            if !next.starts_with("--") {
                                path = Some(std::path::PathBuf::from(next));
                                index += 1;
                            }
                        }
                    }
                }
            }
            "--mode" => {
                let value = take_value(&mut index, rest, inline, "--mode")?;
                mode = EngineMode::parse(&value)
                    .ok_or_else(|| format!("--mode 只接受 app|daemon: {value}"))?;
            }
            "--idle-exit" => {
                let value = take_value(&mut index, rest, inline, "--idle-exit")?;
                let seconds: u64 = value
                    .parse()
                    .map_err(|_| format!("--idle-exit 必须是秒数: {value}"))?;
                idle_exit = Some(Duration::from_secs(seconds));
            }
            other => return Err(format!("未知参数: {other}")),
        }
        index += 1;
    }
    if !socket_requested && mode == EngineMode::App {
        return Ok(None);
    }
    Ok(Some(SocketConfig {
        path: path.unwrap_or_else(socket::lock::default_socket_path),
        mode,
        // idle-exit 只对 daemon 有意义：App 的引擎跟着 App 的生命周期走。
        idle_exit: match mode {
            EngineMode::Daemon => Some(idle_exit.unwrap_or(DEFAULT_IDLE_EXIT)),
            EngineMode::App => None,
        },
    }))
}

fn take_value(
    index: &mut usize,
    rest: &[String],
    inline: Option<String>,
    name: &str,
) -> Result<String, String> {
    match inline {
        Some(value) => Ok(value),
        None => {
            *index += 1;
            rest.get(*index)
                .cloned()
                .ok_or_else(|| format!("{name} 缺少取值"))
        }
    }
}

fn serve_forever(
    dispatcher: RpcDispatcher,
    deps: &CliDeps,
    socket_config: Option<SocketConfig>,
) -> Result<(), String> {
    // 常驻模式：日志只能走 stderr（stdout 是 RPC 通道），宿主会把它接到日志文件。
    enable_stderr_logging();
    let dispatcher = Arc::new(dispatcher);
    // stdio 与全部 socket 连接共用同一对工作道：多开连接不得多开串行道。
    let lanes = Lanes::new();
    let daemon = socket_config
        .as_ref()
        .is_some_and(|config| config.mode == EngineMode::Daemon);
    let server = match &socket_config {
        Some(config) => Some(socket::start(
            config,
            Arc::clone(&lanes),
            Arc::clone(&dispatcher),
            deps.content_index_status.clone(),
        )?),
        None => None,
    };
    // 内容索引在后台预热，首个内容搜索到来时通常已就绪；预热完成才允许
    // idle-exit 开始计时（半截退出等于让下一次调用从头再建索引）。
    match &deps.warm_agent_search {
        Some(warm) => {
            let warm = Arc::clone(warm);
            let done = server.as_ref().map(|server| server.warm_notifier());
            let spawned = std::thread::Builder::new()
                .name("content-index-warmup".into())
                .spawn(move || {
                    warm();
                    if let Some(done) = done {
                        done();
                    }
                });
            // 预热线程起不来的话，idle 计时也得开始——否则 daemon 永不退出。
            if spawned.is_err() {
                if let Some(server) = &server {
                    server.mark_warm();
                }
            }
        }
        None => {
            if let Some(server) = &server {
                server.mark_warm();
            }
        }
    }
    if daemon {
        // daemon 模式：stdin 不是 RPC 通道，主线程只等退出信号。
        if let Some(server) = &server {
            server.run_until_shutdown();
        }
        lanes.shutdown();
        return Ok(());
    }
    // 活索引：源变更轮询 + 增量推送，只接 stdio——socket 连接不订阅事件。
    let notifier = Notifier::new();
    if let Some(enable) = &deps.enable_live_updates {
        enable(&notifier);
    }
    let handler: ServeHandler = Arc::new(move |request: &str| Ok(dispatcher.handle(request)));
    serve_on(
        lanes,
        std::io::BufReader::new(std::io::stdin()),
        Box::new(std::io::stdout()),
        handler,
        Some(&notifier),
    )
}

/// 能力门面的错误在 CLI 直接落成退出信息，不走 RPC 的错误包络。
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
    fn maintainer_command_table_covers_every_subcommand() {
        let cases = [
            ("rpc", Command::Rpc),
            ("serve", Command::Serve),
            ("show", Command::Show),
            ("extract-format", Command::ExtractFormat),
        ];
        for (raw, expected) in cases {
            assert_eq!(Command::parse(raw), Some(expected), "raw={raw}");
        }
        assert_eq!(Command::parse("nope"), None);
        assert_eq!(Command::parse("-v"), None);
        // 这四个已改走客户端路径，不再是进程内子命令。
        for raw in ["health", "scan", "history", "env", "version"] {
            assert_eq!(Command::parse(raw), None, "raw={raw}");
            assert!(ClientCommand::parse(raw).is_some(), "raw={raw}");
        }
    }

    #[test]
    fn serve_without_socket_flags_stays_pure_stdio() {
        assert!(parse_serve_options(&[]).unwrap().is_none());
    }

    #[test]
    fn serve_socket_flags_shape_the_three_deployments() {
        let argv =
            |items: &[&str]| -> Vec<String> { items.iter().map(|item| item.to_string()).collect() };
        // App sidecar：兼听 socket，不 idle-exit。
        let sidecar = parse_serve_options(&argv(&["--socket", "/tmp/a.sock"]))
            .unwrap()
            .expect("开了 socket");
        assert_eq!(sidecar.mode, EngineMode::App);
        assert_eq!(sidecar.path, std::path::PathBuf::from("/tmp/a.sock"));
        assert!(sidecar.idle_exit.is_none(), "App 的引擎不空闲退出");

        // daemon：隐含开 socket，idle-exit 有默认值。
        let daemon = parse_serve_options(&argv(&["--mode", "daemon"]))
            .unwrap()
            .expect("daemon 必然带 socket");
        assert_eq!(daemon.mode, EngineMode::Daemon);
        assert_eq!(daemon.idle_exit, Some(DEFAULT_IDLE_EXIT));
        assert_eq!(daemon.path, socket::lock::default_socket_path());

        // `--socket` 可省略取值；`=` 与空格两种写法等价。
        let explicit =
            parse_serve_options(&argv(&["--socket", "--mode=daemon", "--idle-exit", "30"]))
                .unwrap()
                .expect("开了 socket");
        assert_eq!(explicit.mode, EngineMode::Daemon);
        assert_eq!(explicit.idle_exit, Some(Duration::from_secs(30)));
        assert_eq!(explicit.path, socket::lock::default_socket_path());

        assert!(parse_serve_options(&argv(&["--mode", "both"])).is_err());
        assert!(parse_serve_options(&argv(&["--idle-exit", "soon"])).is_err());
        assert!(parse_serve_options(&argv(&["--nope"])).is_err());
        assert!(parse_serve_options(&argv(&["--mode"])).is_err());
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
    fn missing_command_reports_a_clear_message() {
        let error = main(&[], || Err("不该走到这里".to_string())).unwrap_err();
        assert_eq!(error, "缺少命令");
    }
}
