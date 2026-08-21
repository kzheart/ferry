//! daemon 的空闲退出计时。
//!
//! 与时钟解耦：调用方喂 `now_ms`，单测才能不睡觉地把边界钉死。三个前提缺一
//! 不可——**预热完成**、**没有活动连接**、**距最后一次连接关闭超过 N 秒**。
//! 预热是硬前提：内容索引首建要十几分钟，中途退出等于让下一次 CLI 调用从头
//! 再来。App 模式不用它（App 的引擎跟着 App 的生命周期走）。

use std::time::Duration;

#[derive(Debug)]
pub struct IdleTracker {
    idle_after_ms: u64,
    active: u32,
    warm: bool,
    /// 进入空闲的时刻；有连接在时为 `None`。
    idle_since_ms: Option<u64>,
}

impl IdleTracker {
    pub fn new(idle_after: Duration) -> Self {
        Self {
            // 钳到 u64：`Duration::MAX`（App 模式的占位）不能回绕成一个小阈值。
            idle_after_ms: idle_after.as_millis().min(u128::from(u64::MAX)) as u64,
            active: 0,
            warm: false,
            idle_since_ms: None,
        }
    }

    /// 预热完成。此刻若无连接，空闲计时从这里起算——被拉起后没人再来的
    /// daemon 也要能自己退场。
    pub fn mark_warm(&mut self, now_ms: u64) {
        self.warm = true;
        if self.active == 0 && self.idle_since_ms.is_none() {
            self.idle_since_ms = Some(now_ms);
        }
    }

    pub fn connection_opened(&mut self) {
        self.active += 1;
        self.idle_since_ms = None;
    }

    pub fn connection_closed(&mut self, now_ms: u64) {
        self.active = self.active.saturating_sub(1);
        if self.active == 0 {
            self.idle_since_ms = Some(now_ms);
        }
    }

    pub fn active(&self) -> u32 {
        self.active
    }

    pub fn should_exit(&self, now_ms: u64) -> bool {
        if !self.warm || self.active > 0 {
            return false;
        }
        self.idle_since_ms
            .is_some_and(|since| now_ms.saturating_sub(since) >= self.idle_after_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracker() -> IdleTracker {
        IdleTracker::new(Duration::from_secs(600))
    }

    #[test]
    fn warmup_is_a_hard_precondition() {
        let mut idle = tracker();
        idle.connection_opened();
        idle.connection_closed(0);
        assert!(!idle.should_exit(10_000_000), "预热没完成就不许退");
        idle.mark_warm(0);
        assert!(idle.should_exit(600_000));
    }

    #[test]
    fn active_connections_hold_the_daemon_open() {
        let mut idle = tracker();
        idle.mark_warm(0);
        idle.connection_opened();
        assert!(!idle.should_exit(10_000_000));
        assert_eq!(idle.active(), 1);
        idle.connection_closed(1_000);
        assert!(!idle.should_exit(600_999), "不足 N 秒不退");
        assert!(idle.should_exit(601_000), "刚好 N 秒即退");
    }

    #[test]
    fn a_daemon_nobody_calls_still_exits() {
        let mut idle = tracker();
        idle.mark_warm(5_000);
        assert!(!idle.should_exit(604_999));
        assert!(idle.should_exit(605_000));
    }

    #[test]
    fn reconnecting_restarts_the_countdown() {
        let mut idle = tracker();
        idle.mark_warm(0);
        idle.connection_opened();
        idle.connection_closed(100);
        idle.connection_opened();
        assert!(!idle.should_exit(10_000_000));
        idle.connection_closed(500_000);
        assert!(!idle.should_exit(1_099_999));
        assert!(idle.should_exit(1_100_000));
    }
}
