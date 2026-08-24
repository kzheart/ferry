//! 性能基准辅助工具（非产品代码）：冷扫描 + 内容索引全量构建计时。
//!
//! 必须用隔离的 HOME 跑，否则会写到用户正在运行的 `~/.ferry`。

use std::time::{Duration, Instant};

fn main() {
    // 沙箱 HOME 由 BENCH_HOME 传入并在进程内改写，避免外层 shell 继承。
    if let Ok(sandbox) = std::env::var("BENCH_HOME") {
        unsafe { std::env::set_var("HOME", &sandbox) };
    }
    let home = std::env::var("HOME").unwrap_or_default();
    if !home.starts_with("/tmp/") && !home.starts_with("/private/tmp/") {
        eprintln!("拒绝运行：HOME 必须指向 /tmp 下的隔离沙箱，当前 {home}");
        std::process::exit(2);
    }
    ferry_engine::server::serve::enable_stderr_logging();
    let engine = ferry_engine::bootstrap::build_engine(None).expect("装配引擎");
    let content_index = engine.content_index().expect("内容索引").clone();

    let started = Instant::now();
    let records = engine.index().refresh().expect("冷扫描");
    let scan_secs = started.elapsed().as_secs_f64();
    println!("cold_scan_sessions={} cold_scan_secs={scan_secs:.2}", records.len());

    let warm_started = Instant::now();
    let records = engine.index().refresh().expect("热扫描");
    println!("warm_scan_secs={:.2}", warm_started.elapsed().as_secs_f64());

    let build_started = Instant::now();
    content_index
        .sync(engine.index(), &records, true)
        .expect("入队");
    let limit: u64 = std::env::var("BENCH_INDEX_SECS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(120);
    let idle = content_index.wait_until_idle(Duration::from_secs(limit));
    let build_secs = build_started.elapsed().as_secs_f64();
    let coverage = content_index.coverage(&records).expect("覆盖度");
    let indexed = coverage
        .get("indexed_sessions")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);
    let (parse_nanos, write_nanos) = ferry_engine::sessions::content_index::build_timing();
    println!(
        "index_idle={idle} index_secs={build_secs:.2} indexed={indexed} rate={:.2}/s \
         parse_cpu_secs={:.1} write_secs={:.1}",
        indexed as f64 / build_secs,
        parse_nanos as f64 / 1e9,
        write_nanos as f64 / 1e9
    );
    let path = std::path::PathBuf::from(&home)
        .join(".ferry")
        .join("content-index.sqlite3");
    println!(
        "db_bytes={}",
        std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0)
    );
    engine.close();
}
