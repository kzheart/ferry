//! Engine 独占的会话元数据读写门面。
//!
//! 走 `state_database`（**不**触发崩溃恢复）：元数据查询绝不能把正在执行的
//! Operation 标为中断（§2.3 第 20 条）。

use serde_json::{Map, Value};

use crate::errors::DomainError;
use crate::operations::metadata_store::{metadata_key, MetadataChange};
use crate::operations::types::{EngineResult, Ports};
use crate::storage::database::{now_ms, state_database};

pub fn list_all(ports: &Ports) -> EngineResult<Map<String, Value>> {
    state_database(ports.state_dir())?.metadata.list_all()
}

pub fn key(tool: &str, session_id: &str) -> String {
    metadata_key(tool, session_id)
}

pub fn set_entry(
    tool: &str,
    session_id: &str,
    patch: &Map<String, Value>,
    ports: &Ports,
) -> EngineResult<Map<String, Value>> {
    state_database(ports.state_dir())?
        .metadata
        .set(tool, session_id, patch, now_ms())
}

/// 批准后的两阶段 CAS：expected 不匹配即 `ConcurrentModificationError`。
pub fn compare_and_set_entry(
    tool: &str,
    session_id: &str,
    expected: &Map<String, Value>,
    patch: &Map<String, Value>,
    ports: &Ports,
) -> EngineResult<Map<String, Value>> {
    let change = MetadataChange {
        tool: tool.to_string(),
        session_id: session_id.to_string(),
        expected: Some(expected.clone()),
        patch: patch.clone(),
    };
    let applied = state_database(ports.state_dir())?
        .metadata
        .compare_and_set(std::slice::from_ref(&change), now_ms())?
        .ok_or_else(|| DomainError::concurrent_modification("会话元数据在审批后已变化"))?;
    match applied.get(&key(tool, session_id)) {
        Some(Value::Object(entry)) => Ok(entry.clone()),
        _ => Err(crate::operations::types::EngineError::key_error(key(
            tool, session_id,
        ))),
    }
}
