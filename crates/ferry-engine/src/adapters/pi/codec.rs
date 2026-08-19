//! Pi v3 线性活动分支编辑 codec。
//!
//! 编辑面只作用在**活动分支**上：轮次划分、删除、替换回复都先算一遍
//! [`active_indexes`]（与 `reader::active_branch` 同一算法），非活动分支的记录
//! 原样留在文件里。删除轮次时除了重连 `parentId`，还必须修复三个交叉引用
//! （`targetId` / `firstKeptEntryId` / `fromId`）——它们指向 entry id，被删掉
//! 又没有存活祖先时整个键要 pop 掉，否则 `editor::validate` 会拒绝提交。

use std::collections::HashSet;

use serde_json::{json, Map, Value};

use crate::adapters::shared::codec::{NativeEditCodec, TurnIndex, TurnSpan};
use crate::adapters::shared::dialect::python_str;
use crate::adapters::shared::editing::reject_replacement_spawn;
use crate::errors::{DomainError, DomainResult};
use crate::events::{event, Event};

use super::writer::{iso_stamp, now_millis, uuid4_hex, zero_usage};

/// 会随删除轮次一起重写的交叉引用字段。
pub const CROSS_REFERENCE_FIELDS: [&str; 3] = ["targetId", "firstKeptEntryId", "fromId"];

/// 活动分支在 `records` 中的下标序列（升序，含 header 之后的全部命中项）。
pub fn active_indexes(records: &[Value]) -> Vec<usize> {
    let valid: Vec<(usize, &Value)> = records
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(_, row)| {
            row.is_object()
                && row.get("id").and_then(Value::as_str).is_some()
                && row.get("parentId").is_some()
        })
        .collect();
    let Some(last) = valid.last().copied() else {
        return Vec::new();
    };
    fn id_of(row: &Value) -> &str {
        row.get("id").and_then(Value::as_str).unwrap_or_default()
    }
    let mut out: Vec<usize> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut current = Some(last);
    while let Some((index, row)) = current {
        if seen.contains(id_of(row)) {
            break;
        }
        out.push(index);
        seen.insert(id_of(row).to_string());
        current = row
            .get("parentId")
            .and_then(Value::as_str)
            .and_then(|parent| {
                valid
                    .iter()
                    .find(|(_, candidate)| id_of(candidate) == parent)
                    .copied()
            });
    }
    out.reverse();
    out
}

fn role_of(row: &Value) -> Option<&str> {
    row.get("message")?.get("role")?.as_str()
}

fn is_message(row: &Value) -> bool {
    row.get("type").and_then(Value::as_str) == Some("message")
}

/// 轮次索引：只认活动分支上的 user / assistant / bashExecution 消息。
pub struct PiTurnIndex;

impl TurnIndex for PiTurnIndex {
    type Document = [Value];
    type VisibleMessage = (usize, Value);

    fn visible_messages(&self, records: &Self::Document) -> Vec<Self::VisibleMessage> {
        active_indexes(records)
            .into_iter()
            .filter(|index| {
                is_message(&records[*index])
                    && matches!(
                        role_of(&records[*index]),
                        Some("user") | Some("assistant") | Some("bashExecution")
                    )
            })
            .map(|index| (index, records[index].clone()))
            .collect()
    }

    fn turns(&self, records: &Self::Document) -> Vec<TurnSpan> {
        let active = active_indexes(records);
        let starts: Vec<usize> = active
            .iter()
            .copied()
            .filter(|index| {
                is_message(&records[*index]) && role_of(&records[*index]) == Some("user")
            })
            .collect();
        starts
            .iter()
            .enumerate()
            .map(|(ordinal, start)| {
                let end = match starts.get(ordinal + 1) {
                    Some(next) => *next,
                    None => active.last().map_or(records.len(), |index| index + 1),
                };
                TurnSpan::new(
                    ordinal + 1,
                    records[*start]
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                    *start,
                    end,
                )
            })
            .collect()
    }
}

/// `_entry(parent, message)`：新记录一律 12 位 hex id + 秒级 UTC 时间戳。
fn entry(parent: &Value, message: Value) -> Value {
    json!({"type": "message", "id": uuid4_hex(12), "parentId": parent,
           "timestamp": iso_stamp(), "message": message})
}

fn assistant_entry(parent: &Value, content: Vec<Value>, stop_reason: &str, now: i64) -> Value {
    entry(
        parent,
        json!({
            "role": "assistant", "content": content,
            "api": "ferry", "provider": "ferry", "model": "migrated",
            "usage": zero_usage(), "stopReason": stop_reason, "timestamp": now,
        }),
    )
}

fn locate(active: &[usize], index: usize) -> DomainResult<usize> {
    active
        .iter()
        .position(|item| *item == index)
        .ok_or_else(|| DomainError::internal("Pi 轮次不在活动分支上"))
}

pub struct PiEditCodec;

impl NativeEditCodec for PiEditCodec {
    type Document = Vec<Value>;
    type Reply = Value;
    type Change = Event;

    fn replace_reply(
        &self,
        records: &mut Self::Document,
        span: &TurnSpan,
        reply: &Self::Reply,
    ) -> DomainResult<Vec<Self::Change>> {
        reject_replacement_spawn(reply)?;
        let user_id = records[span.start]
            .get("id")
            .cloned()
            .unwrap_or(Value::Null);
        let active = active_indexes(records);
        let start_pos = locate(&active, span.start)?;
        let end_pos = match active.iter().position(|item| *item == span.end) {
            Some(position) => position,
            None => active.len(),
        };
        let targets: HashSet<usize> = active[start_pos + 1..end_pos].iter().copied().collect();
        let removed: HashSet<String> = targets
            .iter()
            .filter_map(|index| records[*index].get("id").and_then(Value::as_str))
            .map(str::to_string)
            .collect();

        let items = reply
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let now = now_millis();
        let mut parent = user_id;
        let mut compiled: Vec<Value> = Vec::new();
        let mut content: Vec<Value> = Vec::new();
        for item in &items {
            if item.get("kind").and_then(Value::as_str) == Some("text") {
                content.push(json!({"type": "text",
                                    "text": item.get("text").cloned().unwrap_or(Value::Null)}));
                continue;
            }
            let call_id = format!("call_{}", uuid4_hex(16));
            let name = item.get("name").cloned().unwrap_or(Value::Null);
            content.push(json!({"type": "toolCall", "id": call_id, "name": name,
                                "arguments": item.get("input").cloned()
                                    .unwrap_or(Value::Null)}));
            let assistant = assistant_entry(&parent, std::mem::take(&mut content), "toolUse", now);
            let result = entry(
                &assistant["id"],
                json!({
                    "role": "toolResult", "toolCallId": call_id, "toolName": name,
                    "content": [{"type": "text",
                                 "text": item.get("output").cloned().unwrap_or(Value::Null)}],
                    "isError": false, "timestamp": now,
                }),
            );
            parent = result["id"].clone();
            compiled.push(assistant);
            compiled.push(result);
        }
        if !content.is_empty() {
            let assistant = assistant_entry(&parent, content, "stop", now);
            parent = assistant["id"].clone();
            compiled.push(assistant);
        }

        let insert_at = targets.iter().copied().min().unwrap_or(span.start + 1);
        let mut rebuilt: Vec<Value> = Vec::with_capacity(records.len() + compiled.len());
        for (index, row) in records.iter().enumerate() {
            if index == insert_at {
                rebuilt.extend(compiled.iter().cloned());
            }
            if !targets.contains(&index) {
                rebuilt.push(row.clone());
            }
        }
        if insert_at >= records.len() {
            rebuilt.extend(compiled);
        }
        *records = rebuilt;
        for row in records.iter_mut() {
            let hit = row
                .get("parentId")
                .and_then(Value::as_str)
                .is_some_and(|value| removed.contains(value));
            if hit {
                if let Some(entries) = row.as_object_mut() {
                    entries.insert("parentId".into(), parent.clone());
                }
            }
        }
        let mut params = Map::new();
        params.insert("turn".into(), Value::from(span.ordinal as i64));
        params.insert("items".into(), Value::from(items.len() as i64));
        Ok(vec![event("edit.reply_replaced", params)])
    }

    fn delete_turn(
        &self,
        records: &mut Self::Document,
        span: &TurnSpan,
    ) -> DomainResult<Vec<Self::Change>> {
        let active = active_indexes(records);
        let start_pos = locate(&active, span.start)?;
        let end_pos = match active.iter().position(|item| *item == span.end) {
            Some(position) => position,
            None => active.len(),
        };
        let targets: HashSet<usize> = active[start_pos..end_pos].iter().copied().collect();
        let mut removed: HashSet<String> = HashSet::new();
        // id → 被删记录自己的 parentId（可能仍指向另一条被删记录）。
        let mut parent_by_id: Vec<(String, Option<String>)> = Vec::new();
        for index in &targets {
            let row = &records[*index];
            if let Some(id) = row.get("id").and_then(Value::as_str) {
                removed.insert(id.to_string());
                parent_by_id.push((
                    id.to_string(),
                    row.get("parentId")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                ));
            }
        }
        let mut kept: Vec<Value> = Vec::with_capacity(records.len());
        for (index, row) in records.drain(..).enumerate() {
            if !targets.contains(&index) {
                kept.push(row);
            }
        }
        *records = kept;

        // 沿被删记录的 parentId 链上溯，直到落在存活记录上（或走空）。
        let surviving = |start: &str| -> Option<String> {
            let mut seen: HashSet<String> = HashSet::new();
            let mut value: Option<String> = Some(start.to_string());
            while let Some(current) = value.clone() {
                if !removed.contains(&current) || !seen.insert(current.clone()) {
                    break;
                }
                value = parent_by_id
                    .iter()
                    .find(|(id, _)| *id == current)
                    .and_then(|(_, parent)| parent.clone());
            }
            value
        };

        for row in records.iter_mut().skip(1) {
            let Some(entries) = row.as_object_mut() else {
                continue;
            };
            let dangling = entries
                .get("parentId")
                .and_then(Value::as_str)
                .filter(|value| removed.contains(*value))
                .map(str::to_string);
            if let Some(dangling) = dangling {
                entries.insert(
                    "parentId".into(),
                    surviving(&dangling).map_or(Value::Null, Value::from),
                );
            }
            for field in CROSS_REFERENCE_FIELDS {
                let dangling = entries
                    .get(field)
                    .and_then(Value::as_str)
                    .filter(|value| removed.contains(*value))
                    .map(str::to_string);
                let Some(dangling) = dangling else {
                    continue;
                };
                match surviving(&dangling) {
                    // 整条链都被删干净了：留着就是悬空引用，直接摘掉这个键。
                    None => {
                        entries.remove(field);
                    }
                    Some(replacement) => {
                        entries.insert(field.into(), Value::from(replacement));
                    }
                }
            }
        }
        let mut params = Map::new();
        params.insert("turn".into(), Value::from(span.ordinal as i64));
        Ok(vec![event("edit.turn_deleted", params)])
    }

    fn rewrite_message(
        &self,
        records: &mut Self::Document,
        locator: &str,
        text: &str,
    ) -> DomainResult<Vec<Self::Change>> {
        let position = records.iter().position(|row| {
            row.is_object() && row.get("id").and_then(Value::as_str) == Some(locator)
        });
        let Some(position) = position else {
            let mut params = Map::new();
            params.insert("locator".into(), Value::from(locator));
            return Err(DomainError::locator_stale(None, params));
        };
        let role = records[position]
            .get("message")
            .and_then(|message| message.get("role"))
            .cloned();
        let editable = matches!(
            role.as_ref().and_then(Value::as_str),
            Some("user") | Some("assistant")
        );
        if !editable {
            return Err(DomainError::operation_unsupported(
                "pi",
                "rewrite",
                Some(&python_str(&role.unwrap_or(Value::Null))),
            ));
        }
        let content = records[position]["message"].get("content").cloned();
        match content {
            Some(Value::String(_)) => {
                records[position]["message"]["content"] = Value::from(text);
            }
            Some(Value::Array(parts)) => {
                let slot = parts
                    .iter()
                    .position(|part| part.get("type").and_then(Value::as_str) == Some("text"));
                let Some(slot) = slot else {
                    return Err(DomainError::operation_unsupported(
                        "pi",
                        "rewrite",
                        Some("no-text"),
                    ));
                };
                records[position]["message"]["content"][slot] =
                    json!({"type": "text", "text": text});
            }
            _ => {
                return Err(DomainError::operation_unsupported(
                    "pi",
                    "rewrite",
                    Some("no-text"),
                ))
            }
        }
        let mut params = Map::new();
        params.insert("count".into(), Value::from(1));
        Ok(vec![event("edit.message_rewritten", params)])
    }
}

/// 进程级单例，对齐 Python 的模块级 `TURN_INDEX` / `CODEC`。
pub static TURN_INDEX: PiTurnIndex = PiTurnIndex;
pub static CODEC: PiEditCodec = PiEditCodec;

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> Value {
        json!({"type": "session", "version": 3, "id": "s",
               "timestamp": "2026-07-25T00:00:00Z", "cwd": "/tmp"})
    }

    fn user(id: &str, parent: Value, text: &str) -> Value {
        json!({"type": "message", "id": id, "parentId": parent,
               "timestamp": "2026-07-25T00:00:01Z",
               "message": {"role": "user", "content": text, "timestamp": 1}})
    }

    fn assistant(id: &str, parent: Value, text: &str) -> Value {
        json!({"type": "message", "id": id, "parentId": parent,
               "timestamp": "2026-07-25T00:00:02Z",
               "message": {"role": "assistant",
                           "content": [{"type": "text", "text": text}],
                           "api": "a", "provider": "p", "model": "m",
                           "usage": {}, "stopReason": "stop", "timestamp": 2}})
    }

    #[test]
    fn active_indexes_follow_the_last_leaf() {
        let records = vec![
            header(),
            user("u1", Value::Null, "one"),
            assistant("dead", json!("u1"), "dead"),
            assistant("live", json!("u1"), "live"),
        ];
        assert_eq!(active_indexes(&records), [1, 3]);
    }

    #[test]
    fn turns_span_from_each_user_message_to_the_next() {
        let records = vec![
            header(),
            user("u1", Value::Null, "one"),
            assistant("a1", json!("u1"), "reply"),
            user("u2", json!("a1"), "two"),
            assistant("a2", json!("u2"), "reply"),
        ];
        let turns = TURN_INDEX.turns(&records);
        assert_eq!(turns.len(), 2);
        assert_eq!((turns[0].ordinal, turns[0].start, turns[0].end), (1, 1, 3));
        assert_eq!(turns[0].locator, "u1");
        assert_eq!((turns[1].ordinal, turns[1].start, turns[1].end), (2, 3, 5));
        assert_eq!(
            TURN_INDEX
                .visible_messages(&records)
                .iter()
                .map(|(index, _)| *index)
                .collect::<Vec<_>>(),
            [1, 2, 3, 4]
        );
    }

    #[test]
    fn delete_turn_relinks_parents_and_repairs_cross_references() {
        let mut records = vec![
            header(),
            user("u1", Value::Null, "one"),
            assistant("a1", json!("u1"), "reply"),
            user("u2", json!("a1"), "two"),
            json!({"type": "branch_summary", "id": "sum", "parentId": "u2",
                   "fromId": "a1", "summary": "s"}),
            json!({"type": "compaction", "id": "c1", "parentId": "sum",
                   "firstKeptEntryId": "u1", "targetId": "u2"}),
        ];
        let turns = TURN_INDEX.turns(&records);
        let span = turns[0].clone();
        CODEC.delete_turn(&mut records, &span).unwrap();

        let ids: Vec<&str> = records
            .iter()
            .skip(1)
            .map(|row| row["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["u2", "sum", "c1"]);
        // u2 的 parentId 原本指向被删的 a1，回溯到 u1（也被删）→ 最终为 null。
        assert_eq!(records[1]["parentId"], Value::Null);
        // fromId 指向被删的 a1 且无存活祖先 → 整个键 pop 掉。
        assert!(records[2].get("fromId").is_none());
        // firstKeptEntryId 指向被删的 u1 → 同样 pop。
        assert!(records[3].get("firstKeptEntryId").is_none());
        // targetId 指向仍存活的 u2 → 原样保留。
        assert_eq!(records[3]["targetId"], json!("u2"));
    }

    #[test]
    fn delete_turn_keeps_cross_references_that_survive() {
        // 被删的是第二轮（u2/a2）；第三轮里的 compaction 引用它们，必须沿
        // parentId 链多跳回溯到仍存活的 a1，而不是被 pop 掉。
        let mut records = vec![
            header(),
            user("u1", Value::Null, "one"),
            assistant("a1", json!("u1"), "reply"),
            user("u2", json!("a1"), "two"),
            assistant("a2", json!("u2"), "reply"),
            user("u3", json!("a2"), "three"),
            json!({"type": "compaction", "id": "c1", "parentId": "u3",
                   "firstKeptEntryId": "u2", "fromId": "a2"}),
        ];
        let turns = TURN_INDEX.turns(&records);
        assert_eq!(turns.len(), 3);
        let span = turns[1].clone();
        CODEC.delete_turn(&mut records, &span).unwrap();

        let ids: Vec<&str> = records
            .iter()
            .skip(1)
            .map(|row| row["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["u1", "a1", "u3", "c1"]);
        // u3 原本挂在被删的 a2 上 → 回溯到 a1。
        assert_eq!(records[3]["parentId"], json!("a1"));
        // firstKeptEntryId=u2（一跳）与 fromId=a2（两跳）都落到 a1。
        let compaction = records.last().unwrap();
        assert_eq!(compaction["firstKeptEntryId"], json!("a1"));
        assert_eq!(compaction["fromId"], json!("a1"));
        assert_eq!(compaction["parentId"], json!("u3"));
    }

    #[test]
    fn replace_reply_swaps_the_assistant_tail_and_relinks_followers() {
        let mut records = vec![
            header(),
            user("u1", Value::Null, "one"),
            assistant("a1", json!("u1"), "old"),
            user("u2", json!("a1"), "two"),
        ];
        let turns = TURN_INDEX.turns(&records);
        let span = turns[0].clone();
        let reply = json!({"items": [
            {"kind": "text", "text": "hello"},
            {"kind": "tool", "name": "read", "input": {"path": "/a"}, "output": "ok"},
        ]});
        let changes = CODEC.replace_reply(&mut records, &span, &reply).unwrap();
        assert_eq!(changes[0].code, "edit.reply_replaced");
        assert_eq!(changes[0].params["items"], json!(2));

        // a1 被替换成 assistant + toolResult 两条。
        let roles: Vec<Option<&str>> = records.iter().skip(1).map(role_of).collect();
        assert_eq!(
            roles,
            [
                Some("user"),
                Some("assistant"),
                Some("toolResult"),
                Some("user")
            ]
        );
        let new_assistant = &records[2];
        assert_eq!(new_assistant["parentId"], json!("u1"));
        assert_eq!(new_assistant["message"]["stopReason"], json!("toolUse"));
        assert_eq!(
            new_assistant["message"]["content"][0]["text"],
            json!("hello")
        );
        assert_eq!(
            new_assistant["message"]["content"][1]["name"],
            json!("read")
        );
        // 原本挂在 a1 下的 u2 改挂到新链尾。
        assert_eq!(records[4]["parentId"], records[3]["id"]);
    }

    #[test]
    fn replace_reply_with_only_text_emits_a_single_stop_entry() {
        let mut records = vec![
            header(),
            user("u1", Value::Null, "one"),
            assistant("a1", json!("u1"), "old"),
        ];
        let span = TURN_INDEX.turns(&records)[0].clone();
        let reply = json!({"items": [{"kind": "text", "text": "replacement"}]});
        CODEC.replace_reply(&mut records, &span, &reply).unwrap();
        assert_eq!(records.len(), 3);
        assert_eq!(records[2]["message"]["stopReason"], json!("stop"));
        assert_eq!(
            records[2]["message"]["content"][0]["text"],
            json!("replacement")
        );
    }

    #[test]
    fn replace_reply_rejects_spawn_tools() {
        let mut records = vec![header(), user("u1", Value::Null, "one")];
        let span = TURN_INDEX.turns(&records)[0].clone();
        let reply = json!({"items": [{"kind": "tool", "name": "Task",
                                      "input": {}, "output": ""}]});
        let error = CODEC
            .replace_reply(&mut records, &span, &reply)
            .unwrap_err();
        assert_eq!(error.code, "edit.subagent_not_supported");
    }

    #[test]
    fn rewrite_handles_both_content_shapes_and_rejects_the_rest() {
        let mut records = vec![
            header(),
            user("u1", Value::Null, "one"),
            assistant("a1", json!("u1"), "old"),
            json!({"type": "message", "id": "r1", "parentId": "a1",
                   "message": {"role": "toolResult", "content": []}}),
        ];
        CODEC.rewrite_message(&mut records, "u1", "new").unwrap();
        assert_eq!(records[1]["message"]["content"], json!("new"));
        CODEC.rewrite_message(&mut records, "a1", "fixed").unwrap();
        assert_eq!(
            records[2]["message"]["content"],
            json!([{"type": "text", "text": "fixed"}])
        );
        let error = CODEC.rewrite_message(&mut records, "r1", "x").unwrap_err();
        assert_eq!(error.code, "edit.operation_unsupported");
        assert_eq!(error.params()["mode"], json!("toolResult"));
        let stale = CODEC
            .rewrite_message(&mut records, "nope", "x")
            .unwrap_err();
        assert_eq!(stale.code, "session.locator_stale");
    }

    #[test]
    fn rewrite_rejects_content_without_a_text_part() {
        let mut records = vec![
            header(),
            json!({"type": "message", "id": "u1", "parentId": null,
                   "message": {"role": "user",
                               "content": [{"type": "image", "data": "AA=="}]}}),
        ];
        let error = CODEC.rewrite_message(&mut records, "u1", "x").unwrap_err();
        assert_eq!(error.params()["mode"], json!("no-text"));
    }
}
