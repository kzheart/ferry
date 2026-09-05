//! 自包含、短生命周期的只读游标。摘要绑定参数和内容，位置从不充当权限凭证。
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::errors::{DomainError, DomainResult};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Position {
    pub message: usize,
    pub block: usize,
    pub offset: usize,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Cursor {
    v: u8,
    q: String,
    s: String,
    p: Position,
}

pub(super) fn digest(value: &Value) -> String {
    let bytes = serde_json::to_vec(value).expect("Value 可编码");
    // 128 bit 足以识别非敌对的快照变化；游标并非签名或授权凭证。
    URL_SAFE_NO_PAD.encode(&Sha256::digest(bytes)[..16])
}

pub(super) fn error(reason: &'static str, message: &str) -> DomainError {
    let mut error = DomainError::agent_request_invalid(message);
    error.params_mut().insert(
        "field".into(),
        json!(if reason == "byte_budget_too_small" {
            "max_bytes"
        } else {
            "cursor"
        }),
    );
    error.params_mut().insert("reason".into(), json!(reason));
    error.params_mut().insert(
        "recovery".into(),
        json!(if reason == "byte_budget_too_small" {
            "Increase max_bytes and retry with the same cursor."
        } else {
            "Restart the read without cursor using the intended parameters."
        }),
    );
    error
}

impl Cursor {
    pub fn new(binding: &str, snapshot: &str, position: Position) -> Self {
        Self {
            v: 1,
            q: binding.into(),
            s: snapshot.into(),
            p: position,
        }
    }

    pub fn encode(&self) -> String {
        format!(
            "frc_{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(self).expect("游标可编码"))
        )
    }

    pub fn decode(value: Option<&Value>) -> DomainResult<Option<Self>> {
        let Some(value) = value else { return Ok(None) };
        let invalid = || error("cursor_invalid", "cursor 必须是本次读取返回的有效游标");
        let raw = value
            .as_str()
            .filter(|value| value.len() <= 2048)
            .and_then(|value| value.strip_prefix("frc_"))
            .ok_or_else(invalid)?;
        let decoded = URL_SAFE_NO_PAD.decode(raw).map_err(|_| invalid())?;
        let cursor: Self = serde_json::from_slice(&decoded).map_err(|_| invalid())?;
        if cursor.v != 1 || cursor.q.len() != 22 || cursor.s.len() != 22 {
            return Err(invalid());
        }
        Ok(Some(cursor))
    }

    pub fn resume(
        cursor: Option<Self>,
        binding: &str,
        snapshot: &str,
        initial: Position,
    ) -> DomainResult<Position> {
        let Some(cursor) = cursor else {
            return Ok(initial);
        };
        if cursor.q != binding {
            return Err(error(
                "cursor_mismatch",
                "游标与读取参数不匹配，请保持原参数或重新读取",
            ));
        }
        if cursor.s != snapshot {
            return Err(error("cursor_stale", "会话内容已变化，请重新读取第一页"));
        }
        Ok(cursor.p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_rejects_invalid_mismatched_and_stale_inputs() {
        for bad in [
            Value::Null,
            json!(0),
            json!(""),
            json!("frc_%%%%"),
            json!("frc_e30"),
        ] {
            assert_eq!(
                Cursor::decode(Some(&bad)).err().unwrap().params()["reason"],
                "cursor_invalid"
            );
        }
        let query = digest(&json!("query"));
        let snapshot = digest(&json!("snapshot"));
        let encoded = json!(Cursor::new(
            &query,
            &snapshot,
            Position {
                message: 2,
                block: 3,
                offset: 4
            }
        )
        .encode());
        let resumed = Cursor::resume(
            Cursor::decode(Some(&encoded)).unwrap(),
            &query,
            &snapshot,
            Position::default(),
        )
        .unwrap();
        assert_eq!(
            resumed,
            Position {
                message: 2,
                block: 3,
                offset: 4
            }
        );
        assert_eq!(
            Cursor::resume(
                Cursor::decode(Some(&encoded)).unwrap(),
                "different",
                &snapshot,
                Position::default()
            )
            .unwrap_err()
            .params()["reason"],
            "cursor_mismatch"
        );
        assert_eq!(
            Cursor::resume(
                Cursor::decode(Some(&encoded)).unwrap(),
                &query,
                "different",
                Position::default()
            )
            .unwrap_err()
            .params()["reason"],
            "cursor_stale"
        );
    }
}
