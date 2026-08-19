//! 跨工具会话扫描。
//!
//! 语义事实源：`engine/sessions/scan.py`。
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sessions_are_sorted_by_updated_descending() {
        let mut rows: Vec<Map<String, Value>> = [3i64, 1, 2, 3]
            .iter()
            .enumerate()
            .map(|(position, updated)| {
                json!({"id": position, "updated": updated})
                    .as_object()
                    .unwrap()
                    .clone()
            })
            .collect();
        rows.sort_by_key(|row| std::cmp::Reverse(updated_of(row)));
        let ids: Vec<i64> = rows.iter().map(|row| row["id"].as_i64().unwrap()).collect();
        // 相等的 updated 保持原有相对次序（0 在 3 之前）。
        assert_eq!(ids, vec![0, 3, 2, 1]);
    }
}
