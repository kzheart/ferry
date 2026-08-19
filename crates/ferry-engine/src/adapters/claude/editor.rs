//! Claude 会话编辑后端：delete-turn / rewrite / replace-assistant-reply。
//!
//! 三个操作的轮次语义全部消费 [`super::codec`]，与 reader 共用同一份定义。

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::adapters::contracts::SessionEditor;
use crate::adapters::shared::codec::{
    positive_turn, select_span, NativeEditCodec, TurnIndex, TurnSpan,
};
use crate::adapters::shared::editing::{
    default_saved_revision, hash_bytes, json_size, EditDocument,
};
use crate::errors::{DomainError, DomainResult};
use crate::events::Event;

use super::codec::{CODEC, TURN_INDEX};
use super::editing as claude_edit;

/// manifest 声明的三个原生编辑操作，顺序不可改（`AgentAdapter` 会逐项比对）。
pub const OPERATIONS: &[&str] = &["delete-turn", "rewrite", "replace-assistant-reply"];

pub struct ClaudeBackend;

/// 从 `EditDocument` 取出 claude 的原生记录数组。
fn records(doc: &EditDocument) -> DomainResult<&Vec<Value>> {
    doc.data
        .downcast_ref::<Vec<Value>>()
        .ok_or_else(|| DomainError::internal("claude 编辑文档载荷类型不符"))
}

fn records_mut(doc: &mut EditDocument) -> DomainResult<&mut Vec<Value>> {
    doc.data
        .downcast_mut::<Vec<Value>>()
        .ok_or_else(|| DomainError::internal("claude 编辑文档载荷类型不符"))
}

fn handle(doc: &EditDocument) -> DomainResult<&PathBuf> {
    doc.handle
        .downcast_ref::<PathBuf>()
        .ok_or_else(|| DomainError::internal("claude 编辑文档句柄类型不符"))
}

/// 复制一份 span，避免持有对 `doc` 的不可变借用。
fn span_for(records: &[Value], selector: &Value) -> DomainResult<TurnSpan> {
    let spans = TURN_INDEX.turns(records);
    select_span(&spans, selector).cloned()
}

impl SessionEditor for ClaudeBackend {
    fn name(&self) -> &str {
        "claude"
    }

    fn operations(&self) -> &[&str] {
        OPERATIONS
    }

    fn load(&self, reference: &str) -> DomainResult<EditDocument> {
        let path = claude_edit::resolve(reference)?;
        let raw = std::fs::read(&path)
            .map_err(|error| DomainError::internal(format!("读取 claude 会话失败: {error}")))?;
        let data = claude_edit::load(&path)?;
        Ok(EditDocument::new(
            self.name(),
            reference,
            Box::new(path),
            Box::new(data),
            hash_bytes(&raw),
        ))
    }

    fn apply_ops(&self, doc: &mut EditDocument, ops: &[Value]) -> DomainResult<Vec<Event>> {
        let mut notes = Vec::new();
        for operation in ops {
            match operation.get("op").and_then(Value::as_str) {
                Some("delete-turn") => {
                    let turn = operation
                        .get("turn")
                        .cloned()
                        .ok_or_else(|| DomainError::internal("delete-turn 缺少 turn"))?;
                    let ordinal = positive_turn(&turn)?;
                    let span = span_for(records(doc)?, &Value::from(ordinal as i64))?;
                    notes.extend(CODEC.delete_turn(records_mut(doc)?, &span)?);
                }
                Some("rewrite") => {
                    let locator = operation
                        .get("locator")
                        .or_else(|| operation.get("uuid"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let text = operation
                        .get("text")
                        .and_then(Value::as_str)
                        .ok_or_else(|| DomainError::internal("rewrite 缺少 text"))?
                        .to_string();
                    notes.extend(CODEC.rewrite_message(records_mut(doc)?, &locator, &text)?);
                }
                other => {
                    return Err(DomainError::operation_unsupported(
                        "claude",
                        other.unwrap_or(""),
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
        let span = span_for(records(doc)?, turn)?;
        CODEC.replace_reply(records_mut(doc)?, &span, reply)
    }

    fn validate(&self, doc: &EditDocument) -> DomainResult<()> {
        claude_edit::check_invariants(records(doc)?)
    }

    fn stats(&self, doc: &EditDocument) -> DomainResult<Map<String, Value>> {
        let data = records(doc)?;
        let mut stats = Map::new();
        stats.insert("count".into(), Value::from(data.len() as i64));
        stats.insert(
            "size".into(),
            Value::from(json_size(&Value::Array(data.clone())) as i64),
        );
        Ok(stats)
    }

    fn snapshot(
        &self,
        doc: &EditDocument,
        reason_code: &str,
        extra: Option<&Map<String, Value>>,
    ) -> DomainResult<Option<PathBuf>> {
        Ok(Some(claude_edit::backup(
            handle(doc)?,
            reason_code,
            self.name(),
            extra,
        )?))
    }

    fn restore_snapshot(&self, snapshot: &Path, doc: &EditDocument) -> DomainResult<()> {
        std::fs::copy(snapshot, handle(doc)?)
            .map(|_| ())
            .map_err(|error| DomainError::internal(format!("claude 快照还原失败: {error}")))
    }

    fn commit(&self, doc: &mut EditDocument) -> DomainResult<Map<String, Value>> {
        let path = handle(doc)?.clone();
        let current = std::fs::read(&path)
            .map_err(|error| DomainError::internal(format!("读取 claude 会话失败: {error}")))?;
        if hash_bytes(&current) != doc.revision {
            return Err(DomainError::concurrent_modification(
                "源会话在预览后已变化，请重新预览",
            ));
        }
        let data = records(doc)?;
        claude_edit::save(&path, data)?;
        let cwd = data
            .iter()
            .filter_map(|record| record.get("cwd").and_then(Value::as_str))
            .find(|cwd| !cwd.is_empty())
            .unwrap_or(".")
            .to_string();
        let stem = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut result = Map::new();
        result.insert("session_id".into(), Value::from(stem.as_str()));
        result.insert(
            "saved_as".into(),
            Value::from(path.to_string_lossy().into_owned()),
        );
        result.insert(
            "resume".into(),
            Value::from(format!("cd {cwd} && claude --resume {stem}")),
        );
        Ok(result)
    }

    fn saved_revision(
        &self,
        result: &Map<String, Value>,
        doc: &EditDocument,
    ) -> DomainResult<String> {
        default_saved_revision(self.name(), result, doc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn session_file(root: &Path) -> PathBuf {
        let path = root.join("s.jsonl");
        let lines = [
            json!({"uuid": "u1", "parentUuid": null, "type": "user", "isSidechain": false,
                   "cwd": "/work", "message": {"role": "user", "content": "first"}}),
            json!({"uuid": "a1", "parentUuid": "u1", "type": "assistant", "isSidechain": false,
                   "cwd": "/work",
                   "message": {"type": "message", "role": "assistant",
                               "content": [{"type": "text", "text": "reply"}]}}),
            json!({"uuid": "u2", "parentUuid": "a1", "type": "user", "isSidechain": false,
                   "cwd": "/work", "message": {"role": "user", "content": "second"}}),
        ];
        let payload: String = lines
            .iter()
            .map(|line| format!("{}\n", serde_json::to_string(line).unwrap()))
            .collect();
        std::fs::write(&path, payload).unwrap();
        path
    }

    #[test]
    fn load_apply_validate_commit_round_trip() {
        let root = tempfile::tempdir().unwrap();
        let path = session_file(root.path());
        let backend = ClaudeBackend;
        let mut doc = backend.load(path.to_str().unwrap()).unwrap();
        assert_eq!(backend.stats(&doc).unwrap()["count"], json!(3));

        let changes = backend
            .apply_ops(&mut doc, &[json!({"op": "delete-turn", "turn": 1})])
            .unwrap();
        assert_eq!(changes[0].code, "edit.turn_deleted");
        backend.validate(&doc).unwrap();
        let result = backend.commit(&mut doc).unwrap();
        assert_eq!(result["session_id"], json!("s"));
        assert_eq!(result["resume"], json!("cd /work && claude --resume s"));
        assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 1);
    }

    #[test]
    fn commit_detects_concurrent_modification() {
        let root = tempfile::tempdir().unwrap();
        let path = session_file(root.path());
        let backend = ClaudeBackend;
        let mut doc = backend.load(path.to_str().unwrap()).unwrap();
        std::fs::write(&path, "{}\n").unwrap();
        let error = backend.commit(&mut doc).unwrap_err();
        assert_eq!(error.code, "session.concurrent_modification");
        assert_eq!(error.message(), "源会话在预览后已变化，请重新预览");
    }

    #[test]
    fn unknown_ops_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let path = session_file(root.path());
        let backend = ClaudeBackend;
        let mut doc = backend.load(path.to_str().unwrap()).unwrap();
        let error = backend
            .apply_ops(&mut doc, &[json!({"op": "nope"})])
            .unwrap_err();
        assert_eq!(error.code, "edit.operation_unsupported");
        assert_eq!(backend.operations(), OPERATIONS);
    }

    #[test]
    fn replace_reply_selects_by_ordinal_or_locator() {
        let root = tempfile::tempdir().unwrap();
        let path = session_file(root.path());
        let backend = ClaudeBackend;
        let mut doc = backend.load(path.to_str().unwrap()).unwrap();
        let reply = json!({"items": [{"kind": "text", "text": "new"}]});
        let changes = backend
            .replace_reply(&mut doc, &json!("u1"), &reply)
            .unwrap();
        assert_eq!(changes[0].params["turn"], json!(1));
        backend.validate(&doc).unwrap();
    }
}
