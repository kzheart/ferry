//! Operation 输入的严格形状校验与规范化。
//!
//! `validate_json_shape` / `validate_agent_edit_ops` 只有一份实现（住在
//! `crate::sessions::safety`），本模块直接复用，不自带副本。

use serde_json::{Map, Value};

use crate::errors::DomainError;
use crate::operations::types::{AssistantReply, EngineResult};
use crate::sessions::safety::{validate_agent_edit_ops, validate_json_shape};
use crate::storage::database::canonical_json;

/// `json.loads(canonical_json(value))`：既排序键又做一次深拷贝定型。
fn canonicalized(value: &Value) -> EngineResult<Value> {
    let text = canonical_json(value)?;
    serde_json::from_str(&text)
        .map_err(|error| crate::operations::types::EngineError::value_error(error.to_string()))
}

fn unknown_fields(value: &Map<String, Value>, allowed: &[&str]) -> Vec<String> {
    let mut unknown: Vec<String> = value
        .keys()
        .filter(|key| !allowed.contains(&key.as_str()))
        .cloned()
        .collect();
    unknown.sort_unstable();
    unknown
}

fn unknown_field_error(message: &str, fields: Vec<String>) -> DomainError {
    let mut params = Map::new();
    params.insert(
        "fields".into(),
        Value::Array(fields.into_iter().map(Value::from).collect()),
    );
    DomainError::new(
        "agent.request_invalid",
        "AgentRequestError",
        message,
        params,
    )
}

fn request_error(message: impl Into<String>) -> DomainError {
    DomainError::agent_request_invalid(message)
}

/// 非空且长度在 `[1, limit]` 的字符串。
fn bounded_string(value: Option<&Value>, limit: usize) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .filter(|text| (1..=limit).contains(&text.chars().count()))
}

/// Python 的 `any(ord(ch) < floor for ch in ref)` 取反。
fn all_chars_at_least(text: &str, floor: u32) -> bool {
    text.chars().all(|character| character as u32 >= floor)
}

/// 严格整数（bool 不是整数）。
fn strict_int(value: &Value) -> Option<i64> {
    if value.is_boolean() {
        return None;
    }
    value.as_i64()
}

// ---------------------------------------------------------------------------
// edit
// ---------------------------------------------------------------------------

pub fn validate_edit_input(value: &Value) -> EngineResult<Value> {
    let object = value
        .as_object()
        .ok_or_else(|| request_error("operation input 必须是 object"))?;
    let allowed = ["kind", "tool", "ref", "ops"];
    let unknown = unknown_fields(object, &allowed);
    if !unknown.is_empty() {
        return Err(unknown_field_error("edit operation 包含未知字段", unknown).into());
    }
    let tool = object
        .get("tool")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| request_error("operation tool 非法"))?;
    let reference = object
        .get("ref")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| request_error("operation ref 非法"))?;
    let ops = validate_ops(object.get("ops").unwrap_or(&Value::Null))?;
    let ops = Value::Array(ops);
    if canonical_json(&ops)?.len() > 64 * 1024 {
        return Err(request_error("ops 超过 64 KiB").into());
    }
    let mut result = Map::new();
    result.insert("kind".into(), Value::from("edit"));
    result.insert("tool".into(), Value::from(tool));
    result.insert("ref".into(), Value::from(reference));
    result.insert("ops".into(), ops);
    canonicalized(&Value::Object(result))
}

// ---------------------------------------------------------------------------
// migration
// ---------------------------------------------------------------------------

pub fn validate_migration_input(value: &Value, adapters: &[String]) -> EngineResult<Value> {
    let object = value
        .as_object()
        .ok_or_else(|| request_error("operation input 必须是 object"))?;
    let allowed = ["kind", "source_tool", "ref", "target_tool", "max_turn"];
    let unknown = unknown_fields(object, &allowed);
    if !unknown.is_empty() {
        return Err(unknown_field_error("migration operation 包含未知字段", unknown).into());
    }
    let source_tool = bounded_string(object.get("source_tool"), 64)
        .ok_or_else(|| request_error("migration source_tool 非法"))?;
    let target_tool = bounded_string(object.get("target_tool"), 64)
        .ok_or_else(|| request_error("migration target_tool 非法"))?;
    let known = |tool: &str| adapters.iter().any(|declared| declared == tool);
    if !known(source_tool) || !known(target_tool) {
        return Err(request_error("migration Agent 非法").into());
    }
    if source_tool == target_tool {
        return Err(request_error("migration 源和目标不能相同").into());
    }
    let reference = bounded_string(object.get("ref"), 512)
        .filter(|text| all_chars_at_least(text, 33))
        .ok_or_else(|| request_error("migration ref 非法"))?;
    let max_turn = match object.get("max_turn") {
        None | Some(Value::Null) => None,
        Some(raw) => Some(
            strict_int(raw)
                .filter(|turn| (1..=1_000_000).contains(turn))
                .ok_or_else(|| request_error("migration max_turn 非法"))?,
        ),
    };
    let mut result = Map::new();
    result.insert("kind".into(), Value::from("migration"));
    result.insert("source_tool".into(), Value::from(source_tool));
    result.insert("ref".into(), Value::from(reference));
    result.insert("target_tool".into(), Value::from(target_tool));
    if let Some(max_turn) = max_turn {
        result.insert("max_turn".into(), Value::from(max_turn));
    }
    canonicalized(&Value::Object(result))
}

// ---------------------------------------------------------------------------
// metadata
// ---------------------------------------------------------------------------

pub fn validate_metadata_input(value: &Value) -> EngineResult<Value> {
    let object = value
        .as_object()
        .ok_or_else(|| request_error("operation input 必须是 object"))?;
    let allowed = ["kind", "tool", "ref", "patch"];
    let unknown = unknown_fields(object, &allowed);
    if !unknown.is_empty() {
        return Err(unknown_field_error("metadata operation 包含未知字段", unknown).into());
    }
    let tool = bounded_string(object.get("tool"), 64)
        .ok_or_else(|| request_error("metadata tool 非法"))?;
    let reference = bounded_string(object.get("ref"), 512)
        .filter(|text| all_chars_at_least(text, 33))
        .ok_or_else(|| request_error("metadata ref 非法"))?;
    let allowed_fields = ["name", "pinned", "archived", "tags"];
    let patch = object
        .get("patch")
        .and_then(Value::as_object)
        .filter(|patch| !patch.is_empty())
        .filter(|patch| {
            patch
                .keys()
                .all(|key| allowed_fields.contains(&key.as_str()))
        })
        .ok_or_else(|| request_error("metadata patch 字段非法"))?;
    validate_json_shape(&Value::Object(patch.clone()), 3, 50)?;
    if let Some(name) = patch.get("name") {
        if name.as_str().is_none_or(|text| text.chars().count() > 200) {
            return Err(request_error("metadata name 非法").into());
        }
    }
    for field in ["pinned", "archived"] {
        if patch.get(field).is_some_and(|value| !value.is_boolean()) {
            return Err(request_error(format!("metadata {field} 必须是 boolean")).into());
        }
    }
    if let Some(tags) = patch.get("tags") {
        let valid = tags.as_array().is_some_and(|tags| {
            tags.len() <= 20
                && tags.iter().all(|tag| {
                    tag.as_str()
                        .is_some_and(|text| (1..=64).contains(&text.chars().count()))
                })
        });
        if !valid {
            return Err(request_error("metadata tags 非法").into());
        }
    }
    if canonical_json(&Value::Object(patch.clone()))?.len() > 4096 {
        return Err(request_error("metadata patch 超过 4 KiB").into());
    }

    let mut result = Map::new();
    result.insert("kind".into(), Value::from("metadata"));
    result.insert("tool".into(), Value::from(tool));
    result.insert("ref".into(), Value::from(reference));
    result.insert("patch".into(), Value::Object(patch.clone()));
    canonicalized(&Value::Object(result))
}

// ---------------------------------------------------------------------------
// ops
// ---------------------------------------------------------------------------

/// `replace-assistant-reply` 的去重键：`(type(turn).__name__, turn)` 复合键，
/// 所以 `turn=1` 与 `turn="1"` 不算重复。
#[derive(PartialEq)]
enum TurnKey {
    Int(i64),
    Str(String),
}

pub fn validate_ops(ops: &Value) -> EngineResult<Vec<Value>> {
    let items = ops
        .as_array()
        .filter(|items| !items.is_empty() && items.len() <= 50)
        .ok_or_else(|| request_error("ops 必须是 1 到 50 项的数组"))?;
    validate_json_shape(ops, 8, 2000)?;

    let mut ordinary: Vec<Value> = Vec::new();
    let mut normalized: Vec<Value> = Vec::new();
    let mut replaced_turns: Vec<TurnKey> = Vec::new();
    for operation in items {
        let object = operation
            .as_object()
            .ok_or_else(|| request_error("每个 edit op 必须是 object"))?;
        if object.get("op").and_then(Value::as_str) != Some("replace-assistant-reply") {
            ordinary.push(operation.clone());
            normalized.push(operation.clone());
            continue;
        }
        let expected = ["op", "turn", "reply"];
        if object.len() != expected.len() || !expected.iter().all(|key| object.contains_key(*key)) {
            return Err(request_error("replace-assistant-reply 参数非法").into());
        }
        let turn = &object["turn"];
        let key = match turn {
            Value::Bool(_) => None,
            Value::Number(_) => strict_int(turn)
                .filter(|value| *value >= 1)
                .map(TurnKey::Int),
            Value::String(text) => Some(text)
                .filter(|text| (1..=512).contains(&text.chars().count()))
                .map(|text| TurnKey::Str(text.clone())),
            _ => None,
        }
        .ok_or_else(|| request_error("replace-assistant-reply turn 参数非法"))?;
        let reply = AssistantReply::from_value(&object["reply"])?;
        if replaced_turns.contains(&key) {
            let mut params = Map::new();
            params.insert("field".into(), Value::from("ops.turn"));
            return Err(DomainError::new(
                "agent.request_invalid",
                "AgentRequestError",
                "同一轮次不能在一次编辑中重复替换",
                params,
            )
            .into());
        }
        replaced_turns.push(key);
        let mut item = Map::new();
        item.insert("op".into(), Value::from("replace-assistant-reply"));
        item.insert("turn".into(), turn.clone());
        item.insert("reply".into(), reply.to_value());
        normalized.push(Value::Object(item));
    }
    if !ordinary.is_empty() {
        validate_agent_edit_ops(Some(&Value::Array(ordinary)))?;
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ADAPTERS: &[&str] = &["claude", "codex", "opencode", "pi", "grok", "cursor"];

    fn adapters() -> Vec<String> {
        ADAPTERS.iter().map(|id| (*id).to_string()).collect()
    }

    #[test]
    fn edit_input_is_frozen_and_canonicalized() {
        let value = json!({
            "kind": "edit",
            "tool": "claude",
            "ref": "fsr_abcdefgh",
            "ops": [{"op": "delete-turn", "turn": 1}],
        });
        let frozen = validate_edit_input(&value).unwrap();
        // canonical 化后 key 有序。
        assert_eq!(
            canonical_json(&frozen).unwrap(),
            r#"{"kind":"edit","ops":[{"op":"delete-turn","turn":1}],"ref":"fsr_abcdefgh","tool":"claude"}"#
        );
    }

    #[test]
    fn edit_input_rejects_unknown_fields_with_sorted_names() {
        let error = validate_edit_input(&json!({
            "kind": "edit", "tool": "claude", "ref": "r",
            "ops": [{"op": "delete-turn", "turn": 1}],
            "zeta": 1, "alpha": 2, "probe": false,
        }))
        .unwrap_err();
        assert_eq!(error.message(), "edit operation 包含未知字段");
        let params = error.as_domain().unwrap().params();
        assert_eq!(params["fields"], json!(["alpha", "probe", "zeta"]));
    }

    #[test]
    fn edit_input_rejects_empty_identifiers() {
        for (value, expected) in [
            (
                json!({"kind": "edit", "tool": "", "ref": "r", "ops": []}),
                "operation tool 非法",
            ),
            (
                json!({"kind": "edit", "tool": "claude", "ref": "", "ops": []}),
                "operation ref 非法",
            ),
        ] {
            assert_eq!(validate_edit_input(&value).unwrap_err().message(), expected);
        }
    }

    #[test]
    fn migration_input_matches_the_python_limit_table() {
        let base = json!({
            "kind": "migration",
            "source_tool": "claude",
            "ref": "fsr_abcdefgh",
            "target_tool": "opencode",
        });
        assert!(validate_migration_input(&base, &adapters()).is_ok());

        let cases: &[(Value, &str)] = &[
            (
                json!({"unexpected": true}),
                "migration operation 包含未知字段",
            ),
            (json!({"source_tool": ""}), "migration source_tool 非法"),
            (json!({"target_tool": ""}), "migration target_tool 非法"),
            (json!({"target_tool": "nope"}), "migration Agent 非法"),
            (
                json!({"target_tool": "claude"}),
                "migration 源和目标不能相同",
            ),
            (json!({"ref": "with space"}), "migration ref 非法"),
            (json!({"max_turn": true}), "migration max_turn 非法"),
            (json!({"max_turn": 0}), "migration max_turn 非法"),
            (json!({"max_turn": 1_000_001}), "migration max_turn 非法"),
            // 已移除的探针字段当未知字段拒掉。
            (json!({"probe": false}), "migration operation 包含未知字段"),
            (
                json!({"probe_model": "model"}),
                "migration operation 包含未知字段",
            ),
        ];
        for (patch, expected) in cases {
            let mut value = base.clone();
            for (key, item) in patch.as_object().unwrap() {
                value[key.as_str()] = item.clone();
            }
            assert_eq!(
                validate_migration_input(&value, &adapters())
                    .unwrap_err()
                    .message(),
                *expected,
                "patch={patch}"
            );
        }
    }

    #[test]
    fn migration_optional_fields_are_omitted_when_absent() {
        let frozen = validate_migration_input(
            &json!({
                "kind": "migration", "source_tool": "claude",
                "ref": "fsr_abcdefgh", "target_tool": "opencode",
            }),
            &adapters(),
        )
        .unwrap();
        assert!(frozen.get("max_turn").is_none());
        assert!(frozen.get("probe_model").is_none());
        assert!(frozen.get("probe").is_none());
    }

    #[test]
    fn metadata_patch_limits_are_enforced() {
        let base = json!({"kind": "metadata", "tool": "claude", "ref": "fsr_abcdefgh"});
        let cases: &[(Value, &str)] = &[
            (json!({}), "metadata patch 字段非法"),
            (json!({"unknown": true}), "metadata patch 字段非法"),
            (json!({"pinned": "yes"}), "metadata pinned 必须是 boolean"),
            (json!({"archived": 1}), "metadata archived 必须是 boolean"),
            (json!({"tags": [""]}), "metadata tags 非法"),
            (json!({"tags": "a"}), "metadata tags 非法"),
            (json!({"name": 1}), "metadata name 非法"),
        ];
        for (patch, expected) in cases {
            let mut value = base.clone();
            value["patch"] = patch.clone();
            assert_eq!(
                validate_metadata_input(&value).unwrap_err().message(),
                *expected,
                "patch={patch}"
            );
        }
        let too_many: Vec<Value> = (0..21)
            .map(|index| Value::from(format!("t{index}")))
            .collect();
        let mut value = base.clone();
        value["patch"] = json!({"tags": too_many});
        assert_eq!(
            validate_metadata_input(&value).unwrap_err().message(),
            "metadata tags 非法"
        );
        let mut value = base;
        value["patch"] = json!({"name": "x".repeat(201)});
        assert_eq!(
            validate_metadata_input(&value).unwrap_err().message(),
            "metadata name 非法"
        );
    }

    #[test]
    fn replace_reply_dedup_uses_a_type_and_value_composite_key() {
        // 同类型同值 → 重复；类型不同（int 1 vs str "1"）→ 不重复。
        let reply = json!({"items": [{"kind": "text", "text": "x"}]});
        let duplicated = json!([
            {"op": "replace-assistant-reply", "turn": 1, "reply": reply},
            {"op": "replace-assistant-reply", "turn": 1, "reply": reply},
        ]);
        let error = validate_ops(&duplicated).unwrap_err();
        assert_eq!(error.message(), "同一轮次不能在一次编辑中重复替换");
        assert_eq!(
            error.as_domain().unwrap().params()["field"],
            json!("ops.turn")
        );

        let mixed = json!([
            {"op": "replace-assistant-reply", "turn": 1, "reply": reply},
            {"op": "replace-assistant-reply", "turn": "1", "reply": reply},
        ]);
        assert_eq!(validate_ops(&mixed).unwrap().len(), 2);
    }

    #[test]
    fn replace_reply_shape_errors_match_python() {
        let reply = json!({"items": [{"kind": "text", "text": "x"}]});
        let cases: &[(Value, &str, &str)] = &[
            (
                json!([{"op": "replace-assistant-reply", "turn": 0, "reply": reply}]),
                "AgentRequestError",
                "replace-assistant-reply turn 参数非法",
            ),
            (
                json!([{"op": "replace-assistant-reply", "turn": true, "reply": reply}]),
                "AgentRequestError",
                "replace-assistant-reply turn 参数非法",
            ),
            (
                json!([{"op": "replace-assistant-reply", "turn": 1, "reply": {"items": []}}]),
                "InvalidReplyError",
                "reply.items 必须是非空数组",
            ),
            (
                json!([{
                    "op": "replace-assistant-reply", "turn": 1,
                    "reply": reply, "unexpected": true,
                }]),
                "AgentRequestError",
                "replace-assistant-reply 参数非法",
            ),
        ];
        for (ops, error_type, message) in cases {
            let error = validate_ops(ops).unwrap_err();
            assert_eq!(error.error_type(), *error_type, "ops={ops}");
            assert_eq!(error.message(), *message, "ops={ops}");
        }
    }

    #[test]
    fn agent_edit_ops_whitelist_only_delete_turn_and_rewrite() {
        let cases: &[(Value, &str)] = &[
            (
                json!([{"op": "nope"}]),
                "Agent edit 仅允许 delete-turn/rewrite",
            ),
            (
                json!([{"op": "delete-turn", "turn": 0}]),
                "delete-turn 参数非法",
            ),
            (
                json!([{"op": "delete-turn", "turn": true}]),
                "delete-turn 参数非法",
            ),
            (
                json!([{"op": "delete-turn", "turn": 1, "extra": 1}]),
                "delete-turn 参数非法",
            ),
            (
                json!([{"op": "rewrite", "locator": "a"}]),
                "rewrite 参数非法",
            ),
            (
                json!([{"op": "rewrite", "locator": "", "text": "t"}]),
                "rewrite locator/text 超出范围",
            ),
            (
                json!([{"op": "rewrite", "locator": "a", "text": ""}]),
                "rewrite locator/text 超出范围",
            ),
            (
                json!([
                    {"op": "rewrite", "locator": "a", "text": "t"},
                    {"op": "rewrite", "locator": "a", "text": "u"},
                ]),
                "同一消息不能在一次编辑中重复改写",
            ),
        ];
        for (ops, expected) in cases {
            assert_eq!(
                validate_ops(ops).unwrap_err().message(),
                *expected,
                "ops={ops}"
            );
        }
    }

    #[test]
    fn ops_array_bounds_are_one_to_fifty() {
        assert_eq!(
            validate_ops(&json!([])).unwrap_err().message(),
            "ops 必须是 1 到 50 项的数组"
        );
        let many: Vec<Value> = (0..51)
            .map(|index| json!({"op": "delete-turn", "turn": index + 1}))
            .collect();
        assert_eq!(
            validate_ops(&Value::Array(many)).unwrap_err().message(),
            "ops 必须是 1 到 50 项的数组"
        );
    }

    #[test]
    fn json_shape_limits_depth_and_node_count() {
        let mut deep = json!(1);
        for _ in 0..9 {
            deep = Value::Array(vec![deep]);
        }
        assert_eq!(
            validate_json_shape(&deep, 8, 2000).unwrap_err().message(),
            "JSON 结构过深或项目过多"
        );
        let wide = Value::Array((0..2001).map(Value::from).collect());
        assert_eq!(
            validate_json_shape(&wide, 8, 2000).unwrap_err().message(),
            "JSON 结构过深或项目过多"
        );
        let mut long_key = Map::new();
        long_key.insert("k".repeat(129), Value::from(1));
        assert_eq!(
            validate_json_shape(&Value::Object(long_key), 8, 2000)
                .unwrap_err()
                .message(),
            "JSON key 必须是不超过 128 字符的字符串"
        );
    }
}
