//! Cursor 适配器（browse + migration-source + resume）。
//!
//! Cursor 只作为会话来源与接续目标：浏览与迁出严格只读，不再提供迁入写入。

pub mod adapter;
pub mod dialect;
pub mod lifecycle;
pub mod native_schema;
pub mod reader;
pub mod scanner;
pub mod store;
