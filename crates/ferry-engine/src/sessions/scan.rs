//! 跨工具会话扫描。
//!
//! 活索引就绪后 `scan` 直接返回内存快照（毫秒级），同时 nudge 活索引在后台
//! 校准；增量经 `sessions.changed` 事件推给前端。只有首次（快照尚未建立）才
//! 阻塞在全量扫描上，且与启动预热单飞合并。

use serde_json::{Map, Value};

use crate::errors::DomainResult;

use super::index::{session_dto, AgentSessionIndex};
use super::live::LiveIndexService;
use super::scan_progress::TRACKER;

/// `scan` RPC：快照优先，回落全量刷新。
pub fn scan(
    index: &AgentSessionIndex,
    live: Option<&LiveIndexService>,
) -> DomainResult<Map<String, Value>> {
    let (tools, records, generation) = match index.snapshot_with_status() {
        Some(snapshot) => {
            if let Some(live) = live {
                live.nudge();
            }
            snapshot
        }
        None => {
            let (tools, records) = index.refresh_with_status()?;
            // generation 与事件同源：刷新后现读，避免与并发 delta 错位。
            let generation = index.generation();
            (tools, records, generation)
        }
    };
    let mut sessions: Vec<Map<String, Value>> = records.iter().map(session_dto).collect();
    // 稳定降序（Python `sort(reverse=True)` 不打乱相等项的相对次序）。
    sessions.sort_by_key(|session| std::cmp::Reverse(updated_of(session)));
    let mut payload = Map::new();
    payload.insert("tools".into(), Value::Object(tools));
    payload.insert(
        "sessions".into(),
        Value::Array(sessions.into_iter().map(Value::Object).collect()),
    );
    payload.insert("generation".into(), Value::from(generation));
    Ok(payload)
}

fn updated_of(session: &Map<String, Value>) -> i64 {
    session
        .get("updated")
        .and_then(Value::as_i64)
        .unwrap_or_default()
}

/// `scan_progress` RPC。
pub fn scan_progress() -> Map<String, Value> {
    TRACKER.snapshot()
}
