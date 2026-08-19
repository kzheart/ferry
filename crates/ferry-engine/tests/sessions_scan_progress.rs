//! 扫描进度的端到端接线验证（WP-D ↔ WP-B2）。
//!
//! 进度上报是注册式的：`adapters::shared::scanner` 定义 `ScanProgress` trait 与
//! `install_scan_progress`，`sessions::scan_progress` 实现并注册。没注册上时全部
//! 上报都是空操作且不报错，所以「到底有没有注册上」必须真的跑一遍 `scan_jsonl`
//! 才算数。
//!
//! 放在集成测试里而不是单测里：`TRACKER` 是进程级单例，单测二进制里被并发跑的
//! 索引测试反复 `begin/end`，对它做数值断言必然 flaky；集成测试各有自己的进程。

use std::io::Write as _;
use std::path::Path;

use ferry_engine::adapters::contracts::ScanCache as ScanCachePort;
use ferry_engine::adapters::shared::scanner::{scan_jsonl, ScanOutcome};
use ferry_engine::errors::DomainResult;
use ferry_engine::jsonutil::FileStat;
use ferry_engine::sessions::scan_cache::ScanCache;
use ferry_engine::sessions::scan_progress::{install_tracker, TRACKER};
use serde_json::{Map, Value};

#[test]
fn scan_jsonl_reports_progress_through_the_registered_tracker() {
    let temp = tempfile::tempdir().expect("临时目录");
    let sessions = temp.path().join("projects");
    std::fs::create_dir_all(&sessions).expect("建目录");
    for position in 0..7 {
        let path = sessions.join(format!("session-{position}.jsonl"));
        let mut file = std::fs::File::create(&path).expect("建会话文件");
        writeln!(file, r#"{{"type":"user","text":"第 {position} 条"}}"#).expect("写入");
    }

    // 组合根之外也必须接通：`AgentSessionIndex::new` 已经调过一次，这里再调
    // 一次验证幂等（底层是 OnceLock）。
    install_tracker();
    install_tracker();

    TRACKER.begin(&["claude".to_string()]);
    TRACKER.start_tool("claude");

    let cache = ScanCache::new(Some(temp.path().join("scan-cache.json")));
    let pattern = format!("{}/*.jsonl", sessions.to_string_lossy());
    let parse = |path: &Path, _stat: &FileStat| -> DomainResult<ScanOutcome> {
        let mut row = Map::new();
        row.insert(
            "id".into(),
            Value::from(path.file_stem().unwrap().to_string_lossy().into_owned()),
        );
        row.insert("updated".into(), Value::from(0));
        Ok(ScanOutcome::Row(row))
    };
    let rows = scan_jsonl(&pattern, &cache as &dyn ScanCachePort, &parse).expect("扫描成功");
    assert_eq!(rows.len(), 7);

    // 注册钩子接通了才会有非零进度：`set_total` 来自 scan_jsonl，
    // `advance` 来自每个文件的 scan_one。
    let snapshot = TRACKER.snapshot();
    assert_eq!(snapshot["state"], Value::from("running"));
    assert_eq!(snapshot["total"], Value::from(7));
    assert_eq!(snapshot["processed"], Value::from(7));
    assert_eq!(snapshot["tools"]["claude"]["total"], Value::from(7));

    TRACKER.finish_tool("claude");
    TRACKER.finalize();
    assert_eq!(TRACKER.snapshot()["phase"], Value::from("finalizing"));
    TRACKER.end();
    let idle = TRACKER.snapshot();
    assert_eq!(idle["state"], Value::from("idle"));
    // 不在扫描中的上报一律忽略：数字不再变。
    TRACKER.advance(5);
    assert_eq!(TRACKER.snapshot()["processed"], idle["processed"]);
}
