//! 契约生成物。
//!
//! 本目录下除 `mod.rs` 外的所有文件都由 `scripts/generate-contracts.py`
//! 的 `engine-rust` 目标生成，请勿手改；新增一类契约时同步补一行 `pub mod`。

pub mod agents;
pub mod engine_methods;
pub mod errors;
pub mod events;
pub mod ipc;
pub mod operations;
pub mod session_ref;
