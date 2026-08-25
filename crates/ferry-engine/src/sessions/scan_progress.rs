//! 扫描进度跟踪。
//!
//! 全量刷新经 `AgentSessionIndex` 单飞合并，同一时刻至多一次在扫，任何来源
//! （UI 扫描、启动预热、agent 搜索）都驱动同一份进度；`scan_progress` 走
//! 独立轻量控制池，可在重读占满时继续查询。单例状态由锁保护；未处于扫描中的
//! 上报调用一律忽略。

use std::sync::{LazyLock, Mutex};

use serde_json::{Map, Value};

#[derive(Clone, Debug, Default)]
struct ToolProgress {
    processed: i64,
    total: Option<i64>,
    done: bool,
}

#[derive(Default)]
struct ProgressState {
    running: bool,
    phase: &'static str,
    /// 名字顺序必须与 `begin()` 的入参一致（DTO 里 tools 是有序 map）。
    order: Vec<String>,
    tools: std::collections::HashMap<String, ToolProgress>,
    current: Option<String>,
    /// finalizing 阶段（身份摘要/索引整理）的行级进度，与 tools 的读取计数无关。
    finalize_processed: i64,
    finalize_total: i64,
}

pub struct ScanProgressTracker {
    state: Mutex<ProgressState>,
}

impl Default for ScanProgressTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl ScanProgressTracker {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(ProgressState {
                running: false,
                phase: "reading",
                ..ProgressState::default()
            }),
        }
    }

    fn locked(&self) -> std::sync::MutexGuard<'_, ProgressState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn begin(&self, names: &[String]) {
        let mut state = self.locked();
        state.running = true;
        state.phase = "reading";
        state.current = None;
        state.order = names.to_vec();
        state.tools = names
            .iter()
            .map(|name| (name.clone(), ToolProgress::default()))
            .collect();
        state.finalize_processed = 0;
        state.finalize_total = 0;
    }

    /// 文件读取结束，进入索引整理阶段（index_rows/落盘）。
    /// `total` 是待整理的行数，供 UI 把这一阶段画成有终点的进度条。
    pub fn finalize(&self, total: i64) {
        let mut state = self.locked();
        if state.running {
            state.phase = "finalizing";
            state.current = None;
            state.finalize_processed = 0;
            state.finalize_total = total;
        }
    }

    /// finalizing 阶段的行级推进；不在该阶段的上报一律忽略。
    pub fn advance_finalize(&self, count: i64) {
        let mut state = self.locked();
        if state.running && state.phase == "finalizing" {
            state.finalize_processed += count;
        }
    }

    pub fn start_tool(&self, name: &str) {
        let mut state = self.locked();
        if state.running && state.tools.contains_key(name) {
            state.current = Some(name.to_string());
        }
    }

    pub fn set_total(&self, total: i64) {
        let mut state = self.locked();
        let Some(current) = state.current.clone() else {
            return;
        };
        if !state.running {
            return;
        }
        if let Some(tool) = state.tools.get_mut(&current) {
            tool.total = Some(total);
        }
    }

    pub fn advance(&self, count: i64) {
        let mut state = self.locked();
        let Some(current) = state.current.clone() else {
            return;
        };
        if !state.running {
            return;
        }
        if let Some(tool) = state.tools.get_mut(&current) {
            tool.processed += count;
        }
    }

    pub fn finish_tool(&self, name: &str) {
        let mut state = self.locked();
        if !state.running {
            return;
        }
        let cleared = match state.tools.get_mut(name) {
            Some(tool) => {
                tool.done = true;
                if tool.total.is_none() {
                    tool.total = Some(tool.processed);
                }
                true
            }
            None => false,
        };
        if cleared && state.current.as_deref() == Some(name) {
            state.current = None;
        }
    }

    pub fn end(&self) {
        let mut state = self.locked();
        state.running = false;
        state.current = None;
    }

    pub fn snapshot(&self) -> Map<String, Value> {
        let state = self.locked();
        let mut tools = Map::new();
        let mut processed = 0i64;
        let mut total = 0i64;
        for name in &state.order {
            let Some(tool) = state.tools.get(name) else {
                continue;
            };
            processed += tool.processed;
            if let Some(value) = tool.total {
                total += value;
            }
            let mut entry = Map::new();
            entry.insert("processed".into(), Value::from(tool.processed));
            entry.insert(
                "total".into(),
                tool.total.map(Value::from).unwrap_or(Value::Null),
            );
            entry.insert("done".into(), Value::Bool(tool.done));
            tools.insert(name.clone(), Value::Object(entry));
        }
        let mut payload = Map::new();
        payload.insert(
            "state".into(),
            Value::from(if state.running { "running" } else { "idle" }),
        );
        payload.insert("phase".into(), Value::from(state.phase));
        payload.insert("processed".into(), Value::from(processed));
        payload.insert("total".into(), Value::from(total));
        payload.insert("tools".into(), Value::Object(tools));
        let mut finalizing = Map::new();
        finalizing.insert("processed".into(), Value::from(state.finalize_processed));
        finalizing.insert("total".into(), Value::from(state.finalize_total));
        payload.insert("finalizing".into(), Value::Object(finalizing));
        payload
    }
}

/// `adapters::shared::scanner` 的进度回调面。
///
/// Python 里 adapters 直接 import 了 `sessions.scan_progress.TRACKER`，形成
/// `adapters → sessions` 的倒置；Rust 改成注册式（B2 定义 trait，sessions 实现
/// 并在组合根注册一次），方向反转后结构测试才成立。
impl crate::adapters::shared::scanner::ScanProgress for ScanProgressTracker {
    fn set_total(&self, total: usize) {
        ScanProgressTracker::set_total(self, total as i64);
    }

    fn advance(&self, count: usize) {
        ScanProgressTracker::advance(self, count as i64);
    }
}

/// 进程级单例（对齐 Python 的模块级 `TRACKER`）。
pub static TRACKER: LazyLock<ScanProgressTracker> = LazyLock::new(ScanProgressTracker::new);

/// 把全局 TRACKER 注册为 scanner 的进度出口。
///
/// `AgentSessionIndex::new` 已经调过一次（索引一存在，出口就接通，对齐 Python
/// 侧 import 即接线的语义）；组合根再调也无妨——底层是 `OnceLock`，重复调用
/// 无副作用。
pub fn install_tracker() {
    crate::adapters::shared::scanner::install_scan_progress(&*TRACKER);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn progress_accumulates_per_tool_and_totals() {
        let tracker = ScanProgressTracker::new();
        assert_eq!(tracker.snapshot()["state"], Value::from("idle"));
        tracker.begin(&names(&["claude", "codex"]));
        tracker.start_tool("claude");
        tracker.set_total(10);
        tracker.advance(3);
        tracker.finish_tool("claude");
        tracker.start_tool("codex");
        tracker.advance(2);
        tracker.finish_tool("codex");
        let snapshot = tracker.snapshot();
        assert_eq!(snapshot["state"], Value::from("running"));
        assert_eq!(snapshot["phase"], Value::from("reading"));
        assert_eq!(snapshot["processed"], Value::from(5));
        // codex 未显式 set_total，收尾时用 processed 补齐。
        assert_eq!(snapshot["total"], Value::from(12));
        assert_eq!(snapshot["tools"]["codex"]["total"], Value::from(2));
        assert_eq!(snapshot["tools"]["claude"]["done"], Value::Bool(true));
        tracker.finalize(12);
        let finalizing = tracker.snapshot();
        assert_eq!(finalizing["phase"], Value::from("finalizing"));
        assert_eq!(finalizing["finalizing"]["total"], Value::from(12));
        tracker.advance_finalize(5);
        assert_eq!(
            tracker.snapshot()["finalizing"]["processed"],
            Value::from(5)
        );
        tracker.end();
        // 扫描结束后 finalizing 上报同样忽略。
        tracker.advance_finalize(3);
        assert_eq!(tracker.snapshot()["finalizing"]["processed"], Value::from(5));
        assert_eq!(tracker.snapshot()["state"], Value::from("idle"));
    }

    #[test]
    fn reports_outside_a_scan_are_ignored() {
        let tracker = ScanProgressTracker::new();
        tracker.advance(5);
        tracker.set_total(9);
        tracker.finish_tool("claude");
        let snapshot = tracker.snapshot();
        assert_eq!(snapshot["processed"], Value::from(0));
        assert!(snapshot["tools"].as_object().unwrap().is_empty());
    }

    /// B2 的注册钩子把 `usize` 上报折进 `i64` 计数，且注册幂等。
    ///
    /// 只断言 trait 转发与幂等：全局 TRACKER 是进程单例，被并发跑的索引测试
    /// 反复 `begin/end`，对它的快照做数值断言必然是 flaky 的。
    #[test]
    fn the_registered_sink_forwards_to_the_inherent_methods() {
        use crate::adapters::shared::scanner::ScanProgress;

        install_tracker();
        install_tracker();

        let tracker = ScanProgressTracker::new();
        tracker.begin(&names(&["claude"]));
        tracker.start_tool("claude");
        let sink: &dyn ScanProgress = &tracker;
        sink.set_total(7);
        sink.advance(2);
        sink.advance(1);
        let snapshot = tracker.snapshot();
        assert_eq!(snapshot["total"], Value::from(7));
        assert_eq!(snapshot["processed"], Value::from(3));
    }

    #[test]
    fn unknown_tools_never_become_current() {
        let tracker = ScanProgressTracker::new();
        tracker.begin(&names(&["claude"]));
        tracker.start_tool("ghost");
        tracker.advance(4);
        assert_eq!(tracker.snapshot()["processed"], Value::from(0));
    }
}
