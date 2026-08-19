//! Agent 适配器：把各 Agent 的原生会话格式收敛到 canonical 模型。
//!
//! 分层规则（替代 `scripts/check-engine-layering.py`）：`adapters` 不得引用
//! `crate::operations` 与 `crate::sessions`。Python 现状里 adapters →
//! sessions.usage / sessions.topology 的倒置依赖，在 Rust 侧由
//! `adapters::shared` 提供并由 sessions 复用，方向反转。

pub mod contracts;
pub mod registry;

pub mod claude;
pub mod codex;
pub mod grok;
pub mod opencode;
pub mod pi;
pub mod shared;
