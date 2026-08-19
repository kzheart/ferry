//! Ferry Runtime 会话事件的 Engine 侧门面。
//!
//! 只做形状校验（`session_id` 是 str、`timestamp` 是 str、`messages`/`events`
//! 是数组），不解释载荷内容；底层走 `cached_state_database`（不触发崩溃恢复）。

use std::path::Path;

use serde_json::{Map, Value};

use crate::errors::DomainError;
use crate::operations::types::EngineResult;
use crate::storage::database::cached_state_database;

pub fn load_all(state_dir: impl AsRef<Path>) -> EngineResult<Vec<Value>> {
    cached_state_database(state_dir)?
        .runtime_sessions
        .load_all()
}

/// 校验并提交一次 Runtime 更新。
pub fn commit(update: &Value, state_dir: impl AsRef<Path>) -> EngineResult<Value> {
    let Some(object) = update.as_object() else {
        return Err(DomainError::agent_request_invalid("runtime commit 必须是 object").into());
    };
    let session_id = object
        .get("metadata")
        .and_then(Value::as_object)
        .and_then(|metadata| metadata.get("session_id"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            DomainError::agent_request_invalid("runtime commit 缺少 metadata.session_id")
        })?
        .to_string();
    if !object.get("timestamp").is_some_and(Value::is_string) {
        return Err(DomainError::agent_request_invalid("runtime commit 缺少 timestamp").into());
    }
    for key in ["messages", "events"] {
        if object.get(key).is_some_and(|value| !value.is_array()) {
            return Err(DomainError::agent_request_invalid(format!(
                "runtime commit 的 {key} 必须是数组"
            ))
            .into());
        }
    }
    cached_state_database(state_dir)?
        .runtime_sessions
        .commit(update)?;
    let mut result = Map::new();
    result.insert("session_id".into(), Value::from(session_id));
    result.insert("committed".into(), Value::Bool(true));
    Ok(Value::Object(result))
}

/// 截断：`from_ordinal` / `from_seq` 必须是非负整数（bool 不算整数）。
pub fn truncate(
    session_id: &Value,
    from_ordinal: &Value,
    from_seq: &Value,
    state_dir: impl AsRef<Path>,
) -> EngineResult<Value> {
    let session_id = session_id
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| DomainError::agent_request_invalid("runtime truncate 缺少 session_id"))?;
    let mut bounds = [0_i64; 2];
    for (index, (name, value)) in [("from_ordinal", from_ordinal), ("from_seq", from_seq)]
        .into_iter()
        .enumerate()
    {
        bounds[index] = non_negative_integer(value).ok_or_else(|| {
            DomainError::agent_request_invalid(format!("runtime truncate 的 {name} 必须是非负整数"))
        })?;
    }
    let (messages, events) = cached_state_database(state_dir)?
        .runtime_sessions
        .truncate(session_id, bounds[0], bounds[1])?;
    let mut result = Map::new();
    result.insert("session_id".into(), Value::from(session_id));
    result.insert("messages_deleted".into(), Value::from(messages));
    result.insert("events_deleted".into(), Value::from(events));
    Ok(Value::Object(result))
}

/// Python 的 `isinstance(value, int) and not isinstance(value, bool) and value >= 0`。
fn non_negative_integer(value: &Value) -> Option<i64> {
    if value.is_boolean() {
        return None;
    }
    value.as_i64().filter(|number| *number >= 0)
}

pub fn delete(session_id: &Value, state_dir: impl AsRef<Path>) -> EngineResult<Value> {
    // 对齐 Python：delete 不做形状校验，非字符串 session_id 直接匹配不到行。
    let key = session_id.as_str().unwrap_or_default();
    let deleted = cached_state_database(state_dir)?
        .runtime_sessions
        .delete(key)?;
    let mut result = Map::new();
    result.insert("session_id".into(), session_id.clone());
    result.insert("deleted".into(), Value::Bool(deleted));
    Ok(Value::Object(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn commit_rejects_malformed_shapes() {
        let dir = tempfile::tempdir().unwrap();
        let cases: &[(Value, &str)] = &[
            (json!([]), "runtime commit 必须是 object"),
            (json!({}), "runtime commit 缺少 metadata.session_id"),
            (
                json!({"metadata": {"session_id": 1}}),
                "runtime commit 缺少 metadata.session_id",
            ),
            (
                json!({"metadata": {"session_id": "s"}}),
                "runtime commit 缺少 timestamp",
            ),
            (
                json!({"metadata": {"session_id": "s"}, "timestamp": "t", "messages": {}}),
                "runtime commit 的 messages 必须是数组",
            ),
            (
                json!({"metadata": {"session_id": "s"}, "timestamp": "t", "events": 1}),
                "runtime commit 的 events 必须是数组",
            ),
        ];
        for (value, expected) in cases {
            let error = commit(value, dir.path()).unwrap_err();
            assert_eq!(error.message(), *expected, "value={value}");
            assert_eq!(error.error_type(), "AgentRequestError");
        }
    }

    #[test]
    fn truncate_rejects_booleans_and_negatives() {
        let dir = tempfile::tempdir().unwrap();
        for bad in [json!(true), json!(-1), json!("0"), json!(1.5)] {
            let error = truncate(&json!("s"), &bad, &json!(0), dir.path()).unwrap_err();
            assert_eq!(
                error.message(),
                "runtime truncate 的 from_ordinal 必须是非负整数"
            );
        }
        let error = truncate(&json!(""), &json!(0), &json!(0), dir.path()).unwrap_err();
        assert_eq!(error.message(), "runtime truncate 缺少 session_id");
    }
}
