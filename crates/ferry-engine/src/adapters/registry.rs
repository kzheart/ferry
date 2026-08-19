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

/// 装配全部内置 adapter；builder 集合必须与 `AGENT_IDS` 精确一致。
pub fn create_registry() -> Result<AdapterRegistry, String> {
    let declared: Vec<&str> = ADAPTER_BUILDERS.iter().map(|(id, _)| *id).collect();
    let mut missing: Vec<&str> = AGENT_IDS
        .iter()
        .copied()
        .filter(|id| !declared.contains(id))
        .collect();
    let mut extra: Vec<&str> = declared
        .iter()
        .copied()
        .filter(|id| !AGENT_IDS.contains(id))
        .collect();
    if !missing.is_empty() || !extra.is_empty() {
        missing.sort_unstable();
        extra.sort_unstable();
        return Err(format!(
            "Adapter builders 与 AGENT_IDS 不一致: missing={missing:?}, extra={extra:?}"
        ));
    }
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
    fn unknown_tools_raise_tool_unknown() {
        let registry = AdapterRegistry::default();
        let error = registry.get("nope").unwrap_err();
        assert_eq!(error.code, "tool.unknown");
        assert_eq!(error.message(), "未知工具: nope");
    }
}
