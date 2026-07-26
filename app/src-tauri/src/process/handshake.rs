//! 两个 sidecar 共用的握手校验。
//!
//! engine 与 ferry-runtime 的读循环差异很大（后者要做混流分发），但「health
//! 响应必须 ok、service 对得上、契约哈希一致」这三条是同一道关卡，放在这里
//! 保证改协议时只改一处。失败文案由调用方保留各自现状。

use serde_json::Value;

pub(crate) fn verify_handshake(
    resp: &Value,
    expected_service: &str,
    contract_hash: &str,
) -> Result<(), String> {
    if resp.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err("health 响应未返回 ok".to_owned());
    }
    if resp.pointer("/result/service").and_then(Value::as_str) != Some(expected_service) {
        return Err(format!("health 响应的 service 不是 {expected_service}"));
    }
    if resp
        .pointer("/result/contract_hash")
        .and_then(Value::as_str)
        != Some(contract_hash)
    {
        return Err("health 响应的契约哈希与本进程不一致".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn health(service: &str, hash: &str) -> Value {
        json!({"ok": true, "result": {"service": service, "contract_hash": hash}})
    }

    #[test]
    fn matching_service_and_contract_hash_pass() {
        assert!(verify_handshake(&health("engine", "hash-1"), "engine", "hash-1").is_ok());
        assert!(verify_handshake(
            &health("ferry-runtime", "hash-1"),
            "ferry-runtime",
            "hash-1"
        )
        .is_ok());
    }

    #[test]
    fn each_of_the_three_conditions_is_load_bearing() {
        assert!(verify_handshake(&health("runtime", "hash-1"), "engine", "hash-1").is_err());
        assert!(verify_handshake(&health("engine", "hash-2"), "engine", "hash-1").is_err());
        assert!(verify_handshake(
            &json!({"ok": false, "result": {"service": "engine", "contract_hash": "hash-1"}}),
            "engine",
            "hash-1",
        )
        .is_err());
    }

    #[test]
    fn missing_or_malformed_fields_are_rejected() {
        assert!(verify_handshake(&json!({}), "engine", "hash-1").is_err());
        assert!(verify_handshake(&json!({"ok": true}), "engine", "hash-1").is_err());
        assert!(verify_handshake(
            &json!({"ok": true, "result": {"service": "engine"}}),
            "engine",
            "hash-1",
        )
        .is_err());
        assert!(verify_handshake(
            &json!({"ok": "true", "result": {"service": "engine", "contract_hash": "hash-1"}}),
            "engine",
            "hash-1",
        )
        .is_err());
    }
}
