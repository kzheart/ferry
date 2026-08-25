//! 性能基准辅助工具（非产品代码）。
//!
//! 跨版本对比时在**各自的提交**上编译本文件：旧提交带旧版 cli_bench，
//! 两个二进制天然可比。build 子命令顺带打印解析/写入的插桩计时
//! （`build_timing`，原 `index_bench` 的功能已并入此处）。
//!
//! 必须用隔离的 HOME 跑，否则会写到用户正在运行的 `~/.ferry`。
//!
//! 子命令：
//!   build                        冷扫描 + 建索引 + 等体积稳定（含段压实）
//!   query <标签>                 对当前 .ferry 里的索引跑一组检索，报 p50/p95
//!   cli <ferry-engine 可执行文件> 端到端 CLI 延迟（真的起 daemon、真的 fork）
//!   interfere <秒数>             构建进行中并发检索，报 p50/p95/最坏停顿

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;

use ferry_engine::server::rpc::{ContentSearchRequest, EngineService, SessionReadRequest};

fn sandbox_home() -> String {
    if let Ok(sandbox) = std::env::var("BENCH_HOME") {
        // SAFETY: 基准进程在起任何线程之前先改 HOME。
        unsafe { std::env::set_var("HOME", &sandbox) };
    }
    let home = std::env::var("HOME").unwrap_or_default();
    if !home.starts_with("/tmp/") && !home.starts_with("/private/tmp/") {
        eprintln!("拒绝运行：HOME 必须指向 /tmp 下的隔离沙箱，当前 {home}");
        std::process::exit(2);
    }
    home
}

fn index_path(home: &str) -> PathBuf {
    Path::new(home).join(".ferry").join("content-index.sqlite3")
}

fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

/// 百分位（最近秩法），输入需已排序。
fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = ((sorted.len() as f64) * fraction).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

struct Stats {
    p50: f64,
    p95: f64,
    max: f64,
    n: usize,
}

fn stats(mut samples: Vec<f64>) -> Stats {
    samples.sort_by(|a, b| a.partial_cmp(b).expect("耗时样本不会是 NaN"));
    Stats {
        p50: percentile(&samples, 0.50),
        p95: percentile(&samples, 0.95),
        max: samples.last().copied().unwrap_or_default(),
        n: samples.len(),
    }
}

fn report(label: &str, samples: Vec<f64>) {
    let stats = stats(samples);
    println!(
        "{label}\tn={}\tp50={:.1}ms\tp95={:.1}ms\tmax={:.1}ms",
        stats.n, stats.p50, stats.p95, stats.max
    );
}

fn search_request(query: &str, limit: i64, tool_outputs: bool) -> ContentSearchRequest {
    ContentSearchRequest {
        query: Value::from(query),
        limit: Value::from(limit),
        include_tool_outputs: Value::Bool(tool_outputs),
        exhaustive: Value::Bool(false),
        scope: Value::from("any"),
        ..ContentSearchRequest::default()
    }
}

/// 取第一条命中的 `(tool, ref)`，给 read 用。
fn first_hit(engine: &Arc<ferry_engine::app::Engine>, query: &str) -> Option<(String, String)> {
    let result = engine.content_search(&search_request(query, 5, false)).ok()?;
    let session = result.get("sessions")?.as_array()?.first()?;
    Some((
        session.get("tool")?.as_str()?.to_string(),
        session.get("ref")?.as_str()?.to_string(),
    ))
}

fn build_engine() -> Arc<ferry_engine::app::Engine> {
    ferry_engine::bootstrap::build_engine(None).expect("装配引擎")
}

/// 等索引文件体积连续 `quiet` 秒不变，认为后台（含段压实）已收敛。
fn wait_until_stable(path: &Path, quiet: Duration, budget: Duration) -> f64 {
    let started = Instant::now();
    let mut last = file_size(path);
    let mut last_change = Instant::now();
    while started.elapsed() < budget {
        std::thread::sleep(Duration::from_millis(500));
        let now = file_size(path);
        if now != last {
            last = now;
            last_change = Instant::now();
        } else if last_change.elapsed() >= quiet {
            break;
        }
    }
    (started.elapsed().saturating_sub(quiet)).as_secs_f64()
}

fn cmd_build(home: &str) {
    ferry_engine::server::serve::enable_stderr_logging();
    let engine = build_engine();
    let content = engine.content_index().expect("内容索引").clone();
    let path = index_path(home);

    let started = Instant::now();
    let records = engine.index().refresh().expect("冷扫描");
    println!(
        "cold_scan_sessions={}\tcold_scan_secs={:.2}",
        records.len(),
        started.elapsed().as_secs_f64()
    );

    let warm = Instant::now();
    let records = engine.index().refresh().expect("热扫描");
    println!("warm_scan_secs={:.2}", warm.elapsed().as_secs_f64());

    let build = Instant::now();
    content.sync(engine.index(), &records, true).expect("入队");
    let idle = content.wait_until_idle(Duration::from_secs(2400));
    let build_secs = build.elapsed().as_secs_f64();
    let usable_bytes = file_size(&path);
    // 压实（若该版本有）在队列排空后继续跑，wait_until_idle 不等它。
    let settle_secs = wait_until_stable(&path, Duration::from_secs(10), Duration::from_secs(900));

    let coverage = content.coverage(&records).expect("覆盖度");
    let indexed = coverage
        .get("indexed_sessions")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let (parse_nanos, write_nanos) = ferry_engine::sessions::content_index::build_timing();
    println!(
        "index_idle={idle}\tindex_secs={build_secs:.2}\tindexed={indexed}\trate={:.2}/s\t\
         parse_cpu_secs={:.1}\twrite_secs={:.1}",
        indexed as f64 / build_secs,
        parse_nanos as f64 / 1e9,
        write_nanos as f64 / 1e9
    );
    println!(
        "settle_secs={settle_secs:.1}\tbytes_at_usable={usable_bytes}\tbytes_final={}",
        file_size(&path)
    );
    engine.close();
}

/// 代表性检索集：短词、多词 AND、大结果集、含工具输出、纯元数据过滤。
fn query_cases() -> Vec<(&'static str, ContentSearchRequest)> {
    vec![
        ("search/short-term", search_request("sqlite", 10, false)),
        ("search/multi-term", search_request("sqlite index revision", 10, false)),
        ("search/large-limit", search_request("error", 50, false)),
        ("search/tool-outputs", search_request("cargo", 10, true)),
        (
            "search/metadata-only",
            ContentSearchRequest {
                query: Value::from(""),
                include_tool_outputs: Value::Bool(false),
                agents: Value::Array(vec![Value::from("codex")]),
                limit: Value::from(50),
                exhaustive: Value::Bool(false),
                scope: Value::from("any"),
                ..ContentSearchRequest::default()
            },
        ),
    ]
}

fn cmd_query(home: &str, label: &str, rounds: usize) {
    let engine = build_engine();
    let content = engine.content_index().expect("内容索引").clone();
    let records = engine.index().refresh().expect("扫描");
    println!(
        "{label}\tsessions={}\tbytes={}",
        records.len(),
        file_size(&index_path(home))
    );
    // 预热：把「本轮新增会话」的前台补建挤掉，不让它算进检索耗时。
    for (_, request) in query_cases() {
        let _ = engine.content_search(&request);
    }
    content.wait_until_idle(Duration::from_secs(600));

    for (name, request) in query_cases() {
        let mut samples = Vec::with_capacity(rounds);
        for _ in 0..rounds {
            let started = Instant::now();
            engine.content_search(&request).expect("检索");
            samples.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        report(&format!("{label}\t{name}"), samples);
    }

    if let Some((tool, reference)) = first_hit(&engine, "sqlite") {
        let context = SessionReadRequest {
            tool: Value::from(tool.as_str()),
            reference: Value::from(reference.as_str()),
            limit: Value::from(40i64),
            include_tool_outputs: Value::Bool(false),
            inert: Value::Bool(false),
            ..SessionReadRequest::default()
        };
        let searching = SessionReadRequest {
            terms: Value::Array(vec![Value::from("sqlite")]),
            ..context.clone()
        };
        for (name, request) in [("read/context", context), ("read/terms", searching)] {
            let mut samples = Vec::with_capacity(rounds);
            for _ in 0..rounds {
                let started = Instant::now();
                engine.session_read(&request).expect("读取");
                samples.push(started.elapsed().as_secs_f64() * 1000.0);
            }
            report(&format!("{label}\t{name}"), samples);
        }
    }
    engine.close();
}

/// 构建进行中并发检索：这是批量写事务最可疑的回退面。
fn cmd_interfere(_home: &str, window_secs: u64) {
    let engine = build_engine();
    let content = engine.content_index().expect("内容索引").clone();
    let records = engine.index().refresh().expect("扫描");
    println!("interfere\tsessions={}", records.len());

    let stop = Arc::new(AtomicBool::new(false));
    let readers: Vec<_> = [("search", 0usize), ("read", 1usize)]
        .into_iter()
        .map(|(kind, _)| {
            let engine = engine.clone();
            let stop = stop.clone();
            let kind = kind.to_string();
            std::thread::spawn(move || {
                let request = search_request("sqlite", 10, false);
                let read = first_hit(&engine, "sqlite").map(|(tool, reference)| SessionReadRequest {
                    tool: Value::from(tool),
                    reference: Value::from(reference),
                    limit: Value::from(40i64),
                    include_tool_outputs: Value::Bool(false),
                    inert: Value::Bool(false),
                    ..SessionReadRequest::default()
                });
                let mut samples = Vec::new();
                while !stop.load(Ordering::Relaxed) {
                    let started = Instant::now();
                    if kind == "search" {
                        let _ = engine.content_search(&request);
                    } else if let Some(read) = read.as_ref() {
                        let _ = engine.session_read(read);
                    }
                    samples.push(started.elapsed().as_secs_f64() * 1000.0);
                    std::thread::sleep(Duration::from_millis(200));
                }
                (kind, samples)
            })
        })
        .collect();

    // 读者先跑起来（并完成各自的首次 first_hit）再开建，测的才是构建期的干扰。
    std::thread::sleep(Duration::from_secs(3));
    content.sync(engine.index(), &records, true).expect("入队");
    std::thread::sleep(Duration::from_secs(window_secs));
    stop.store(true, Ordering::Relaxed);
    for handle in readers {
        let (kind, samples) = handle.join().expect("读者线程");
        report(&format!("interfere\t{kind}"), samples);
    }
    let coverage = content.coverage(&records).expect("覆盖度");
    println!(
        "interfere\tindexed_during_window={}",
        coverage
            .get("indexed_sessions")
            .and_then(Value::as_i64)
            .unwrap_or(0)
    );
    engine.close();
}

/// 端到端 CLI：真的 fork `ferry-engine <子命令>`，含进程启动与 socket 往返。
fn cmd_cli(home: &str, binary: &str, rounds: usize) {
    let run = |argv: &[&str]| -> (f64, String) {
        let started = Instant::now();
        let output = std::process::Command::new(binary)
            .args(argv)
            .env("HOME", home)
            .output()
            .expect("执行 CLI");
        (
            started.elapsed().as_secs_f64() * 1000.0,
            String::from_utf8_lossy(&output.stdout).into_owned(),
        )
    };
    // 第一次调用会拉起 daemon 并触发预热扫描，不计入。
    let _ = run(&["env"]);
    let _ = run(&["scan", "--wait"]);

    // read 需要一个本进程 daemon 现签的 fsr_ ref。
    let (_, hit) = run(&["search", "sqlite", "--limit", "1"]);
    let reference = serde_json::from_str::<Value>(&hit)
        .ok()
        .and_then(|value| {
            let session = value.get("sessions")?.as_array()?.first()?.clone();
            Some((
                session.get("tool")?.as_str()?.to_string(),
                session.get("ref")?.as_str()?.to_string(),
            ))
        });

    let mut cases: Vec<(&str, Vec<&str>)> = vec![
        ("cli/env", vec!["env"]),
        ("cli/daemon-status", vec!["daemon", "status"]),
        ("cli/scan-warm", vec!["scan"]),
        ("cli/usage-all", vec!["usage"]),
        ("cli/usage-since", vec!["usage", "--since", "2026-08-01"]),
        ("cli/search-short", vec!["search", "sqlite"]),
        ("cli/search-multi", vec!["search", "sqlite", "index", "revision"]),
        ("cli/search-large", vec!["search", "error", "--limit", "50"]),
        ("cli/search-metadata", vec!["search", "--agent", "codex", "--limit", "50"]),
    ];
    let owned;
    if let Some((tool, reference)) = reference.as_ref() {
        owned = (tool.clone(), reference.clone());
        cases.push(("cli/read-context", vec!["read", &owned.0, &owned.1, "--limit", "40"]));
        cases.push((
            "cli/read-terms",
            vec!["read", &owned.0, &owned.1, "--limit", "40", "--terms", "sqlite"],
        ));
    }

    for (name, argv) in &cases {
        let mut samples = Vec::with_capacity(rounds);
        for _ in 0..rounds {
            samples.push(run(argv).0);
        }
        report(name, samples);
    }
    let _ = run(&["daemon", "stop"]);
}

fn main() {
    let home = sandbox_home();
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let rounds: usize = std::env::var("BENCH_ROUNDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(7);
    match argv.first().map(String::as_str) {
        Some("build") => cmd_build(&home),
        Some("query") => cmd_query(&home, argv.get(1).map(String::as_str).unwrap_or("query"), rounds),
        Some("cli") => cmd_cli(
            &home,
            argv.get(1).expect("用法: cli <ferry-engine 可执行文件>"),
            rounds,
        ),
        Some("interfere") => cmd_interfere(
            &home,
            argv.get(1)
                .and_then(|value| value.parse().ok())
                .unwrap_or(180),
        ),
        _ => {
            eprintln!("用法: cli_bench build|query <标签>|cli <bin>|interfere <秒>");
            std::process::exit(2);
        }
    }
}
