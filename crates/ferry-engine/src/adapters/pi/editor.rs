//! 事务式 Pi v3 文件编辑器。
//!
//! `commit` 有三道闸：字节级 revision 复核（源文件在预览后被改过就拒写）、
//! [`PiBackend::validate`] 的结构自检、以及落盘后用 reader 复读一遍。

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::adapters::contracts::SessionEditor;
use crate::adapters::shared::codec::{positive_turn, select_span, TurnIndex};
use crate::adapters::shared::editing::{
    default_saved_revision, hash_bytes, json_size, write_jsonl, EditDocument,
};
use crate::adapters::shared::scanner::split_jsonl_lines;
use crate::errors::{DomainError, DomainResult};
use crate::events::Event;
use crate::system::snapshots::snapshot_file;

use super::codec::CROSS_REFERENCE_FIELDS;
use super::codec::{CODEC, TURN_INDEX};
use crate::adapters::shared::codec::NativeEditCodec;

/// assistant 记录必须齐备的 7 个终态字段。
const ASSISTANT_TERMINAL_FIELDS: [&str; 7] = [
    "content",
    "api",
    "provider",
    "model",
    "usage",
    "stopReason",
    "timestamp",
];

pub struct PiBackend;

fn records_ref(doc: &EditDocument) -> DomainResult<&Vec<Value>> {
    doc.data
        .downcast_ref::<Vec<Value>>()
        .ok_or_else(|| DomainError::internal("Pi 编辑文档载荷类型不符"))
}

fn records_mut(doc: &mut EditDocument) -> DomainResult<&mut Vec<Value>> {
    doc.data
        .downcast_mut::<Vec<Value>>()
        .ok_or_else(|| DomainError::internal("Pi 编辑文档载荷类型不符"))
}

pub(super) fn handle_of(doc: &EditDocument) -> DomainResult<&PathBuf> {
    doc.handle
        .downcast_ref::<PathBuf>()
        .ok_or_else(|| DomainError::internal("Pi 编辑文档句柄类型不符"))
}

/// 会话记录的原生 cwd（探针复用）。
pub(super) fn document_cwd(doc: &EditDocument) -> Option<String> {
    records_ref(doc)
        .ok()?
        .first()?
        .get("cwd")?
        .as_str()
        .map(str::to_string)
}

/// 集合成员用 JSON 文本做键：Python 侧的 `calls` / `results` 是任意值的集合
/// （含 `None`），必须能表达非字符串成员，否则配对判定会漏掉异常数据。
fn key(value: Option<&Value>) -> String {
    serde_json::to_string(value.unwrap_or(&Value::Null)).unwrap_or_default()
}

fn truthy_id(row: &Value) -> Option<&str> {
    row.get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

impl SessionEditor for PiBackend {
    fn name(&self) -> &str {
        "pi"
    }

    fn operations(&self) -> &[&str] {
        &["delete-turn", "rewrite", "replace-assistant-reply"]
    }

    fn load(&self, reference: &str) -> DomainResult<EditDocument> {
        let path = fs::canonicalize(reference)
            .map_err(|_| DomainError::session_not_found("pi", reference))?;
        let raw = fs::read(&path).map_err(|_| DomainError::session_not_found("pi", reference))?;
        let text = String::from_utf8(raw.clone())
            .map_err(|_| DomainError::internal("Pi 会话文件不是合法 UTF-8"))?;
        let mut records: Vec<Value> = Vec::new();
        for line in split_jsonl_lines(&text) {
            if line.trim().is_empty() {
                continue;
            }
            let value = serde_json::from_str::<Value>(line)
                .map_err(|error| DomainError::internal(format!("Pi 会话行不可解析: {error}")))?;
            records.push(value);
        }
        Ok(EditDocument::new(
            "pi",
            reference,
            Box::new(path),
            Box::new(records),
            hash_bytes(&raw),
        ))
    }

    fn apply_ops(&self, doc: &mut EditDocument, ops: &[Value]) -> DomainResult<Vec<Event>> {
        let mut notes: Vec<Event> = Vec::new();
        for op in ops {
            match op.get("op").and_then(Value::as_str) {
                Some("delete-turn") => {
                    let ordinal = positive_turn(op.get("turn").unwrap_or(&Value::Null))?;
                    let spans = TURN_INDEX.turns(records_ref(doc)?);
                    let span = select_span(&spans, &Value::from(ordinal as i64))?.clone();
                    notes.extend(CODEC.delete_turn(records_mut(doc)?, &span)?);
                }
                Some("rewrite") => {
                    // Python 兼容旧字段名 `uuid`。
                    let locator = op
                        .get("locator")
                        .filter(|value| !value.is_null())
                        .or_else(|| op.get("uuid"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let text = op
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    notes.extend(CODEC.rewrite_message(records_mut(doc)?, &locator, &text)?);
                }
                other => {
                    return Err(DomainError::operation_unsupported(
                        "pi",
                        other.unwrap_or_default(),
                        None,
                    ))
                }
            }
        }
        Ok(notes)
    }

    fn replace_reply(
        &self,
        doc: &mut EditDocument,
        turn: &Value,
        reply: &Value,
    ) -> DomainResult<Vec<Event>> {
        let spans = TURN_INDEX.turns(records_ref(doc)?);
        let span = select_span(&spans, turn)?.clone();
        CODEC.replace_reply(records_mut(doc)?, &span, reply)
    }

    fn validate(&self, doc: &EditDocument) -> DomainResult<()> {
        let records = records_ref(doc)?;
        let header_ok = records.first().is_some_and(|header| {
            header.get("type").and_then(Value::as_str) == Some("session")
                && header.get("version").and_then(Value::as_i64) == Some(3)
        });
        if !header_ok {
            return Err(DomainError::internal("Pi 会话缺少 v3 header"));
        }
        let entries: Vec<&Value> = records
            .iter()
            .skip(1)
            .filter(|row| row.is_object())
            .collect();
        let ids: Vec<&str> = entries.iter().filter_map(|row| truthy_id(row)).collect();
        let known: HashSet<&str> = ids.iter().copied().collect();
        if ids.len() != entries.len() || ids.len() != known.len() {
            return Err(DomainError::internal("Pi entry id 无效或重复"));
        }

        let mut parents: Vec<(&str, Option<&str>)> = Vec::new();
        let mut calls: HashSet<String> = HashSet::new();
        let mut results: HashSet<String> = HashSet::new();
        for row in &entries {
            let parent = match row.get("parentId") {
                None | Some(Value::Null) => None,
                Some(value) => {
                    let parent = value.as_str().filter(|value| known.contains(value));
                    if parent.is_none() {
                        return Err(DomainError::internal("Pi parentId 指向不存在 entry"));
                    }
                    parent
                }
            };
            parents.push((truthy_id(row).unwrap_or_default(), parent));
            for field in CROSS_REFERENCE_FIELDS {
                if let Some(value) = row.get(field) {
                    if !value.as_str().is_some_and(|value| known.contains(value)) {
                        return Err(DomainError::internal(format!(
                            "Pi {field} 指向不存在 entry"
                        )));
                    }
                }
            }
            let Some(message) = row.get("message").filter(|value| value.is_object()) else {
                continue;
            };
            match message.get("role").and_then(Value::as_str) {
                Some("assistant") => {
                    let complete = ASSISTANT_TERMINAL_FIELDS
                        .iter()
                        .all(|field| message.get(*field).is_some());
                    if !complete {
                        return Err(DomainError::internal("Pi assistant 缺少终态字段"));
                    }
                    for part in message
                        .get("content")
                        .and_then(Value::as_array)
                        .map(Vec::as_slice)
                        .unwrap_or_default()
                    {
                        if part.get("type").and_then(Value::as_str) == Some("toolCall") {
                            calls.insert(key(part.get("id")));
                        }
                    }
                }
                Some("toolResult") => {
                    results.insert(key(message.get("toolCallId")));
                }
                _ => {}
            }
        }
        if calls != results {
            return Err(DomainError::internal("Pi 工具调用与结果未完整配对"));
        }
        for start in &known {
            let mut seen: HashSet<&str> = HashSet::new();
            let mut current = Some(*start);
            while let Some(id) = current {
                if !seen.insert(id) {
                    return Err(DomainError::internal("Pi entry tree 存在环"));
                }
                current = parents
                    .iter()
                    .find(|(entry_id, _)| *entry_id == id)
                    .and_then(|(_, parent)| *parent);
            }
        }
        Ok(())
    }

    fn stats(&self, doc: &EditDocument) -> DomainResult<Map<String, Value>> {
        let records = records_ref(doc)?;
        let mut stats = Map::new();
        stats.insert("count".into(), Value::from(records.len() as i64));
        stats.insert(
            "size".into(),
            Value::from(json_size(&Value::Array(records.clone())) as i64),
        );
        Ok(stats)
    }

    fn snapshot(
        &self,
        doc: &EditDocument,
        reason_code: &str,
        extra: Option<&Map<String, Value>>,
    ) -> DomainResult<Option<PathBuf>> {
        snapshot_file(handle_of(doc)?, reason_code, "pi", extra)
            .map(Some)
            .map_err(|error| DomainError::internal(format!("Pi 会话快照失败: {error}")))
    }

    fn restore_snapshot(&self, snapshot: &Path, doc: &EditDocument) -> DomainResult<()> {
        fs::copy(snapshot, handle_of(doc)?)
            .map(|_| ())
            .map_err(|error| DomainError::internal(format!("Pi 会话还原失败: {error}")))
    }

    fn commit(&self, doc: &mut EditDocument) -> DomainResult<Map<String, Value>> {
        let path = handle_of(doc)?.clone();
        let current = fs::read(&path)
            .map_err(|error| DomainError::internal(format!("Pi 会话不可读: {error}")))?;
        if hash_bytes(&current) != doc.revision {
            return Err(DomainError::concurrent_modification(
                "源会话在预览后已变化，请重新预览",
            ));
        }
        self.validate(doc)?;
        let records = records_ref(doc)?;
        write_jsonl(&path, records)
            .map_err(|error| DomainError::internal(format!("Pi 会话落盘失败: {error}")))?;
        // 落盘后立刻用 reader 复读一遍：写出去的必须是自己读得回来的。
        let reference = path.to_string_lossy().into_owned();
        super::reader::read(&reference)?;
        let session_id = records
            .first()
            .and_then(|header| header.get("id"))
            .cloned()
            .unwrap_or(Value::Null);
        let mut result = Map::new();
        result.insert("session_id".into(), session_id);
        result.insert("saved_as".into(), Value::from(reference.as_str()));
        result.insert(
            "resume".into(),
            Value::from(format!("pi --session {reference}")),
        );
        Ok(result)
    }

    fn saved_revision(
        &self,
        result: &Map<String, Value>,
        doc: &EditDocument,
    ) -> DomainResult<String> {
        default_saved_revision("pi", result, doc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const FIXTURE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/agent_formats/pi/case-01-plain/session.jsonl"
    );

    fn staged(root: &Path) -> String {
        let path = root.join("session.jsonl");
        fs::copy(FIXTURE, &path).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn rewrite_replace_and_commit_round_trip() {
        let root = tempfile::tempdir().unwrap();
        let reference = staged(root.path());
        let editor = PiBackend;
        let mut doc = editor.load(&reference).unwrap();
        editor
            .apply_ops(
                &mut doc,
                &[json!({"op": "rewrite", "locator": "u1", "text": "/raw token"})],
            )
            .unwrap();
        editor
            .replace_reply(
                &mut doc,
                &json!(1),
                &json!({"items": [{"kind": "text", "text": "replacement"}]}),
            )
            .unwrap();
        let result = editor.commit(&mut doc).unwrap();
        assert_eq!(result["session_id"], json!("fixture-pi-plain"));
        assert!(result["resume"]
            .as_str()
            .unwrap()
            .starts_with("pi --session "));

        let session = super::super::reader::read(&reference).unwrap();
        assert_eq!(session.messages[0].blocks[0].text, "/raw token");
        assert_eq!(session.messages[1].blocks[0].text, "replacement");
    }

    #[test]
    fn delete_turn_preserves_a_valid_header() {
        let root = tempfile::tempdir().unwrap();
        let reference = staged(root.path());
        let editor = PiBackend;
        let mut doc = editor.load(&reference).unwrap();
        editor
            .apply_ops(&mut doc, &[json!({"op": "delete-turn", "turn": 1})])
            .unwrap();
        editor.commit(&mut doc).unwrap();
        assert!(super::super::reader::read(&reference)
            .unwrap()
            .messages
            .is_empty());
    }

    #[test]
    fn commit_refuses_to_overwrite_a_changed_source() {
        let root = tempfile::tempdir().unwrap();
        let reference = staged(root.path());
        let editor = PiBackend;
        let mut doc = editor.load(&reference).unwrap();
        fs::write(&reference, b"{}\n").unwrap();
        let error = editor.commit(&mut doc).unwrap_err();
        assert_eq!(error.code, "session.concurrent_modification");
    }

    #[test]
    fn unknown_operations_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let reference = staged(root.path());
        let editor = PiBackend;
        let mut doc = editor.load(&reference).unwrap();
        let error = editor
            .apply_ops(&mut doc, &[json!({"op": "fork"})])
            .unwrap_err();
        assert_eq!(error.code, "edit.operation_unsupported");
    }

    fn document(records: Vec<Value>) -> EditDocument {
        EditDocument::new(
            "pi",
            "ref",
            Box::new(PathBuf::from("/tmp/x.jsonl")),
            Box::new(records),
            "sha256:0",
        )
    }

    fn header() -> Value {
        json!({"type": "session", "version": 3, "id": "s",
               "timestamp": "2026-07-25T00:00:00Z", "cwd": "/tmp"})
    }

    fn assistant(id: &str, parent: Value, calls: Value) -> Value {
        json!({"type": "message", "id": id, "parentId": parent,
               "message": {"role": "assistant", "content": calls,
                           "api": "ferry", "provider": "ferry", "model": "m",
                           "usage": {}, "stopReason": "stop", "timestamp": 1}})
    }

    #[test]
    fn validate_enforces_every_documented_invariant() {
        let editor = PiBackend;
        // v3 header 必需。
        let mut bad = header();
        bad["version"] = json!(2);
        assert!(editor.validate(&document(vec![bad])).is_err());
        assert!(editor.validate(&document(Vec::new())).is_err());

        // id 唯一。
        let duplicated = document(vec![
            header(),
            assistant("a", Value::Null, json!([])),
            assistant("a", json!("a"), json!([])),
        ]);
        assert_eq!(
            editor.validate(&duplicated).unwrap_err().message(),
            "Pi entry id 无效或重复"
        );

        // parentId 必须指向已知 entry。
        let dangling = document(vec![header(), assistant("a", json!("ghost"), json!([]))]);
        assert_eq!(
            editor.validate(&dangling).unwrap_err().message(),
            "Pi parentId 指向不存在 entry"
        );

        // 四个引用字段之一悬空即拒绝。
        let mut cross = assistant("a", Value::Null, json!([]));
        cross["fromId"] = json!("ghost");
        assert_eq!(
            editor
                .validate(&document(vec![header(), cross]))
                .unwrap_err()
                .message(),
            "Pi fromId 指向不存在 entry"
        );

        // assistant 终态字段齐备。
        let partial = json!({"type": "message", "id": "a", "parentId": null,
                             "message": {"role": "assistant", "content": []}});
        assert_eq!(
            editor
                .validate(&document(vec![header(), partial]))
                .unwrap_err()
                .message(),
            "Pi assistant 缺少终态字段"
        );

        // toolCall / toolResult 集合必须相等。
        let unpaired = document(vec![
            header(),
            assistant(
                "a",
                Value::Null,
                json!([{"type": "toolCall", "id": "c1", "name": "read", "arguments": {}}]),
            ),
        ]);
        assert_eq!(
            editor.validate(&unpaired).unwrap_err().message(),
            "Pi 工具调用与结果未完整配对"
        );
        let paired = document(vec![
            header(),
            assistant(
                "a",
                Value::Null,
                json!([{"type": "toolCall", "id": "c1", "name": "read", "arguments": {}}]),
            ),
            json!({"type": "message", "id": "r", "parentId": "a",
                   "message": {"role": "toolResult", "toolCallId": "c1",
                               "content": []}}),
        ]);
        assert!(editor.validate(&paired).is_ok());

        // 树无环。
        let cyclic = document(vec![
            header(),
            assistant("a", json!("b"), json!([])),
            assistant("b", json!("a"), json!([])),
        ]);
        assert_eq!(
            editor.validate(&cyclic).unwrap_err().message(),
            "Pi entry tree 存在环"
        );
    }

    #[test]
    fn stats_count_records_and_utf8_bytes() {
        let editor = PiBackend;
        let doc = document(vec![header()]);
        let stats = editor.stats(&doc).unwrap();
        assert_eq!(stats["count"], json!(1));
        assert!(stats["size"].as_i64().unwrap() > 0);
    }
}
