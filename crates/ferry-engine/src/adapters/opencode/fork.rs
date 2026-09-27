//! OpenCode's messageID is an exclusive boundary. Resolve it from a fresh native
//! transcript instead of trusting IDs synthesized by Ferry or silently falling back.
use super::api::{OpenCodeApi, OpenCodeApiClient};
use crate::errors::{DomainError, DomainResult};
use serde_json::{json, Value};
use std::time::Duration;

pub fn exclusive_boundary(messages: &[Value], last: &str) -> DomainResult<Option<String>> {
    let position = messages
        .iter()
        .position(|m| m.pointer("/info/id").and_then(Value::as_str) == Some(last))
        .ok_or_else(|| DomainError::agent_request_invalid("OpenCode branch message is missing"))?;
    let info = &messages[position]["info"];
    if info["role"] != "assistant" || info["finish"] != "stop" || !info["error"].is_null() {
        return Err(DomainError::agent_request_invalid(
            "OpenCode answer has not completed",
        ));
    }
    messages
        .get(position + 1)
        .map(|m| {
            m.pointer("/info/id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| DomainError::agent_request_invalid("Invalid OpenCode boundary"))
        })
        .transpose()
}

pub fn create(request: &Value) -> DomainResult<Value> {
    let id = request["sessionId"].as_str().unwrap_or("");
    let last = request["lastMessageId"].as_str().unwrap_or("");
    // Session IDs are path components, never URLs.
    if ![id, last]
        .iter()
        .all(|id| !id.is_empty() && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_'))
    {
        return Err(DomainError::agent_request_invalid(
            "Invalid OpenCode native IDs",
        ));
    }
    let api = OpenCodeApi::start(
        request["cwd"].as_str().unwrap_or("."),
        Duration::from_secs(30),
    )?;
    api.assert_idle(id)?;
    let source = api.request("GET", &api.scoped(&format!("/session/{id}/message")), None)?;
    let messages = source
        .as_array()
        .ok_or_else(|| DomainError::internal("OpenCode returned no messages"))?;
    let next = exclusive_boundary(messages, last)?;
    let body = next.map_or(json!({}), |id| json!({"messageID": id}));
    let created = api.request(
        "POST",
        &api.scoped(&format!("/session/{id}/fork")),
        Some(&body),
    )?;
    let new_id = created["id"]
        .as_str()
        .filter(|s| !s.is_empty() && *s != id)
        .ok_or_else(|| DomainError::internal("OpenCode did not return a new session ID"))?;
    let expected = messages
        .iter()
        .position(|m| m.pointer("/info/id").and_then(Value::as_str) == Some(last))
        .unwrap()
        + 1;
    let child = api.request(
        "GET",
        &api.scoped(&format!("/session/{new_id}/message")),
        None,
    )?;
    let child = child
        .as_array()
        .ok_or_else(|| DomainError::internal("OpenCode fork readback failed"))?;
    let content = |m: &Value| {
        json!([
            m.pointer("/info/role"),
            m["parts"].as_array().map(|parts| parts
                .iter()
                .filter_map(|p| p.get("text"))
                .collect::<Vec<_>>())
        ])
    };
    if child.len() != expected
        || !child
            .iter()
            .zip(&messages[..expected])
            .all(|(a, b)| content(a) == content(b))
    {
        return Err(DomainError::internal(format!("OpenCode created {new_id}, but its fork boundary could not be verified. Do not retry automatically.")));
    }
    Ok(json!({"sessionId": new_id}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keeps_answer_by_excluding_the_following_message() {
        let messages = vec![
            json!({"info":{"id":"u1","role":"user"}}),
            json!({"info":{"id":"a1","role":"assistant","finish":"stop"}}),
            json!({"info":{"id":"u2","role":"user"}}),
        ];
        assert_eq!(
            exclusive_boundary(&messages, "a1").unwrap(),
            Some("u2".into())
        );
        assert_eq!(exclusive_boundary(&messages[..2], "a1").unwrap(), None);
        assert!(exclusive_boundary(&messages, "missing").is_err());
        assert!(exclusive_boundary(&messages, "u1").is_err());
    }
}
