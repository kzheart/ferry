//! 两阶段操作（plan/apply）的状态机、存储与执行。

pub mod delete;
pub mod edit;
pub mod executor;
pub mod history;
pub mod history_store;
pub mod metadata;
pub mod metadata_store;
pub mod migrate;
pub mod plan_store;
pub mod planner;
pub mod service;
pub mod snapshots;
pub mod state_store;
pub mod types;
pub mod validation;
pub mod verification;
