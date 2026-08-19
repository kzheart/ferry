//! Ferry Session Engine：宿主通过 `ferry-ipc/1` 驱动的 sidecar。
//!
//! 协议面（`ferry-ipc/1` 版本号、`FERRY_CONTRACT_HASH`、方法表、事件帧）由
//! `contracts/` 生成，本 crate 只实现它；改协议要先改契约再重新生成。

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
