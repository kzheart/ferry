//! Claude 原生会话的唯一轮次解析与编辑编解码。
//!
//! reader DTO、delete-turn、rewrite、replace-reply 全部消费本模块的
//! [`TURN_INDEX`]；轮次定义：非 sidechain、非 isMeta、非 tool_result 载体、
//! 且含可见内容的用户消息，到下一条这样的消息之前。

use std::collections::BTreeSet;

use serde_json::{Map, Value};

use crate::adapters::shared::codec::{NativeEditCodec, TurnIndex, TurnSpan};
use crate::adapters::shared::dialect::python_str;
use crate::adapters::shared::editing::{
    is_spawn_name, reject_replacement_spawn, reject_target_spawn, replace_at_first,
};
use crate::errors::{DomainError, DomainResult};
use crate::events::Event;

use super::editing::{relink, token_urlsafe, utc_iso_now_micros, uuid4};

/// `_record` 不从模板继承的键；其余字段一律深拷贝保留。
const TEMPLATE_EXCLUDED: &[&str] = &[
    "uuid",
    "parentUuid",
    "promptId",
    "type",
    "message",
    "toolUseResult",
    "timestamp",
];

fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|float| float != 0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(entries)) => !entries.is_empty(),
    }
}

fn record_type(record: &Value) -> Option<&str> {
    record.get("type").and_then(Value::as_str)
}

fn content_of(record: &Value) -> Option<&Value> {
    record
        .get("message")
        .and_then(|message| message.get("content"))
}

/// 用户消息是否含可见内容。
fn visible_user_content(content: Option<&Value>) -> bool {
    match content {
        Some(Value::String(_)) => true,
        Some(Value::Array(items)) => items.iter().any(|item| {
            let Some(entry) = item.as_object() else {
                return false;
            };
            match entry.get("type").and_then(Value::as_str) {
                Some("text" | "tool_use") => true,
                Some("thinking") => entry
                    .get("thinking")
                    .and_then(Value::as_str)
                    .is_some_and(|text| !text.trim().is_empty()),
                _ => false,
            }
        }),
        _ => false,
    }
}

/// 这条消息是不是 tool_result 的载体。
fn is_tool_carrier(content: Option<&Value>) -> bool {
    content.and_then(Value::as_array).is_some_and(|items| {
        items
            .iter()
            .any(|item| item.get("type").and_then(Value::as_str) == Some("tool_result"))
    })
}

/// 「AI 回复」记录：非 sidechain 的 assistant，或承载 tool_result 的 user。
fn is_reply_record(record: &Value) -> bool {
    if truthy(record.get("isSidechain")) {
        return false;
    }
    if record_type(record) == Some("assistant") {
        return true;
    }
    record_type(record) == Some("user") && is_tool_carrier(content_of(record))
}

/// Claude 的轮次索引。
pub struct ClaudeTurnIndex;

impl TurnIndex for ClaudeTurnIndex {
    type Document = [Value];
    type VisibleMessage = (usize, Value);

    fn visible_messages(&self, records: &[Value]) -> Vec<(usize, Value)> {
        let mut out = Vec::new();
        for (index, record) in records.iter().enumerate() {
            if truthy(record.get("isSidechain"))
                || truthy(record.get("isMeta"))
                || !matches!(record_type(record), Some("user" | "assistant"))
            {
                continue;
            }
            if record_type(record) == Some("user") && is_tool_carrier(content_of(record)) {
                continue;
            }
            out.push((index, record.clone()));
        }
        out
    }

    fn turns(&self, records: &[Value]) -> Vec<TurnSpan> {
        let mut starts = Vec::new();
        for (index, record) in records.iter().enumerate() {
            if record_type(record) != Some("user")
                || truthy(record.get("isSidechain"))
                || truthy(record.get("isMeta"))
            {
                continue;
            }
            let content = content_of(record);
            if !is_tool_carrier(content) && visible_user_content(content) {
                starts.push(index);
            }
        }
        starts
            .iter()
            .enumerate()
            .map(|(position, start)| {
                let end = starts.get(position + 1).copied().unwrap_or(records.len());
                let locator = records[*start]
                    .get("uuid")
                    .filter(|value| truthy(Some(value)))
                    .map_or_else(|| format!("record:{start}"), python_str);
                TurnSpan::new(position + 1, locator, *start, end)
            })
            .collect()
    }
}

/// Claude 的编辑编解码。
pub struct ClaudeEditCodec;

impl ClaudeEditCodec {
    /// 从模板派生一条新记录；除排除键外的字段一律保留。
    fn record(
        &self,
        template: &Value,
        record_type: &str,
        parent: Option<&str>,
        content: Vec<Value>,
        stop_reason: Option<&str>,
    ) -> Value {
        let mut record = Map::new();
        if let Some(entries) = template.as_object() {
            for (key, value) in entries {
                if !TEMPLATE_EXCLUDED.contains(&key.as_str()) {
                    record.insert(key.clone(), value.clone());
                }
            }
        }
        record.insert("uuid".into(), Value::from(uuid4()));
        record.insert("parentUuid".into(), parent.map_or(Value::Null, Value::from));
        record.insert("type".into(), Value::from(record_type));
        record.insert("isSidechain".into(), Value::Bool(false));

        let mut message = Map::new();
        message.insert(
            "role".into(),
            Value::from(if record_type == "assistant" {
                "assistant"
            } else {
                "user"
            }),
        );
        message.insert("content".into(), Value::Array(content));
        if record_type == "assistant" {
            message.insert("type".into(), Value::from("message"));
            message.insert(
                "stop_reason".into(),
                Value::from(stop_reason.unwrap_or("end_turn")),
            );
        }
        record.insert("message".into(), Value::Object(message));
        record.insert("timestamp".into(), Value::from(utc_iso_now_micros()));
        Value::Object(record)
    }
}

impl NativeEditCodec for ClaudeEditCodec {
    type Document = Vec<Value>;
    type Reply = Value;
    type Change = Event;

    fn replace_reply(
        &self,
        records: &mut Vec<Value>,
        span: &TurnSpan,
        reply: &Value,
    ) -> DomainResult<Vec<Event>> {
        let old: Vec<Value> = records[span.start + 1..span.end].to_vec();
        reject_replacement_spawn(reply)?;
        let targets_spawn = old
            .iter()
            .filter(|record| is_reply_record(record))
            .any(|record| {
                content_of(record)
                    .and_then(Value::as_array)
                    .is_some_and(|items| {
                        items.iter().any(|item| {
                            item.get("type").and_then(Value::as_str) == Some("tool_use")
                                && is_spawn_name(item.get("name"))
                        })
                    })
            });
        if targets_spawn {
            return Err(reject_target_spawn("claude"));
        }
        let removed_ids: BTreeSet<String> = old
            .iter()
            .filter(|record| is_reply_record(record))
            .filter_map(|record| record.get("uuid").and_then(Value::as_str))
            .map(std::string::ToString::to_string)
            .collect();

        let user = records[span.start].clone();
        let template = old
            .iter()
            .find(|record| {
                record_type(record) == Some("assistant") && !truthy(record.get("isSidechain"))
            })
            .cloned()
            .unwrap_or(user.clone());
        let mut parent = user
            .get("uuid")
            .and_then(Value::as_str)
            .map(std::string::ToString::to_string);

        let items = reply
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut compiled: Vec<Value> = Vec::new();
        let mut content: Vec<Value> = Vec::new();
        for item in &items {
            if item.get("kind").and_then(Value::as_str) == Some("text") {
                let mut block = Map::new();
                block.insert("type".into(), Value::from("text"));
                block.insert(
                    "text".into(),
                    item.get("text").cloned().unwrap_or_else(|| Value::from("")),
                );
                content.push(Value::Object(block));
                continue;
            }
            let call_id = format!("toolu_{}", &token_urlsafe(18)[..24]);
            let mut block = Map::new();
            block.insert("type".into(), Value::from("tool_use"));
            block.insert("id".into(), Value::from(call_id.as_str()));
            block.insert(
                "name".into(),
                item.get("name").cloned().unwrap_or_else(|| Value::from("")),
            );
            block.insert(
                "input".into(),
                item.get("input").cloned().unwrap_or(Value::Null),
            );
            content.push(Value::Object(block));

            let call = self.record(
                &template,
                "assistant",
                parent.as_deref(),
                std::mem::take(&mut content),
                Some("tool_use"),
            );
            let call_uuid = call["uuid"].as_str().unwrap_or_default().to_string();
            let mut result_block = Map::new();
            result_block.insert("type".into(), Value::from("tool_result"));
            result_block.insert("tool_use_id".into(), Value::from(call_id.as_str()));
            result_block.insert(
                "content".into(),
                item.get("output")
                    .cloned()
                    .unwrap_or_else(|| Value::from("")),
            );
            let result = self.record(
                &template,
                "user",
                Some(&call_uuid),
                vec![Value::Object(result_block)],
                None,
            );
            parent = result["uuid"]
                .as_str()
                .map(std::string::ToString::to_string);
            compiled.push(call);
            compiled.push(result);
        }
        if !content.is_empty() {
            let final_record =
                self.record(&template, "assistant", parent.as_deref(), content, None);
            parent = final_record["uuid"]
                .as_str()
                .map(std::string::ToString::to_string);
            compiled.push(final_record);
        }

        let replacement = replace_at_first(&old, is_reply_record, &compiled);
        records.splice(span.start + 1..span.end, replacement);
        for record in records.iter_mut().skip(span.start + 1) {
            let hit = record
                .get("parentUuid")
                .and_then(Value::as_str)
                .is_some_and(|value| removed_ids.contains(value));
            if hit {
                if let Some(entries) = record.as_object_mut() {
                    entries.insert(
                        "parentUuid".into(),
                        parent.as_deref().map_or(Value::Null, Value::from),
                    );
                }
            }
        }

        let mut params = Map::new();
        params.insert("turn".into(), Value::from(span.ordinal as i64));
        params.insert("items".into(), Value::from(items.len() as i64));
        Ok(vec![Event::new("edit.reply_replaced", params)])
    }

    fn delete_turn(&self, records: &mut Vec<Value>, span: &TurnSpan) -> DomainResult<Vec<Event>> {
        let removed_uuids: BTreeSet<String> = records[span.start..span.end]
            .iter()
            .filter_map(|record| record.get("uuid").and_then(Value::as_str))
            .map(std::string::ToString::to_string)
            .collect();
        let mut kept: Vec<Value> = records[..span.start].to_vec();
        kept.extend_from_slice(&records[span.end..]);
        relink(&mut kept, &removed_uuids);
        *records = kept;
        let mut params = Map::new();
        params.insert("turn".into(), Value::from(span.ordinal as i64));
        Ok(vec![Event::new("edit.turn_deleted", params)])
    }

    fn rewrite_message(
        &self,
        records: &mut Vec<Value>,
        locator: &str,
        text: &str,
    ) -> DomainResult<Vec<Event>> {
        let record = records
            .iter_mut()
            .find(|record| record.get("uuid").and_then(Value::as_str) == Some(locator))
            .ok_or_else(|| {
                let mut params = Map::new();
                params.insert("locator".into(), Value::from(locator));
                DomainError::locator_stale(Some("Claude 消息定位符已失效，请刷新会话"), params)
            })?;

        let record_kind = record.get("type").cloned().unwrap_or(Value::Null);
        let Some(message) = record
            .get_mut("message")
            .and_then(Value::as_object_mut)
            .filter(|message| !message.is_empty())
        else {
            // Python 的 `record.get("message") or {}` 落到临时 dict：改写不会生效，
            // 后续一定走到 no-text 分支。
            let role = record_kind
                .as_str()
                .map(std::string::ToString::to_string)
                .unwrap_or_else(|| python_str(&record_kind));
            return Err(unsupported_rewrite(
                if role == "user" || role == "assistant" {
                    "no-text"
                } else {
                    &role
                },
            ));
        };
        let role = message
            .get("role")
            .filter(|role| truthy(Some(role)))
            .cloned()
            .unwrap_or(record_kind);
        let role_text = role
            .as_str()
            .map(std::string::ToString::to_string)
            .unwrap_or_else(|| python_str(&role));
        if role_text != "user" && role_text != "assistant" {
            return Err(unsupported_rewrite(&role_text));
        }
        match message.get("content") {
            Some(Value::String(_)) => {
                message.insert("content".into(), Value::from(text));
            }
            Some(Value::Array(items)) => {
                let items = items.clone();
                let first = items
                    .iter()
                    .position(|item| item.get("type").and_then(Value::as_str) == Some("text"))
                    .ok_or_else(|| unsupported_rewrite("no-text"))?;
                let mut rewritten: Vec<Value> = items
                    .into_iter()
                    .filter(|item| item.get("type").and_then(Value::as_str) != Some("text"))
                    .collect();
                let mut block = Map::new();
                block.insert("type".into(), Value::from("text"));
                block.insert("text".into(), Value::from(text));
                rewritten.insert(first.min(rewritten.len()), Value::Object(block));
                message.insert("content".into(), Value::Array(rewritten));
            }
            _ => return Err(unsupported_rewrite("no-text")),
        }
        let mut params = Map::new();
        params.insert("count".into(), Value::from(1));
        Ok(vec![Event::new("edit.message_rewritten", params)])
    }
}

fn unsupported_rewrite(mode: &str) -> DomainError {
    DomainError::operation_unsupported("claude", "rewrite", Some(mode))
}

/// 进程级单例，对齐 Python 的模块级 `TURN_INDEX` / `CODEC`。
pub static TURN_INDEX: ClaudeTurnIndex = ClaudeTurnIndex;
/// 见 [`TURN_INDEX`]。
pub static CODEC: ClaudeEditCodec = ClaudeEditCodec;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn conversation() -> Vec<Value> {
        vec![
            json!({"uuid": "u1", "parentUuid": null, "type": "user", "isSidechain": false,
                   "message": {"role": "user", "content": "first"}}),
            json!({"uuid": "a1", "parentUuid": "u1", "type": "assistant", "isSidechain": false,
                   "message": {"type": "message", "role": "assistant",
                               "content": [{"type": "text", "text": "reply"}],
                               "stop_reason": "end_turn"}}),
            json!({"uuid": "u2", "parentUuid": "a1", "type": "user", "isSidechain": false,
                   "message": {"role": "user", "content": "second"}}),
            json!({"uuid": "a2", "parentUuid": "u2", "type": "assistant", "isSidechain": false,
                   "message": {"type": "message", "role": "assistant",
                               "content": [{"type": "text", "text": "done"}]}}),
        ]
    }

    #[test]
    fn turns_start_at_visible_user_messages() {
        let spans = TURN_INDEX.turns(&conversation());
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0], TurnSpan::new(1, "u1", 0, 2));
        assert_eq!(spans[1], TurnSpan::new(2, "u2", 2, 4));
    }

    #[test]
    fn tool_carriers_meta_and_sidechains_never_start_a_turn() {
        let records = vec![
            json!({"uuid": "m", "type": "user", "isMeta": true,
                   "message": {"role": "user", "content": "meta"}}),
            json!({"uuid": "s", "type": "user", "isSidechain": true,
                   "message": {"role": "user", "content": "side"}}),
            json!({"uuid": "t", "type": "user",
                   "message": {"role": "user",
                               "content": [{"type": "tool_result", "tool_use_id": "x"}]}}),
            json!({"uuid": "e", "type": "user", "message": {"role": "user", "content": []}}),
        ];
        assert!(TURN_INDEX.turns(&records).is_empty());
        // visible_messages 只按 sidechain/isMeta/type/tool_carrier 过滤，
        // 「内容不可见」不在它的判定里：空 content 的普通用户消息仍然可见。
        let visible = TURN_INDEX.visible_messages(&records);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].0, 3);
    }

    #[test]
    fn missing_uuid_falls_back_to_a_positional_locator() {
        let records = vec![json!({"type": "user", "message": {"role": "user", "content": "hi"}})];
        assert_eq!(TURN_INDEX.turns(&records)[0].locator, "record:0");
    }

    #[test]
    fn delete_turn_drops_the_span_and_relinks_the_chain() {
        let mut records = conversation();
        let spans = TURN_INDEX.turns(&records);
        let changes = CODEC.delete_turn(&mut records, &spans[0]).unwrap();
        assert_eq!(changes[0].code, "edit.turn_deleted");
        assert_eq!(changes[0].params["turn"], json!(1));
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["uuid"], json!("u2"));
        assert_eq!(records[0]["parentUuid"], json!(null));
    }

    #[test]
    fn replace_reply_rebuilds_the_chain_and_pairs_tools() {
        let mut records = conversation();
        let spans = TURN_INDEX.turns(&records);
        let reply = json!({"items": [
            {"kind": "text", "text": "thinking out loud"},
            {"kind": "tool", "name": "Bash", "input": {"command": "ls"}, "output": "a\nb"},
            {"kind": "text", "text": "done"}
        ]});
        let changes = CODEC
            .replace_reply(&mut records, &spans[0], &reply)
            .unwrap();
        assert_eq!(changes[0].code, "edit.reply_replaced");
        assert_eq!(changes[0].params["items"], json!(3));

        // u1 + (tool_use assistant, tool_result user, 收尾 assistant) + u2 + a2
        assert_eq!(records.len(), 6);
        assert_eq!(records[1]["type"], json!("assistant"));
        let call = &records[1]["message"]["content"];
        assert_eq!(call[0]["type"], json!("text"));
        assert_eq!(call[1]["type"], json!("tool_use"));
        let call_id = call[1]["id"].as_str().unwrap().to_string();
        assert!(call_id.starts_with("toolu_") && call_id.len() == 6 + 24);
        assert_eq!(records[1]["message"]["stop_reason"], json!("tool_use"));
        assert_eq!(
            records[2]["message"]["content"][0]["tool_use_id"],
            json!(call_id)
        );
        assert_eq!(records[3]["message"]["content"][0]["text"], json!("done"));
        // 链首指向被替换轮的用户消息，末尾接回后续轮次。
        assert_eq!(records[1]["parentUuid"], json!("u1"));
        assert_eq!(records[4]["parentUuid"], records[3]["uuid"]);
        // 模板字段（这里是 a1 的 role/type 之外的键）不会凭空丢失。
        assert_eq!(records[1]["isSidechain"], json!(false));
    }

    #[test]
    fn replace_reply_rejects_spawn_tools_on_both_sides() {
        let mut records = conversation();
        let spans = TURN_INDEX.turns(&records);
        let spawn_reply = json!({"items": [
            {"kind": "tool", "name": "Task", "input": {}, "output": ""}
        ]});
        assert_eq!(
            CODEC
                .replace_reply(&mut records, &spans[0], &spawn_reply)
                .unwrap_err()
                .code,
            "edit.subagent_not_supported"
        );

        let mut spawning = conversation();
        spawning[1] = json!({"uuid": "a1", "parentUuid": "u1", "type": "assistant",
                             "isSidechain": false,
                             "message": {"role": "assistant", "content": [
                                 {"type": "tool_use", "id": "t", "name": "Agent", "input": {}}]}});
        let spans = TURN_INDEX.turns(&spawning);
        let plain = json!({"items": [{"kind": "text", "text": "hi"}]});
        let error = CODEC
            .replace_reply(&mut spawning, &spans[0], &plain)
            .unwrap_err();
        assert_eq!(error.code, "edit.subagent_not_supported");
        assert_eq!(error.params()["tool"], json!("claude"));
    }

    #[test]
    fn rewrite_replaces_string_and_first_text_block() {
        let mut records = conversation();
        CODEC
            .rewrite_message(&mut records, "u1", "changed")
            .unwrap();
        assert_eq!(records[0]["message"]["content"], json!("changed"));

        let changes = CODEC.rewrite_message(&mut records, "a1", "edited").unwrap();
        assert_eq!(changes[0].code, "edit.message_rewritten");
        assert_eq!(
            records[1]["message"]["content"],
            json!([{"type": "text", "text": "edited"}])
        );
    }

    #[test]
    fn rewrite_keeps_non_text_blocks_and_reports_stale_locators() {
        let mut records = vec![json!({
            "uuid": "a", "type": "assistant",
            "message": {"role": "assistant", "content": [
                {"type": "tool_use", "id": "t", "name": "Bash", "input": {}},
                {"type": "text", "text": "old"}]}
        })];
        CODEC.rewrite_message(&mut records, "a", "new").unwrap();
        assert_eq!(
            records[0]["message"]["content"],
            json!([{"type": "tool_use", "id": "t", "name": "Bash", "input": {}},
                   {"type": "text", "text": "new"}])
        );

        let error = CODEC
            .rewrite_message(&mut records, "gone", "x")
            .unwrap_err();
        assert_eq!(error.code, "session.locator_stale");
        assert_eq!(error.message(), "Claude 消息定位符已失效，请刷新会话");

        let mut without_text = vec![json!({
            "uuid": "b", "type": "user",
            "message": {"role": "user", "content": [{"type": "image"}]}
        })];
        let error = CODEC
            .rewrite_message(&mut without_text, "b", "x")
            .unwrap_err();
        assert_eq!(error.code, "edit.operation_unsupported");
    }
}
