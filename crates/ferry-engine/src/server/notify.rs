//! 引擎主动通知：与 RPC 响应共用 stdout 的事件帧。
//!
//! 语义事实源：`engine/server/notify.py`。
//!
//! 事件帧遵循 IPC 契约的 event 信封（`protocol` / `type` / `payload`，**无 `id`**，
//! §2.1 第 3 条），宿主按契约中的事件策略转发给前端。未绑定输出（一次性 rpc /
//! 测试）时静默丢弃。

use std::sync::{Arc, Mutex};

use serde_json::{Map, Value};

use crate::contracts::events::{event_policy, EventSource};
use crate::contracts::ipc::FERRY_IPC_PROTOCOL;

/// 单行写出回调；由 `serve` 提供，负责换行 / flush / 输出互斥。
pub type LineWriter = Arc<dyn Fn(&str) + Send + Sync>;

#[derive(Clone, Default)]
pub struct Notifier {
    writer: Arc<Mutex<Option<LineWriter>>>,
}

impl Notifier {
    pub fn new() -> Self {
        Self::default()
    }

    /// 绑定输出；`write` 接收单行字符串。
    pub fn bind(&self, write: LineWriter) {
        *self
            .writer
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(write);
    }

    /// 发一个引擎事件；未注册或非 engine 源的事件是缺陷，直接报错。
    pub fn emit(&self, event_type: &str, payload: Value) -> Result<(), String> {
        match event_policy(event_type) {
            Some(policy) if policy.source == EventSource::Engine => {}
            _ => return Err(format!("未注册的引擎事件: {event_type}")),
        }
        let write = self
            .writer
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        let Some(write) = write else {
            return Ok(());
        };
        let mut frame = Map::new();
        frame.insert("protocol".into(), Value::from(FERRY_IPC_PROTOCOL));
        frame.insert("type".into(), Value::from(event_type));
        frame.insert("payload".into(), payload);
        // 事件发送失败不能拖垮引擎主流程：写失败由 LineWriter 自行吞掉并记日志。
        write(&Value::Object(frame).to_string());
        Ok(())
    }
}

impl std::fmt::Debug for Notifier {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Notifier").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn event_frames_carry_no_id() {
        let lines: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let notifier = Notifier::new();
        let sink = Arc::clone(&lines);
        notifier.bind(Arc::new(move |line: &str| {
            sink.lock().unwrap().push(line.to_string());
        }));

        notifier
            .emit("sessions.changed", json!({"generation": 2}))
            .unwrap();

        let recorded = lines.lock().unwrap();
        let frame: Value = serde_json::from_str(&recorded[0]).unwrap();
        assert_eq!(frame["protocol"], json!("ferry-ipc/1"));
        assert_eq!(frame["type"], json!("sessions.changed"));
        assert_eq!(frame["payload"], json!({"generation": 2}));
        assert!(frame.get("id").is_none());
    }

    #[test]
    fn only_engine_sourced_events_are_allowed() {
        let notifier = Notifier::new();
        assert_eq!(
            notifier.emit("run.started", json!({})).unwrap_err(),
            "未注册的引擎事件: run.started"
        );
        assert_eq!(
            notifier.emit("nope.nope", json!({})).unwrap_err(),
            "未注册的引擎事件: nope.nope"
        );
    }

    #[test]
    fn unbound_notifier_drops_silently() {
        assert!(Notifier::new().emit("sessions.changed", json!({})).is_ok());
    }
}
