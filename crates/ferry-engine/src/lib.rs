//! Ferry Session Engine 的 Rust 实现。
//!
//! 与 Python 引擎（`engine/`）是协议兼容的两套实现：同一个 `ferry-ipc/1`
//! 协议、同一个 `FERRY_CONTRACT_HASH`、同一组事件帧。迁移期内 Python 侧是
//! 行为基准（golden oracle），任何语义分歧都以 Python 源码为准。

pub mod adapters;
pub mod app;
pub mod bootstrap;
pub mod context;
pub mod contracts;
pub mod errors;
pub mod events;
pub mod jsonutil;
pub mod loss;
pub mod model;
pub mod operations;
pub mod runtime;
pub mod server;
pub mod sessions;
pub mod storage;
pub mod system;
pub mod tool_ops;
