//! Agent 适配器：把各 Agent 的原生会话格式收敛到 canonical 模型。
//!
//! 分层规则（由 `tests/structure.rs` 守住）：`adapters` 不得引用
//! `crate::operations` 与 `crate::sessions`。两边都要用的助手（用量归一、
//! 会话树装配等）住在 `adapters::shared`，由 sessions 复用，依赖单向向下。

pub mod contracts;
pub mod registry;

pub mod claude;
pub mod codex;
pub mod cursor;
pub mod grok;
pub mod opencode;
pub mod pi;
pub mod shared;
