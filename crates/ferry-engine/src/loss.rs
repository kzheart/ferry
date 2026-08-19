//! 会话保真度损耗目录：谁产生 loss code，谁声明它的后果。
//!
//! 共享迁移层不持有任何 Agent 私有 code。某个 Adapter 独有的降级/丢弃语义在
//! 产生它的模块里 [`declare`]；Rust 没有 import 副作用，各 adapter 必须在自己的
//! 装配入口（`build()`）里调用一次 `declare`。重复声明必须一致，避免同一 code
//! 在两处得到不同后果。

use std::collections::BTreeMap;
use std::sync::{LazyLock, RwLock};

use crate::events::Event;

/// 损耗后果：降级（内容仍在，形态变了）/ 丢弃（内容没了）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    Degraded,
    Dropped,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Degraded => "degraded",
            Self::Dropped => "dropped",
        }
    }
}

/// 跨 Adapter 通用的 canonical 损耗语义（进程启动即生效）。
const SHARED_OUTCOMES: &[(&str, Outcome)] = &[
    ("migration.children_not_migrated", Outcome::Dropped),
    ("migration.fork_parent_fallback", Outcome::Degraded),
    ("migration.reasoning_dropped", Outcome::Dropped),
    ("migration.reasoning_metadata_dropped", Outcome::Degraded),
    ("migration.truncated", Outcome::Dropped),
    ("migration.unknown_block_dropped", Outcome::Dropped),
    ("session.child_foreign_ignored", Outcome::Dropped),
    ("session.child_parent_conflict", Outcome::Dropped),
    ("session.unpaired_tool_use", Outcome::Degraded),
];

static CATALOG: LazyLock<RwLock<BTreeMap<&'static str, Outcome>>> =
    LazyLock::new(|| RwLock::new(SHARED_OUTCOMES.iter().copied().collect()));

/// 声明一批损耗后果。重复声明必须一致，否则是装配缺陷（对齐 Python 的
/// `ValueError`：Python 在 import 期抛出会直接打死进程，这里同样 panic）。
pub fn declare(outcomes: &[(&'static str, Outcome)]) {
    // 冲突检查放在读锁下：std 的 RwLock 只在**写锁**期间 panic 才会中毒，
    // 这样冲突 panic 不会让整个进程后续的查询都炸掉。
    {
        let catalog = CATALOG.read().expect("损耗目录锁中毒");
        for (code, value) in outcomes {
            if let Some(existing) = catalog.get(code) {
                assert!(
                    existing == value,
                    "损耗后果声明冲突: {code} 已声明为 {}，又声明为 {}",
                    existing.as_str(),
                    value.as_str()
                );
            }
        }
    }
    let mut catalog = CATALOG.write().expect("损耗目录锁中毒");
    for (code, value) in outcomes {
        catalog.insert(code, *value);
    }
}

/// 返回一条损耗记录的保真度后果；未声明的 code 不计入迁移差异。
pub fn outcome(loss: &Event) -> Option<Outcome> {
    outcome_for_code(&loss.code)
}

/// 直接按 code 查询。
pub fn outcome_for_code(code: &str) -> Option<Outcome> {
    CATALOG.read().expect("损耗目录锁中毒").get(code).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Map;

    #[test]
    fn shared_canonical_codes_are_declared_up_front() {
        assert_eq!(
            outcome_for_code("migration.truncated"),
            Some(Outcome::Dropped)
        );
        assert_eq!(
            outcome_for_code("session.unpaired_tool_use"),
            Some(Outcome::Degraded)
        );
        assert_eq!(outcome_for_code("never.declared"), None);
    }

    #[test]
    fn outcome_reads_the_event_code() {
        let event = Event::new("migration.reasoning_dropped", Map::new());
        assert_eq!(outcome(&event), Some(Outcome::Dropped));
    }

    #[test]
    fn repeated_declarations_must_agree() {
        declare(&[("test.loss_repeat", Outcome::Dropped)]);
        declare(&[("test.loss_repeat", Outcome::Dropped)]);
        assert_eq!(outcome_for_code("test.loss_repeat"), Some(Outcome::Dropped));
    }

    #[test]
    #[should_panic(expected = "损耗后果声明冲突")]
    fn conflicting_declarations_are_a_bug() {
        declare(&[("test.loss_conflict", Outcome::Dropped)]);
        declare(&[("test.loss_conflict", Outcome::Degraded)]);
    }
}
