//! 迁移历史：由 Engine 独占的 SQLite 状态持久化。
//!
//! 语义事实源：`engine/operations/history.py`。
//!
//! history_id = `history_` + `token_urlsafe(18)`（§2.2 第 14 条）。

use serde_json::Value;

use crate::operations::history_store::HistoryDeletion;
use crate::operations::plan_store::token_urlsafe;
use crate::operations::types::{EngineResult, Ports};
use crate::storage::database::state_database;

pub const HISTORY_ID_PREFIX: &str = "history_";

pub fn append(entry: &Value, ports: &Ports) -> EngineResult<String> {
    let history_id = format!("{HISTORY_ID_PREFIX}{}", token_urlsafe(18));
    state_database(ports.state_dir())?
        .migration_history
        .append(&history_id, entry)?;
    Ok(history_id)
}

pub fn list_entries(ports: &Ports) -> EngineResult<Vec<Value>> {
    state_database(ports.state_dir())?
        .migration_history
        .list_all()
}

pub fn delete(history_id: &str, ports: &Ports) -> EngineResult<HistoryDeletion> {
    state_database(ports.state_dir())?
        .migration_history
        .delete(history_id)
}
