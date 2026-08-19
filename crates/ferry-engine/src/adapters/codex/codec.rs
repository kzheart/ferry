//! Codex 原生 rollout 的唯一轮次解析与编辑编解码。
//!
//! 轮次定义：非环境上下文前缀、正文非空的用户 message 到下一条之前。

use serde_json::{Map, Value};

use crate::adapters::shared::codec::{NativeEditCodec, TurnIndex, TurnSpan};
use crate::adapters::shared::editing::{
    is_spawn_name, reject_replacement_spawn, reject_target_spawn, replace_at_first, EditDocument,
};
use crate::errors::{DomainError, DomainResult};
use crate::events::{event, Event};

use super::native::{prune_referenced_subtrees, CodexClosure};

const SKIP_USER_PREFIX: [&str; 4] = [
    "<environment_context>",
    "<user_instructions>",
    "<ENVIRONMENT_CONTEXT>",
    "<turn_aborted>",
];

/// 参与 `call/output` 配对与回复判定的四个子类型名。
pub const CALL_SUBTYPES: [&str; 2] = ["custom_tool_call", "function_call"];
pub const OUTPUT_SUBTYPES: [&str; 2] = ["custom_tool_call_output", "function_call_output"];

fn message_text(payload: &Map<String, Value>) -> String {
    payload
        .get("content")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_object)
                .map(|block| {
                    block
                        .get("text")
                        .map(|value| match value {
                            Value::String(text) => text.clone(),
                            other => crate::adapters::shared::dialect::python_str(other),
                        })
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn payload_of(record: &Value) -> &Map<String, Value> {
    static EMPTY: std::sync::LazyLock<Map<String, Value>> = std::sync::LazyLock::new(Map::new);
    record
        .get("payload")
        .and_then(Value::as_object)
        .unwrap_or(&EMPTY)
}

/// 一条记录是否属于「AI 回复」：assistant 消息或四个 call/output 子类型之一。
fn is_reply_record(record: &Value) -> bool {
    if record.get("type").and_then(Value::as_str) != Some("response_item") {
        return false;
    }
    let payload = payload_of(record);
    let subtype = payload.get("type").and_then(Value::as_str).unwrap_or("");
    (subtype == "message" && payload.get("role").and_then(Value::as_str) == Some("assistant"))
        || CALL_SUBTYPES.contains(&subtype)
        || OUTPUT_SUBTYPES.contains(&subtype)
}

fn is_spawn(record: &Value) -> bool {
    let payload = payload_of(record);
    let subtype = payload.get("type").and_then(Value::as_str).unwrap_or("");
    CALL_SUBTYPES.contains(&subtype) && is_spawn_name(payload.get("name"))
}

/// Codex 的轮次索引。
pub struct CodexTurnIndex;

impl TurnIndex for CodexTurnIndex {
    type Document = [Value];
    /// 可见消息在原生记录序列里的下标。
    type VisibleMessage = usize;

    fn visible_messages(&self, records: &Self::Document) -> Vec<usize> {
        let mut out = Vec::new();
        for (index, record) in records.iter().enumerate() {
            let payload = payload_of(record);
            if record.get("type").and_then(Value::as_str) != Some("response_item")
                || payload.get("type").and_then(Value::as_str) != Some("message")
            {
                continue;
            }
            let text = message_text(payload);
            let trimmed = text.trim();
            if payload.get("role").and_then(Value::as_str) == Some("user")
                && SKIP_USER_PREFIX
                    .iter()
                    .any(|prefix| trimmed.starts_with(prefix))
            {
                continue;
            }
            if !trimmed.is_empty() {
                out.push(index);
            }
        }
        out
    }

    fn turns(&self, records: &Self::Document) -> Vec<TurnSpan> {
        let starts: Vec<usize> = self
            .visible_messages(records)
            .into_iter()
            .filter(|index| {
                payload_of(&records[*index])
                    .get("role")
                    .and_then(Value::as_str)
                    == Some("user")
            })
            .collect();
        starts
            .iter()
            .enumerate()
            .map(|(offset, start)| {
                let end = starts.get(offset + 1).copied().unwrap_or(records.len());
                TurnSpan::new(offset + 1, format!("record:{start}"), *start, end)
            })
            .collect()
    }
}

/// Codex 的编辑编解码器。
pub struct CodexEditCodec;

impl CodexEditCodec {
    fn records<'a>(&self, doc: &'a mut EditDocument) -> &'a mut Vec<Value> {
        doc.data
            .downcast_mut::<Vec<Value>>()
            .expect("Codex 编辑文档承载 Vec<Value>")
    }
}

impl NativeEditCodec for CodexEditCodec {
    type Document = EditDocument;
    type Reply = Value;
    type Change = Event;

    fn replace_reply(
        &self,
        document: &mut EditDocument,
        span: &TurnSpan,
        reply: &Value,
    ) -> DomainResult<Vec<Event>> {
        reject_replacement_spawn(reply)?;
        let now = super::writer::now_iso();
        {
            let records = self.records(document);
            let old: Vec<Value> = records[span.start + 1..span.end].to_vec();
            if old.iter().any(is_spawn) {
                return Err(reject_target_spawn("codex"));
            }
            let items = reply
                .get("items")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let mut compiled: Vec<Value> = Vec::new();
            for item in &items {
                if item.get("kind").and_then(Value::as_str) == Some("text") {
                    let text = item.get("text").cloned().unwrap_or(Value::from(""));
                    let mut payload = Map::new();
                    payload.insert("type".into(), Value::from("message"));
                    payload.insert(
                        "id".into(),
                        Value::from(format!("msg_{}", super::writer::token_hex(12))),
                    );
                    payload.insert("role".into(), Value::from("assistant"));
                    let mut block = Map::new();
                    block.insert("type".into(), Value::from("output_text"));
                    block.insert("text".into(), text);
                    payload.insert("content".into(), Value::Array(vec![Value::Object(block)]));
                    payload.insert("phase".into(), Value::from("final_answer"));
                    compiled.push(record(&now, "response_item", payload));
                    continue;
                }
                let call_id = format!("call_{}", super::writer::token_urlsafe_24());
                let input = item.get("input").cloned().unwrap_or(Value::Null);
                let arguments = match &input {
                    Value::Object(_) => {
                        Value::from(crate::adapters::shared::writing::python_json_dumps(&input))
                    }
                    other => other.clone(),
                };
                let mut call = Map::new();
                call.insert("type".into(), Value::from("function_call"));
                call.insert(
                    "id".into(),
                    Value::from(format!("fc_{}", super::writer::token_hex(12))),
                );
                call.insert(
                    "name".into(),
                    item.get("name").cloned().unwrap_or(Value::from("")),
                );
                call.insert("arguments".into(), arguments);
                call.insert("call_id".into(), Value::from(call_id.as_str()));
                call.insert("status".into(), Value::from("completed"));
                let mut output = Map::new();
                output.insert("type".into(), Value::from("function_call_output"));
                output.insert(
                    "id".into(),
                    Value::from(format!("fco_{}", super::writer::token_hex(12))),
                );
                output.insert("call_id".into(), Value::from(call_id.as_str()));
                output.insert(
                    "output".into(),
                    item.get("output").cloned().unwrap_or(Value::from("")),
                );
                compiled.push(record(&now, "response_item", call));
                compiled.push(record(&now, "response_item", output));
            }
            let replacement = replace_at_first(&old, is_reply_record, &compiled);
            records.splice(span.start + 1..span.end, replacement);
        }
        let items = reply
            .get("items")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        let mut params = Map::new();
        params.insert("turn".into(), Value::from(span.ordinal as i64));
        params.insert("items".into(), Value::from(items as i64));
        Ok(vec![event("edit.reply_replaced", params)])
    }

    fn delete_turn(
        &self,
        document: &mut EditDocument,
        span: &TurnSpan,
    ) -> DomainResult<Vec<Event>> {
        let removed: Vec<Value> = {
            let records = self.records(document);
            records
                .splice(span.start..span.end, std::iter::empty())
                .collect()
        };
        let mut pruned = 0usize;
        if let Some(closure) = document
            .context
            .as_mut()
            .and_then(|context| context.downcast_mut::<CodexClosure>())
        {
            pruned = prune_referenced_subtrees(closure, &removed)?.len();
        }
        let mut params = Map::new();
        params.insert("turn".into(), Value::from(span.ordinal as i64));
        if pruned > 0 {
            params.insert("pruned_children".into(), Value::from(pruned as i64));
            return Ok(vec![event("edit.turn_deleted_with_children", params)]);
        }
        Ok(vec![event("edit.turn_deleted", params)])
    }

    fn rewrite_message(
        &self,
        document: &mut EditDocument,
        locator: &str,
        text: &str,
    ) -> DomainResult<Vec<Event>> {
        let index = self.locate(document, locator);
        let Some(index) = index else {
            let mut params = Map::new();
            params.insert("locator".into(), Value::from(locator));
            return Err(DomainError::locator_stale(
                Some("Codex 消息定位符已失效，请刷新会话"),
                params,
            ));
        };
        let records = self.records(document);
        let payload = records[index]
            .get_mut("payload")
            .and_then(Value::as_object_mut)
            .expect("定位阶段已确认 payload 是对象");
        let role = payload
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if role != "user" && role != "assistant" {
            return Err(DomainError::operation_unsupported(
                "codex",
                "rewrite",
                Some(&role),
            ));
        }
        let content = payload
            .get("content")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let text_types = ["input_text", "output_text"];
        let first = content.iter().position(|item| {
            item.get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| text_types.contains(&kind))
        });
        let Some(first) = first else {
            return Err(DomainError::operation_unsupported(
                "codex",
                "rewrite",
                Some("no-text"),
            ));
        };
        let mut rewritten: Vec<Value> = content
            .iter()
            .filter(|item| {
                !item
                    .get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| text_types.contains(&kind))
            })
            .cloned()
            .collect();
        let mut block = Map::new();
        block.insert(
            "type".into(),
            Value::from(if role == "user" {
                "input_text"
            } else {
                "output_text"
            }),
        );
        block.insert("text".into(), Value::from(text));
        // Python 的 `list.insert` 在超界时等价 append。
        let position = first.min(rewritten.len());
        rewritten.insert(position, Value::Object(block));
        payload.insert("content".into(), Value::Array(rewritten));
        Ok(vec![event("edit.message_rewritten", {
            let mut params = Map::new();
            params.insert("count".into(), Value::from(1));
            params
        })])
    }
}

impl CodexEditCodec {
    /// `record:N` 走原生下标，`index:N` 走可见消息序号。
    fn locate(&self, document: &mut EditDocument, locator: &str) -> Option<usize> {
        let records = document
            .data
            .downcast_ref::<Vec<Value>>()
            .expect("Codex 编辑文档承载 Vec<Value>");
        let is_message = |index: usize| -> bool {
            records.get(index).is_some_and(|record| {
                record.get("type").and_then(Value::as_str) == Some("response_item")
                    && payload_of(record).get("type").and_then(Value::as_str) == Some("message")
            })
        };
        if let Some(rest) = locator.strip_prefix("record:") {
            let ordinal = rest.parse::<usize>().ok()?;
            return is_message(ordinal).then_some(ordinal);
        }
        if let Some(rest) = locator.strip_prefix("index:") {
            let wanted = rest.parse::<usize>().ok()?;
            return CodexTurnIndex
                .visible_messages(records)
                .get(wanted)
                .copied();
        }
        None
    }
}

fn record(timestamp: &str, kind: &str, payload: Map<String, Value>) -> Value {
    let mut entry = Map::new();
    entry.insert("timestamp".into(), Value::from(timestamp));
    entry.insert("type".into(), Value::from(kind));
    entry.insert("payload".into(), Value::Object(payload));
    Value::Object(entry)
}

/// 进程级单例，对齐 Python 的 `TURN_INDEX` / `CODEC`。
pub const TURN_INDEX: CodexTurnIndex = CodexTurnIndex;
pub const CODEC: CodexEditCodec = CodexEditCodec;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn message(role: &str, text: &str) -> Value {
        json!({"type": "response_item", "payload": {
            "type": "message", "role": role,
            "content": [{"type": if role == "user" {"input_text"} else {"output_text"},
                         "text": text}]}})
    }

    fn document(records: Vec<Value>) -> EditDocument {
        let mut doc = EditDocument::new(
            "codex",
            "ref",
            Box::new(std::path::PathBuf::from("/tmp/x.jsonl")),
            Box::new(records),
            "sha256:x",
        );
        doc.context = None;
        doc
    }

    fn records_of(doc: &EditDocument) -> Vec<Value> {
        doc.data.downcast_ref::<Vec<Value>>().unwrap().clone()
    }

    #[test]
    fn turns_start_at_every_visible_user_message() {
        let records = vec![
            json!({"type": "session_meta", "payload": {"id": "s"}}),
            message("user", "<environment_context>skip"),
            message("user", "one"),
            message("assistant", "a"),
            message("user", "two"),
        ];
        let spans = TURN_INDEX.turns(&records);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].locator, "record:2");
        assert_eq!((spans[0].start, spans[0].end), (2, 4));
        assert_eq!((spans[1].start, spans[1].end), (4, 5));
        // 环境上下文与空正文都不算可见消息。
        assert_eq!(TURN_INDEX.visible_messages(&records), [2, 3, 4]);
    }

    #[test]
    fn delete_turn_removes_the_whole_span() {
        let records = vec![
            json!({"type": "session_meta", "payload": {"id": "s"}}),
            message("user", "one"),
            message("assistant", "a"),
            message("user", "two"),
        ];
        let mut doc = document(records);
        let spans = TURN_INDEX.turns(doc.data.downcast_ref::<Vec<Value>>().unwrap());
        let changes = CODEC.delete_turn(&mut doc, &spans[0]).unwrap();
        assert_eq!(changes[0].code, "edit.turn_deleted");
        assert_eq!(records_of(&doc).len(), 2);
    }

    #[test]
    fn replace_reply_swaps_the_first_reply_slot() {
        let records = vec![
            json!({"type": "session_meta", "payload": {"id": "s"}}),
            message("user", "one"),
            message("assistant", "old"),
            json!({"type": "response_item", "payload": {
                "type": "function_call", "call_id": "c", "name": "exec", "arguments": "{}"}}),
            json!({"type": "response_item", "payload": {
                "type": "function_call_output", "call_id": "c", "output": "x"}}),
        ];
        let mut doc = document(records);
        let spans = TURN_INDEX.turns(doc.data.downcast_ref::<Vec<Value>>().unwrap());
        let reply = json!({"items": [
            {"kind": "text", "text": "new"},
            {"kind": "tool", "name": "exec", "input": {"cmd": "ls"}, "output": "ok"},
        ]});
        let changes = CODEC.replace_reply(&mut doc, &spans[0], &reply).unwrap();
        assert_eq!(changes[0].code, "edit.reply_replaced");
        assert_eq!(changes[0].params["items"], json!(2));
        let out = records_of(&doc);
        // 旧的 3 条回复记录被压成 3 条新记录（1 条文本 + 1 对 call/output）。
        assert_eq!(out.len(), 5);
        assert_eq!(out[2]["payload"]["content"][0]["text"], json!("new"));
        assert_eq!(out[3]["payload"]["type"], json!("function_call"));
        assert_eq!(out[3]["payload"]["arguments"], json!("{\"cmd\": \"ls\"}"));
        assert_eq!(out[4]["payload"]["call_id"], out[3]["payload"]["call_id"]);
    }

    #[test]
    fn replace_reply_rejects_spawn_on_both_sides() {
        let records = vec![
            message("user", "one"),
            json!({"type": "response_item", "payload": {
                "type": "function_call", "call_id": "c", "name": "spawn_agent",
                "arguments": "{}"}}),
        ];
        let mut doc = document(records);
        let spans = TURN_INDEX.turns(doc.data.downcast_ref::<Vec<Value>>().unwrap());
        let error = CODEC
            .replace_reply(
                &mut doc,
                &spans[0],
                &json!({"items": [{"kind": "text", "text": "x"}]}),
            )
            .unwrap_err();
        assert_eq!(error.code, "edit.subagent_not_supported");

        let mut doc = document(vec![message("user", "one")]);
        let spans = TURN_INDEX.turns(doc.data.downcast_ref::<Vec<Value>>().unwrap());
        let error = CODEC
            .replace_reply(
                &mut doc,
                &spans[0],
                &json!({"items": [{"kind": "tool", "name": "Task", "input": {}, "output": ""}]}),
            )
            .unwrap_err();
        assert_eq!(error.code, "edit.subagent_not_supported");
    }

    #[test]
    fn rewrite_supports_both_locator_forms() {
        let records = vec![
            json!({"type": "session_meta", "payload": {"id": "s"}}),
            message("user", "one"),
            message("assistant", "a"),
        ];
        let mut doc = document(records);
        CODEC
            .rewrite_message(&mut doc, "record:1", "edited")
            .unwrap();
        assert_eq!(
            records_of(&doc)[1]["payload"]["content"],
            json!([{"type": "input_text", "text": "edited"}])
        );
        CODEC.rewrite_message(&mut doc, "index:1", "reply").unwrap();
        assert_eq!(
            records_of(&doc)[2]["payload"]["content"],
            json!([{"type": "output_text", "text": "reply"}])
        );
    }

    #[test]
    fn rewrite_reports_stale_locators_and_unsupported_roles() {
        let records = vec![
            message("user", "one"),
            json!({"type": "response_item", "payload": {
                "type": "message", "role": "developer",
                "content": [{"type": "input_text", "text": "sys"}]}}),
        ];
        let mut doc = document(records);
        let error = CODEC
            .rewrite_message(&mut doc, "record:9", "x")
            .unwrap_err();
        assert_eq!(error.code, "session.locator_stale");
        assert_eq!(error.message(), "Codex 消息定位符已失效，请刷新会话");

        let error = CODEC
            .rewrite_message(&mut doc, "record:1", "x")
            .unwrap_err();
        assert_eq!(error.code, "edit.operation_unsupported");
        assert_eq!(error.params()["mode"], json!("developer"));
    }
}
