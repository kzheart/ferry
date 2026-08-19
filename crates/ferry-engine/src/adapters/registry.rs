//! Explicit composition of Ferry's built-in session adapters.
//!
//! 语义事实源：`engine/adapters/registry.py`。
//!
//! WP-A 只落地骨架：5 个 builder 槽位都指向占位实现，C1..C5 各自把自己的
//! `build()` 接进来（只改本文件里对应的一行，互不冲突）。

use std::collections::BTreeMap;

use crate::adapters::contracts::AgentAdapter;
use crate::contracts::agents::AGENT_IDS;
use crate::errors::{DomainError, DomainResult};

/// Adapter 装配函数：失败返回装配错误文本（对齐 Python 的 `ValueError`）。
pub type AdapterBuilder = fn() -> Result<AgentAdapter, String>;

/// 内置 adapter 的装配入口，顺序与 `AGENT_IDS` 一致。
pub const ADAPTER_BUILDERS: &[(&str, AdapterBuilder)] = &[
    ("claude", crate::adapters::claude::adapter::build),
    ("codex", crate::adapters::codex::adapter::build),
    ("opencode", crate::adapters::opencode::adapter::build),
    ("pi", crate::adapters::pi::adapter::build),
    ("grok", crate::adapters::grok::adapter::build),
    ("cursor", crate::adapters::cursor::adapter::build),
];

/// Immutable adapter lookup owned by the Engine composition root.
#[derive(Clone, Debug, Default)]
pub struct AdapterRegistry {
    items: BTreeMap<String, AgentAdapter>,
    order: Vec<String>,
}

impl AdapterRegistry {
    pub fn new(adapters: impl IntoIterator<Item = AgentAdapter>) -> Result<Self, String> {
        let mut items = BTreeMap::new();
        let mut order = Vec::new();
        for adapter in adapters {
            let id = adapter.id().to_string();
            if items.contains_key(&id) {
                return Err(format!("重复的 adapter id: {id}"));
            }
            order.push(id.clone());
            items.insert(id, adapter);
        }
        Ok(Self { items, order })
    }

    pub fn get(&self, tool: &str) -> DomainResult<&AgentAdapter> {
        self.items
            .get(tool)
            .ok_or_else(|| DomainError::tool_unknown(tool))
    }

    /// 装配顺序（等价 Python `tuple(self._items)`，dict 保序）。
    pub fn ids(&self) -> &[String] {
        &self.order
    }
}

/// builder 集合与契约声明的 agent 集合必须精确一致。
///
/// 单独抽出来是为了能在测试里喂入不匹配的两组 id：真实的
/// `ADAPTER_BUILDERS` / `AGENT_IDS` 都是常量，正常路径永远走不到报错分支。
fn check_builder_coverage(declared: &[&str], agent_ids: &[&str]) -> Result<(), String> {
    let mut missing: Vec<&str> = agent_ids
        .iter()
        .copied()
        .filter(|id| !declared.contains(id))
        .collect();
    let mut extra: Vec<&str> = declared
        .iter()
        .copied()
        .filter(|id| !agent_ids.contains(id))
        .collect();
    if missing.is_empty() && extra.is_empty() {
        return Ok(());
    }
    missing.sort_unstable();
    extra.sort_unstable();
    Err(format!(
        "Adapter builders 与 AGENT_IDS 不一致: missing={missing:?}, extra={extra:?}"
    ))
}

/// 装配全部内置 adapter；builder 集合必须与 `AGENT_IDS` 精确一致。
pub fn create_registry() -> Result<AdapterRegistry, String> {
    let declared: Vec<&str> = ADAPTER_BUILDERS.iter().map(|(id, _)| *id).collect();
    check_builder_coverage(&declared, AGENT_IDS)?;
    let mut adapters = Vec::with_capacity(AGENT_IDS.len());
    for agent_id in AGENT_IDS {
        let build = ADAPTER_BUILDERS
            .iter()
            .find(|(id, _)| id == agent_id)
            .map(|(_, build)| *build)
            .expect("上面已校验 builder 覆盖 AGENT_IDS");
        adapters.push(build()?);
    }
    AdapterRegistry::new(adapters)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builders_cover_exactly_the_generated_agent_ids() {
        let declared: Vec<&str> = ADAPTER_BUILDERS.iter().map(|(id, _)| *id).collect();
        assert_eq!(declared, AGENT_IDS);
    }

    #[test]
    fn duplicate_adapter_ids_are_rejected() {
        // 同一个 builder 跑两次就得到两个 id 相同的 adapter：注册表必须拒绝，
        // 否则后装配的那个会静默盖掉前一个。
        let first = crate::adapters::claude::adapter::build().expect("claude adapter 可装配");
        let second = crate::adapters::claude::adapter::build().expect("claude adapter 可装配");
        let error = AdapterRegistry::new([first, second]).unwrap_err();
        assert_eq!(error, "重复的 adapter id: claude");
    }

    #[test]
    fn builder_coverage_reports_both_directions() {
        assert!(check_builder_coverage(&["a", "b"], &["b", "a"]).is_ok());
        let error = check_builder_coverage(&["claude", "ghost"], &["claude", "codex"]).unwrap_err();
        assert_eq!(
            error,
            "Adapter builders 与 AGENT_IDS 不一致: missing=[\"codex\"], extra=[\"ghost\"]"
        );
    }

    #[test]
    fn unknown_tools_raise_tool_unknown() {
        let registry = AdapterRegistry::default();
        let error = registry.get("nope").unwrap_err();
        assert_eq!(error.code, "tool.unknown");
        assert_eq!(error.message(), "未知工具: nope");
    }
}
