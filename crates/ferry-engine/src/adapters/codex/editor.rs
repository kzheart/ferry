//! Codex 会话编辑后端：delete-turn / rewrite / replace-assistant-reply。

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::adapters::contracts::SessionEditor;
use crate::adapters::shared::codec::{positive_turn, select_span, NativeEditCodec, TurnIndex};
use crate::adapters::shared::editing::{
    default_saved_revision, hash_bytes, json_size, write_jsonl, EditDocument,
};
use crate::adapters::shared::scanner::split_jsonl_lines;
use crate::errors::{DomainError, DomainResult};
use crate::events::Event;
use crate::system::paths::expanduser;
use crate::system::snapshots::snapshot_file;

use super::codec::{CALL_SUBTYPES, CODEC, OUTPUT_SUBTYPES, TURN_INDEX};
use super::native::{self, CodexClosure, CodexStore};

/// 把引用解析成 rollout 路径：优先当作路径，否则按 session id 检索。
pub fn resolve(reference: &str) -> DomainResult<PathBuf> {
    let direct = Path::new(reference);
    if direct.exists() {
        return Ok(direct.to_path_buf());
    }
    let pattern = expanduser(&format!(
        "~/.codex/sessions/*/*/*/rollout-*-{reference}.jsonl"
    ));
    let mut hits: Vec<PathBuf> = glob::glob(&pattern.to_string_lossy())
        .map(|paths| paths.filter_map(Result::ok).collect())
        .unwrap_or_default();
    hits.sort();
    hits.into_iter()
        .next()
        .ok_or_else(|| DomainError::session_not_found("codex", reference))
}

/// Codex 编辑后端。
pub struct CodexBackend;

const OPERATIONS: [&str; 3] = ["delete-turn", "rewrite", "replace-assistant-reply"];

impl CodexBackend {
    fn load_document(&self, reference: &str, recover: bool) -> DomainResult<EditDocument> {
        let path = resolve(reference)?;
        let store = CodexStore::for_rollout(&path);
        if recover {
            native::recover_transactions(&store);
        }
        let raw =
            fs::read(&path).map_err(|_| DomainError::session_not_found("codex", reference))?;
        let text = std::str::from_utf8(&raw)
            .map_err(|error| DomainError::internal(format!("Codex 会话不是合法 UTF-8: {error}")))?;
        let mut records: Vec<Value> = Vec::new();
        for line in split_jsonl_lines(text) {
            if line.trim().is_empty() {
                continue;
            }
            records.push(
                serde_json::from_str(line).map_err(|error| {
                    DomainError::internal(format!("Codex 会话解析失败: {error}"))
                })?,
            );
        }
        let closure = native::discover_closure(&path, Some(store))?;
        let mut doc = EditDocument::new(
            "codex",
            reference,
            Box::new(path),
            Box::new(records),
            hash_bytes(&raw),
        );
        doc.context = Some(Box::new(closure));
        Ok(doc)
    }

    fn handle(doc: &EditDocument) -> &PathBuf {
        doc.handle
            .downcast_ref::<PathBuf>()
            .expect("Codex 编辑文档承载 PathBuf")
    }

    fn records(doc: &EditDocument) -> &Vec<Value> {
        doc.data
            .downcast_ref::<Vec<Value>>()
            .expect("Codex 编辑文档承载 Vec<Value>")
    }
}

impl SessionEditor for CodexBackend {
    fn name(&self) -> &str {
        "codex"
    }

    fn operations(&self) -> &[&str] {
        &OPERATIONS
    }

    fn load(&self, reference: &str) -> DomainResult<EditDocument> {
        self.load_document(reference, true)
    }

    fn load_preview(&self, reference: &str) -> Option<DomainResult<EditDocument>> {
        Some(self.load_document(reference, false))
    }

    fn apply_ops(&self, doc: &mut EditDocument, ops: &[Value]) -> DomainResult<Vec<Event>> {
        let mut notes = Vec::new();
        for op in ops {
            match op.get("op").and_then(Value::as_str).unwrap_or("") {
                "delete-turn" => {
                    // Python 先 `int(op["turn"])` 再走 positive_turn：字符串数字可接受。
                    let raw = op.get("turn").cloned().unwrap_or(Value::Null);
                    let numeric = match &raw {
                        Value::String(text) => text
                            .trim()
                            .parse::<i64>()
                            .map(Value::from)
                            .unwrap_or(Value::Null),
                        other => other.clone(),
                    };
                    let ordinal = positive_turn(&numeric)?;
                    let span = {
                        let spans = TURN_INDEX.turns(Self::records(doc));
                        select_span(&spans, &Value::from(ordinal as i64))?.clone()
                    };
                    notes.extend(CODEC.delete_turn(doc, &span)?);
                }
                "rewrite" => {
                    let locator = op
                        .get("locator")
                        .and_then(Value::as_str)
                        .filter(|value| !value.is_empty())
                        .or_else(|| {
                            op.get("uuid")
                                .and_then(Value::as_str)
                                .filter(|value| !value.is_empty())
                        })
                        .unwrap_or("")
                        .to_string();
                    let text = op.get("text").and_then(Value::as_str).unwrap_or("");
                    notes.extend(CODEC.rewrite_message(doc, &locator, text)?);
                }
                other => {
                    return Err(DomainError::operation_unsupported("codex", other, None));
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
        let span = {
            let spans = TURN_INDEX.turns(Self::records(doc));
            select_span(&spans, turn)?.clone()
        };
        CODEC.replace_reply(doc, &span, reply)
    }

    fn validate(&self, doc: &EditDocument) -> DomainResult<()> {
        let mut calls: BTreeSet<String> = BTreeSet::new();
        let mut outputs: BTreeSet<String> = BTreeSet::new();
        let mut metas = 0usize;
        for record in Self::records(doc) {
            if record.get("type").and_then(Value::as_str) == Some("session_meta") {
                metas += 1;
            }
            let empty = Map::new();
            let payload = record
                .get("payload")
                .and_then(Value::as_object)
                .unwrap_or(&empty);
            let subtype = payload.get("type").and_then(Value::as_str).unwrap_or("");
            let call_id = payload
                .get("call_id")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty());
            if let Some(call_id) = call_id {
                if CALL_SUBTYPES.contains(&subtype) {
                    calls.insert(call_id.to_string());
                } else if OUTPUT_SUBTYPES.contains(&subtype) {
                    outputs.insert(call_id.to_string());
                }
            }
            if subtype == "message" {
                let role = payload.get("role").and_then(Value::as_str).unwrap_or("");
                let allowed: &[&str] = match role {
                    "user" => &["input_text", "input_image"],
                    "assistant" => &["output_text"],
                    // Codex rollout 用 developer/system 消息携带系统指令。
                    "developer" | "system" => &["input_text"],
                    _ => &[],
                };
                let bad = payload
                    .get("content")
                    .and_then(Value::as_array)
                    .is_some_and(|items| {
                        items.iter().any(|block| {
                            !allowed
                                .contains(&block.get("type").and_then(Value::as_str).unwrap_or(""))
                        })
                    });
                if allowed.is_empty() || bad {
                    return Err(DomainError::internal(format!(
                        "Codex {role} 消息内容类型错误"
                    )));
                }
            }
        }
        if metas < 1 {
            return Err(DomainError::internal("Codex 会话缺少 session_meta"));
        }
        if calls != outputs {
            let only_calls: Vec<&str> = calls.difference(&outputs).map(String::as_str).collect();
            let only_outputs: Vec<&str> = outputs.difference(&calls).map(String::as_str).collect();
            return Err(DomainError::internal(format!(
                "Codex 工具调用未配对: call-only={only_calls:?}, output-only={only_outputs:?}"
            )));
        }
        Ok(())
    }

    fn stats(&self, doc: &EditDocument) -> DomainResult<Map<String, Value>> {
        let records = Self::records(doc);
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
        snapshot_file(Self::handle(doc), reason_code, "codex", extra)
            .map(Some)
            .map_err(|error| DomainError::internal(format!("Codex 会话快照失败: {error}")))
    }

    fn restore_snapshot(&self, snapshot: &Path, doc: &EditDocument) -> DomainResult<()> {
        fs::copy(snapshot, Self::handle(doc))
            .map(|_| ())
            .map_err(|error| DomainError::internal(format!("Codex 会话回滚失败: {error}")))
    }

    fn commit(&self, doc: &mut EditDocument) -> DomainResult<Map<String, Value>> {
        let pruned = doc
            .context
            .as_ref()
            .and_then(|context| context.downcast_ref::<CodexClosure>())
            .is_some_and(|closure| !closure.pruned_ids.is_empty());
        if pruned {
            return Err(DomainError::internal(
                "该轮包含 Codex 子 Agent，不支持安全原地编辑",
            ));
        }
        let path = Self::handle(doc).clone();
        let current = fs::read(&path)
            .map_err(|error| DomainError::internal(format!("Codex 会话读取失败: {error}")))?;
        if hash_bytes(&current) != doc.revision {
            return Err(DomainError::concurrent_modification(
                "源会话在预览后已变化，请重新预览",
            ));
        }
        let records = Self::records(doc).clone();
        write_jsonl(&path, &records)
            .map_err(|error| DomainError::internal(format!("Codex 会话写入失败: {error}")))?;
        let sid = records
            .iter()
            .find(|record| record.get("type").and_then(Value::as_str) == Some("session_meta"))
            .and_then(|record| record.get("payload"))
            .and_then(|payload| payload.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| DomainError::internal("Codex 会话缺少 session_meta"))?
            .to_string();
        let mut result = Map::new();
        result.insert("session_id".into(), Value::from(sid.as_str()));
        result.insert(
            "saved_as".into(),
            Value::from(path.to_string_lossy().into_owned()),
        );
        result.insert("resume".into(), Value::from(format!("codex resume {sid}")));
        Ok(result)
    }

    fn saved_revision(
        &self,
        result: &Map<String, Value>,
        doc: &EditDocument,
    ) -> DomainResult<String> {
        default_saved_revision("codex", result, doc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sessions_dir(root: &Path) -> PathBuf {
        let dir = root.join(".codex").join("sessions").join("2026/07/25");
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_rollout(dir: &Path, name: &str, records: &[Value]) -> PathBuf {
        let path = dir.join(name);
        let payload: String = records
            .iter()
            .map(|record| serde_json::to_string(record).unwrap() + "\n")
            .collect();
        fs::write(&path, payload).unwrap();
        path
    }

    fn base(id: &str) -> Vec<Value> {
        vec![
            json!({"type": "session_meta", "payload": {"id": id, "cwd": "/w"}}),
            json!({"type": "response_item", "payload": {"type": "message", "role": "user",
                   "content": [{"type": "input_text", "text": "one"}]}}),
            json!({"type": "response_item", "payload": {"type": "message", "role": "assistant",
                   "content": [{"type": "output_text", "text": "a"}]}}),
            json!({"type": "response_item", "payload": {"type": "message", "role": "user",
                   "content": [{"type": "input_text", "text": "two"}]}}),
        ]
    }

    #[test]
    fn delete_turn_and_commit_rewrite_the_file() {
        let temp = tempfile::tempdir().unwrap();
        let dir = sessions_dir(temp.path());
        let path = write_rollout(&dir, "rollout-a.jsonl", &base("a"));
        let mut doc = CodexBackend.load(path.to_str().unwrap()).unwrap();
        assert_eq!(CodexBackend.stats(&doc).unwrap()["count"], json!(4));
        let notes = CodexBackend
            .apply_ops(&mut doc, &[json!({"op": "delete-turn", "turn": 1})])
            .unwrap();
        assert_eq!(notes[0].code, "edit.turn_deleted");
        CodexBackend.validate(&doc).unwrap();
        let result = CodexBackend.commit(&mut doc).unwrap();
        assert_eq!(result["session_id"], json!("a"));
        assert_eq!(result["resume"], json!("codex resume a"));
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("\"one\""));
        assert!(text.contains("\"two\""));
    }

    #[test]
    fn commit_detects_concurrent_modification() {
        let temp = tempfile::tempdir().unwrap();
        let dir = sessions_dir(temp.path());
        let path = write_rollout(&dir, "rollout-a.jsonl", &base("a"));
        let mut doc = CodexBackend.load(path.to_str().unwrap()).unwrap();
        fs::write(
            &path,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"a\"}}\n",
        )
        .unwrap();
        let error = CodexBackend.commit(&mut doc).unwrap_err();
        assert_eq!(error.code, "session.concurrent_modification");
    }

    #[test]
    fn commit_refuses_turns_that_own_subagents() {
        let temp = tempfile::tempdir().unwrap();
        let dir = sessions_dir(temp.path());
        let mut records = base("root");
        records.insert(
            2,
            json!({"type": "response_item", "payload": {
                "type": "function_call", "call_id": "c1", "name": "spawn_agent",
                "arguments": "{\"agent_thread_id\": \"child\"}"}}),
        );
        let path = write_rollout(&dir, "rollout-root.jsonl", &records);
        write_rollout(
            &dir,
            "rollout-child.jsonl",
            &[json!({"type": "session_meta",
                     "payload": {"id": "child", "parent_thread_id": "root", "cwd": "/w"}})],
        );
        let mut doc = CodexBackend.load(path.to_str().unwrap()).unwrap();
        let notes = CodexBackend
            .apply_ops(&mut doc, &[json!({"op": "delete-turn", "turn": 1})])
            .unwrap();
        assert_eq!(notes[0].code, "edit.turn_deleted_with_children");
        let error = CodexBackend.commit(&mut doc).unwrap_err();
        assert!(error.message().contains("不支持安全原地编辑"));
    }

    #[test]
    fn validation_enforces_pairing_and_content_types() {
        let temp = tempfile::tempdir().unwrap();
        let dir = sessions_dir(temp.path());
        let mut records = base("a");
        records.push(json!({"type": "response_item", "payload": {
            "type": "custom_tool_call", "call_id": "c1", "name": "exec", "input": ""}}));
        let path = write_rollout(&dir, "rollout-a.jsonl", &records);
        let doc = CodexBackend.load(path.to_str().unwrap()).unwrap();
        let error = CodexBackend.validate(&doc).unwrap_err();
        assert!(error.message().contains("工具调用未配对"));

        // 角色与内容类型不匹配。
        let bad = vec![
            json!({"type": "session_meta", "payload": {"id": "b", "cwd": "/w"}}),
            json!({"type": "response_item", "payload": {"type": "message", "role": "assistant",
                   "content": [{"type": "input_text", "text": "x"}]}}),
        ];
        let path = write_rollout(&dir, "rollout-b.jsonl", &bad);
        let doc = CodexBackend.load(path.to_str().unwrap()).unwrap();
        assert!(CodexBackend
            .validate(&doc)
            .unwrap_err()
            .message()
            .contains("消息内容类型错误"));
    }

    #[test]
    fn unsupported_operations_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let dir = sessions_dir(temp.path());
        let path = write_rollout(&dir, "rollout-a.jsonl", &base("a"));
        let mut doc = CodexBackend.load(path.to_str().unwrap()).unwrap();
        let error = CodexBackend
            .apply_ops(&mut doc, &[json!({"op": "relink"})])
            .unwrap_err();
        assert_eq!(error.code, "edit.operation_unsupported");
    }

    #[test]
    fn missing_references_report_session_not_found() {
        let error = resolve("definitely-not-a-session").unwrap_err();
        assert_eq!(error.code, "session.not_found");
    }
}
