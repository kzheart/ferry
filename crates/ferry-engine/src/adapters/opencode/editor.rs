//! OpenCode 会话编辑后端：经官方 HTTP API 原地更新。
//!
//! 语义事实源：`engine/adapters/opencode/editor.py`。
//!
//! 三条硬约束：
//! 1. 读取只走只读 SQLite（`load` / `load_preview` 都不碰 CLI）；
//! 2. 写入只允许 `patch_part`——消息集合与 part 集合前后必须完全一致，
//!    因此 `delete-turn` 在 `apply_ops` 就被拒；
//! 3. 任何一条 patch 失败都要按**逆序**补偿回滚，补偿再失败就把两次错误一起报。

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::adapters::contracts::SessionEditor;
use crate::adapters::shared::codec::NativeEditCodec;
use crate::adapters::shared::editing::{json_size, EditDocument};
use crate::errors::{DomainError, DomainResult};
use crate::events::Event;
use crate::jsonutil::hash_bytes;
use crate::model::Session;
use crate::system::snapshots::snapshot_payload;

use super::api::{self, ApiFactory};
use super::codec::CODEC;
use super::{reader, store};

/// OpenCode 编辑文档的私有载荷（挂在 `EditDocument::data` 上）。
pub struct OpenCodeData {
    /// 可变的原生 export payload。
    pub data: Value,
    /// 载入时的原始副本，用于算变更集与快照。
    pub original: Value,
    /// canonical 会话树（delete-turn 需要同步摘掉子会话）。
    pub tree: Session,
}

/// 递归按 key 排序（`json.dumps(..., sort_keys=True)` 的等价物）。
fn sorted_value(value: &Value) -> Value {
    match value {
        Value::Object(entries) => {
            let mut keys: Vec<&String> = entries.keys().collect();
            keys.sort_unstable();
            Value::Object(
                keys.into_iter()
                    .map(|key| (key.clone(), sorted_value(&entries[key])))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(sorted_value).collect()),
        other => other.clone(),
    }
}

/// revision 的唯一来源：`json.dumps(payload, ensure_ascii=False, sort_keys=True)`。
///
/// 注意分隔符**带空格**（Python 的默认值），与 canonical_json 那套无空格的摘要
/// 序列化不是一回事，两者不可互换。
fn revision_of(payload: &Value) -> String {
    let sorted = sorted_value(payload);
    hash_bytes(crate::adapters::shared::writing::python_json_dumps(&sorted).as_bytes())
}

fn data_of(doc: &EditDocument) -> DomainResult<&OpenCodeData> {
    doc.data
        .downcast_ref::<OpenCodeData>()
        .ok_or_else(|| DomainError::internal("OpenCode 编辑文档载荷类型不符"))
}

fn data_of_mut(doc: &mut EditDocument) -> DomainResult<&mut OpenCodeData> {
    doc.data
        .downcast_mut::<OpenCodeData>()
        .ok_or_else(|| DomainError::internal("OpenCode 编辑文档载荷类型不符"))
}

/// `part_id → (message_id, part)`。
fn part_map(payload: &Value) -> Vec<(String, String, Value)> {
    let mut items = Vec::new();
    for message in payload
        .get("messages")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let message_id = message
            .get("info")
            .and_then(|info| info.get("id"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        for part in message
            .get("parts")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            if let Some(part_id) = part.get("id").and_then(Value::as_str) {
                items.push((part_id.to_string(), message_id.clone(), part.clone()));
            }
        }
    }
    items
}

fn part_ids(items: &[(String, String, Value)]) -> std::collections::BTreeSet<&str> {
    items.iter().map(|(id, _, _)| id.as_str()).collect()
}

fn message_ids(payload: &Value) -> std::collections::BTreeSet<String> {
    payload
        .get("messages")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|message| {
            message
                .get("info")
                .and_then(|info| info.get("id"))
                .cloned()
                .map_or_else(|| "null".to_string(), |value| value.to_string())
        })
        .collect()
}

/// OpenCode 的编辑后端。
pub struct OpenCodeBackend {
    api_factory: ApiFactory,
}

impl Default for OpenCodeBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenCodeBackend {
    pub fn new() -> Self {
        Self {
            api_factory: api::factory(),
        }
    }

    /// 注入自定义 API 工厂（单测用）。
    pub fn with_factory(api_factory: ApiFactory) -> Self {
        Self { api_factory }
    }

    fn document(&self, reference: &str, payload: Value, tree: Session) -> EditDocument {
        let revision = revision_of(&payload);
        EditDocument::new(
            "opencode",
            reference,
            Box::new(reference.to_string()),
            Box::new(OpenCodeData {
                data: payload.clone(),
                original: payload,
                tree,
            }),
            revision,
        )
    }

    fn client(&self, cwd: &str) -> DomainResult<Box<dyn api::OpenCodeApiClient>> {
        (self.api_factory)(cwd)
    }

    /// 逐条 patch，失败按逆序补偿；补偿也失败就把补偿错误一起抛出去。
    fn patch_with_compensation(
        client: &dyn api::OpenCodeApiClient,
        reference: &str,
        changes: &[(String, String, Value, Value)],
        failure_note: &str,
    ) -> DomainResult<()> {
        let mut applied: Vec<(String, Value)> = Vec::new();
        for (_, message_id, old_part, new_part) in changes {
            if let Err(error) = client.patch_part(reference, message_id, new_part) {
                let mut rollback_errors: Vec<String> = Vec::new();
                for (message_id, old_part) in applied.iter().rev() {
                    if let Err(rollback) = client.patch_part(reference, message_id, old_part) {
                        rollback_errors.push(rollback.message().to_string());
                    }
                }
                if !rollback_errors.is_empty() {
                    return Err(DomainError::internal(format!(
                        "{failure_note}: {}",
                        rollback_errors.join("; ")
                    )));
                }
                return Err(error);
            }
            applied.push((message_id.clone(), old_part.clone()));
        }
        Ok(())
    }
}

impl SessionEditor for OpenCodeBackend {
    fn name(&self) -> &str {
        "opencode"
    }

    fn operations(&self) -> &[&str] {
        // opencode 只能原地改写：官方 API 没有删消息的路由。
        &["rewrite"]
    }

    fn load(&self, reference: &str) -> DomainResult<EditDocument> {
        let payload = store::load_native_payload(reference)?;
        let tree = reader::read(reference)?;
        Ok(self.document(reference, payload, tree))
    }

    fn load_preview(&self, reference: &str) -> Option<DomainResult<EditDocument>> {
        Some((|| {
            let payload = store::load_native_payload(reference)?;
            let tree = reader::read_preview(reference)?;
            Ok(self.document(reference, payload, tree))
        })())
    }

    fn apply_ops(&self, doc: &mut EditDocument, ops: &[Value]) -> DomainResult<Vec<Event>> {
        let data = data_of_mut(doc)?;
        let mut notes = Vec::new();
        for op in ops {
            let kind = op.get("op").and_then(Value::as_str).unwrap_or("");
            match kind {
                "delete-turn" => {
                    return Err(DomainError::operation_unsupported(
                        "opencode",
                        "delete-turn",
                        Some("inplace"),
                    ))
                }
                "rewrite" => {
                    let locator = op
                        .get("locator")
                        .and_then(Value::as_str)
                        .or_else(|| op.get("uuid").and_then(Value::as_str))
                        .unwrap_or("");
                    let text = op.get("text").and_then(Value::as_str).unwrap_or("");
                    notes.extend(CODEC.rewrite_message(data, locator, text)?);
                }
                other => {
                    return Err(DomainError::operation_unsupported("opencode", other, None));
                }
            }
        }
        Ok(notes)
    }

    fn validate(&self, doc: &EditDocument) -> DomainResult<()> {
        let data = data_of(doc)?;
        let sid = data
            .data
            .get("info")
            .and_then(|info| info.get("id"))
            .cloned()
            .unwrap_or(Value::Null);
        let mut seen: Vec<String> = Vec::new();
        for message in data
            .data
            .get("messages")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            let info = message
                .get("info")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_else(Map::new);
            let mid = info.get("id").and_then(Value::as_str).unwrap_or("");
            if mid.is_empty() || seen.iter().any(|id| id == mid) {
                return Err(DomainError::internal("OpenCode message id 缺失或重复"));
            }
            seen.push(mid.to_string());
            if info.get("sessionID") != Some(&sid) {
                return Err(DomainError::internal("OpenCode message.sessionID 不一致"));
            }
            if info.get("role") == Some(&Value::from("assistant"))
                && !truthy(info.get("finish"))
                && !truthy(info.get("error"))
            {
                return Err(DomainError::internal(
                    "OpenCode assistant 消息缺少 finish/error 终态",
                ));
            }
            for part in message
                .get("parts")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default()
            {
                if part.get("messageID") != Some(&Value::from(mid))
                    || part.get("sessionID") != Some(&sid)
                {
                    return Err(DomainError::internal("OpenCode part 外键不一致"));
                }
            }
        }
        Ok(())
    }

    fn stats(&self, doc: &EditDocument) -> DomainResult<Map<String, Value>> {
        let data = data_of(doc)?;
        let mut stats = Map::new();
        stats.insert(
            "count".into(),
            Value::from(
                data.data
                    .get("messages")
                    .and_then(Value::as_array)
                    .map(Vec::len)
                    .unwrap_or(0),
            ),
        );
        stats.insert("size".into(), Value::from(json_size(&data.data)));
        Ok(stats)
    }

    fn snapshot(
        &self,
        doc: &EditDocument,
        reason_code: &str,
        extra: Option<&Map<String, Value>>,
    ) -> DomainResult<Option<PathBuf>> {
        let data = data_of(doc)?;
        let payload = format!(
            "{}\n",
            crate::adapters::shared::writing::python_json_dumps(&data.original)
        );
        snapshot_payload(
            &doc.reference,
            &payload,
            reason_code,
            "opencode",
            &doc.reference,
            extra,
        )
        .map(Some)
        .map_err(|error| DomainError::internal(format!("OpenCode 快照写入失败: {error}")))
    }

    fn restore_snapshot(&self, snapshot: &Path, doc: &EditDocument) -> DomainResult<()> {
        let raw = std::fs::read_to_string(snapshot)
            .map_err(|error| DomainError::internal(format!("OpenCode 快照不可读: {error}")))?;
        let original: Value = serde_json::from_str(&raw)
            .map_err(|error| DomainError::internal(format!("OpenCode 快照非法: {error}")))?;
        let current = store::export_session(&doc.reference)?;
        let current_parts = part_map(&current);
        let original_parts = part_map(&original);
        if part_ids(&current_parts) != part_ids(&original_parts) {
            return Err(DomainError::internal(
                "OpenCode 快照包含消息增删，当前官方 API 无法安全原地恢复",
            ));
        }
        let cwd = original
            .get("info")
            .and_then(|info| info.get("directory"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or(".")
            .to_string();

        let changes: Vec<(String, String, Value, Value)> = original_parts
            .iter()
            .filter_map(|(part_id, message_id, part)| {
                let current_part = current_parts
                    .iter()
                    .find(|(id, _, _)| id == part_id)
                    .map(|(_, _, part)| part.clone())?;
                (current_part != *part).then(|| {
                    (
                        part_id.clone(),
                        message_id.clone(),
                        current_part,
                        part.clone(),
                    )
                })
            })
            .collect();

        {
            let client = self.client(&cwd)?;
            Self::patch_with_compensation(
                client.as_ref(),
                &doc.reference,
                &changes,
                "OpenCode 快照恢复失败且补偿回滚不完整",
            )?;
        }

        let restored = part_map(&store::export_session(&doc.reference)?);
        let drifted = original_parts.iter().any(|(part_id, _, part)| {
            restored
                .iter()
                .find(|(id, _, _)| id == part_id)
                .map(|(_, _, restored)| restored != part)
                .unwrap_or(true)
        });
        if drifted {
            return Err(DomainError::internal("OpenCode 快照恢复后静态校验失败"));
        }
        Ok(())
    }

    fn commit(&self, doc: &mut EditDocument) -> DomainResult<Map<String, Value>> {
        let expected_revision = doc.revision.clone();
        let reference = doc.reference.clone();
        let data = data_of(doc)?;
        let fresh = store::export_session(&reference)?;
        if revision_of(&fresh) != expected_revision {
            return Err(DomainError::concurrent_modification(
                "源会话在预览后已变化，请重新预览",
            ));
        }
        let before = part_map(&data.original);
        let after = part_map(&data.data);
        if message_ids(&data.original) != message_ids(&data.data)
            || part_ids(&before) != part_ids(&after)
        {
            return Err(DomainError::internal("OpenCode 当前不支持安全原地删除整轮"));
        }
        let changes: Vec<(String, String, Value, Value)> = before
            .iter()
            .filter_map(|(part_id, _, old_part)| {
                let (_, message_id, new_part) =
                    after.iter().find(|(id, _, _)| id == part_id)?.clone();
                (old_part != &new_part).then_some((
                    part_id.clone(),
                    message_id,
                    old_part.clone(),
                    new_part,
                ))
            })
            .collect();
        let cwd = data
            .data
            .get("info")
            .and_then(|info| info.get("directory"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or(".")
            .to_string();

        {
            let client = self.client(&cwd)?;
            if !client.supports_part_patch()? {
                return Err(DomainError::internal(
                    "当前 OpenCode server 不支持官方 part 更新 API",
                ));
            }
            client.assert_idle(&reference)?;
            Self::patch_with_compensation(
                client.as_ref(),
                &reference,
                &changes,
                "OpenCode API 更新失败且补偿回滚不完整",
            )?;
        }

        let mut result = Map::new();
        result.insert("session_id".into(), Value::from(reference.as_str()));
        result.insert(
            "saved_as".into(),
            Value::from(store::database_path().to_string_lossy().into_owned()),
        );
        result.insert(
            "resume".into(),
            Value::from(format!("cd {cwd} && opencode -s {reference}")),
        );
        result.insert("updated_parts".into(), Value::from(changes.len()));
        Ok(result)
    }

    fn saved_revision(
        &self,
        result: &Map<String, Value>,
        _doc: &EditDocument,
    ) -> DomainResult<String> {
        let session_id = result
            .get("session_id")
            .and_then(Value::as_str)
            .ok_or_else(|| DomainError::internal("OpenCode 提交结果缺少 session_id"))?;
        Ok(revision_of(&store::export_session(session_id)?))
    }
}

/// Python `bool(value)` 的 JSON 等价。
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

/// 单测用的假客户端工厂：只记录调用，不起进程。
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    pub(crate) struct Recorder {
        pub patches: Mutex<Vec<(String, String, Value)>>,
        pub fail_on: Mutex<Option<usize>>,
        pub fail_rollback: Mutex<bool>,
        pub supports: Mutex<bool>,
        pub busy: Mutex<bool>,
    }

    pub(crate) struct Client(pub Arc<Recorder>);

    impl api::OpenCodeApiClient for Client {
        fn supports_part_patch(&self) -> DomainResult<bool> {
            Ok(*self.0.supports.lock().unwrap())
        }

        fn patch_part(
            &self,
            session_id: &str,
            message_id: &str,
            part: &Value,
        ) -> DomainResult<Value> {
            let mut patches = self.0.patches.lock().unwrap();
            let index = patches.len();
            let failing = *self.0.fail_on.lock().unwrap();
            patches.push((session_id.into(), message_id.into(), part.clone()));
            if failing == Some(index) {
                return Err(DomainError::internal("patch 失败"));
            }
            if failing.is_some_and(|failing| index > failing)
                && *self.0.fail_rollback.lock().unwrap()
            {
                return Err(DomainError::internal("回滚失败"));
            }
            Ok(Value::Null)
        }

        fn assert_idle(&self, session_id: &str) -> DomainResult<()> {
            if *self.0.busy.lock().unwrap() {
                return Err(DomainError::internal(format!(
                    "OpenCode 会话 {session_id} 正在运行，拒绝原地编辑"
                )));
            }
            Ok(())
        }
    }

    pub(crate) fn recorder() -> (Arc<Recorder>, ApiFactory) {
        let recorder = Arc::new(Recorder {
            supports: Mutex::new(true),
            ..Recorder::default()
        });
        let shared = recorder.clone();
        let factory: ApiFactory = Arc::new(move |_cwd: &str| {
            Ok(Box::new(Client(shared.clone())) as Box<dyn api::OpenCodeApiClient>)
        });
        (recorder, factory)
    }
}

#[cfg(test)]
mod tests {
    use super::testing::recorder;
    use super::*;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    /// store 的 CLI 替身：`export_session` 返回预置 payload。
    struct StubCli {
        exports: Mutex<Vec<Value>>,
    }

    impl store::NativeCli for StubCli {
        fn run_command(&self, _args: &[&str], _cwd: Option<&Path>) -> DomainResult<String> {
            Ok(String::new())
        }
        fn export_session(&self, _session_id: &str) -> DomainResult<Value> {
            let mut exports = self.exports.lock().unwrap();
            if exports.len() > 1 {
                Ok(exports.remove(0))
            } else {
                Ok(exports[0].clone())
            }
        }
        fn import_payload(&self, _: &Value, _: &str, _: &str) -> DomainResult<()> {
            Ok(())
        }
        fn delete_session(&self, _: &str, _: Option<&str>) -> DomainResult<()> {
            Ok(())
        }
    }

    fn payload(text: &str) -> Value {
        json!({
            "info": {"id": "session-1", "directory": "/work"},
            "messages": [{
                "info": {"id": "message-1", "sessionID": "session-1", "role": "user"},
                "parts": [{"id": "part-1", "messageID": "message-1",
                           "sessionID": "session-1", "type": "text", "text": text}]
            }]
        })
    }

    fn document(payload: Value) -> EditDocument {
        let revision = revision_of(&payload);
        EditDocument::new(
            "opencode",
            "session-1",
            Box::new("session-1".to_string()),
            Box::new(OpenCodeData {
                data: payload.clone(),
                original: payload,
                tree: Session::new("opencode", "session-1", "/work"),
            }),
            revision,
        )
    }

    #[test]
    fn delete_turn_is_rejected_before_touching_the_document() {
        let backend = OpenCodeBackend::new();
        let mut doc = document(payload("original"));
        let error = backend
            .apply_ops(&mut doc, &[json!({"op": "delete-turn", "turn": 1})])
            .unwrap_err();
        assert_eq!(error.code, "edit.operation_unsupported");
        assert_eq!(error.params()["mode"], json!("inplace"));

        let error = backend
            .apply_ops(&mut doc, &[json!({"op": "replace-assistant-reply"})])
            .unwrap_err();
        assert_eq!(
            error.params()["operation"],
            json!("replace-assistant-reply")
        );
    }

    #[test]
    fn rewrite_accepts_both_locator_and_uuid_keys() {
        let backend = OpenCodeBackend::new();
        let mut doc = document(payload("original"));
        let notes = backend
            .apply_ops(
                &mut doc,
                &[json!({"op": "rewrite", "uuid": "message-1", "text": "changed"})],
            )
            .unwrap();
        assert_eq!(notes[0].code, "edit.message_rewritten");
        let data = data_of(&doc).unwrap();
        assert_eq!(
            data.data["messages"][0]["parts"][0]["text"],
            json!("changed")
        );
        // original 不受影响。
        assert_eq!(
            data.original["messages"][0]["parts"][0]["text"],
            json!("original")
        );
    }

    #[test]
    fn validate_enforces_ids_foreign_keys_and_assistant_terminal_state() {
        let backend = OpenCodeBackend::new();
        assert!(backend.validate(&document(payload("x"))).is_ok());

        let mut broken = payload("x");
        broken["messages"][0]["parts"][0]["sessionID"] = json!("other");
        let error = backend.validate(&document(broken)).unwrap_err();
        assert_eq!(error.message(), "OpenCode part 外键不一致");

        let mut assistant = payload("x");
        assistant["messages"][0]["info"]["role"] = json!("assistant");
        let error = backend.validate(&document(assistant)).unwrap_err();
        assert_eq!(
            error.message(),
            "OpenCode assistant 消息缺少 finish/error 终态"
        );

        let mut duplicated = payload("x");
        let message = duplicated["messages"][0].clone();
        duplicated["messages"].as_array_mut().unwrap().push(message);
        let error = backend.validate(&document(duplicated)).unwrap_err();
        assert_eq!(error.message(), "OpenCode message id 缺失或重复");
    }

    #[test]
    fn commit_patches_only_the_changed_parts() {
        let _guard = store::tests::exclusive();
        store::install_cli(Arc::new(StubCli {
            exports: Mutex::new(vec![payload("original")]),
        }));
        let (recorder, factory) = recorder();
        let backend = OpenCodeBackend::with_factory(factory);
        let mut doc = document(payload("original"));
        backend
            .apply_ops(
                &mut doc,
                &[json!({"op": "rewrite", "locator": "message-1", "text": "changed"})],
            )
            .unwrap();
        let result = backend.commit(&mut doc).unwrap();
        store::reset_cli();

        assert_eq!(result["session_id"], json!("session-1"));
        assert_eq!(result["updated_parts"], json!(1));
        assert_eq!(result["resume"], json!("cd /work && opencode -s session-1"));
        let patches = recorder.patches.lock().unwrap();
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].1, "message-1");
        assert_eq!(patches[0].2["text"], json!("changed"));
    }

    #[test]
    fn commit_refuses_when_the_source_changed_after_preview() {
        let _guard = store::tests::exclusive();
        store::install_cli(Arc::new(StubCli {
            exports: Mutex::new(vec![payload("drifted")]),
        }));
        let (_, factory) = recorder();
        let backend = OpenCodeBackend::with_factory(factory);
        let mut doc = document(payload("original"));
        let error = backend.commit(&mut doc).unwrap_err();
        store::reset_cli();
        assert_eq!(error.code, "session.concurrent_modification");
    }

    #[test]
    fn commit_refuses_message_or_part_set_changes() {
        let _guard = store::tests::exclusive();
        store::install_cli(Arc::new(StubCli {
            exports: Mutex::new(vec![payload("original")]),
        }));
        let (_, factory) = recorder();
        let backend = OpenCodeBackend::with_factory(factory);
        let mut doc = document(payload("original"));
        data_of_mut(&mut doc).unwrap().data["messages"]
            .as_array_mut()
            .unwrap()
            .clear();
        let error = backend.commit(&mut doc).unwrap_err();
        store::reset_cli();
        assert_eq!(error.message(), "OpenCode 当前不支持安全原地删除整轮");
    }

    #[test]
    fn a_failed_patch_rolls_back_in_reverse_order() {
        let two_parts = json!({
            "info": {"id": "session-1", "directory": "/work"},
            "messages": [{
                "info": {"id": "m1", "sessionID": "session-1", "role": "user"},
                "parts": [
                    {"id": "p1", "messageID": "m1", "sessionID": "session-1",
                     "type": "text", "text": "a"},
                    {"id": "p2", "messageID": "m1", "sessionID": "session-1",
                     "type": "text", "text": "b"}
                ]
            }]
        });
        let _guard = store::tests::exclusive();
        store::install_cli(Arc::new(StubCli {
            exports: Mutex::new(vec![two_parts.clone()]),
        }));
        let (recorder, factory) = recorder();
        *recorder.fail_on.lock().unwrap() = Some(1);
        let backend = OpenCodeBackend::with_factory(factory);
        let mut doc = document(two_parts);
        {
            let data = data_of_mut(&mut doc).unwrap();
            data.data["messages"][0]["parts"][0]["text"] = json!("a2");
            data.data["messages"][0]["parts"][1]["text"] = json!("b2");
        }
        let error = backend.commit(&mut doc).unwrap_err();
        store::reset_cli();
        assert_eq!(error.message(), "patch 失败");
        let patches = recorder.patches.lock().unwrap();
        // 两次正向 patch（第二次失败）+ 一次补偿回滚。
        assert_eq!(patches.len(), 3);
        assert_eq!(patches[2].2["id"], json!("p1"));
        assert_eq!(patches[2].2["text"], json!("a"));
    }

    #[test]
    fn an_incomplete_rollback_reports_both_failures() {
        let two_parts = json!({
            "info": {"id": "session-1", "directory": "/work"},
            "messages": [{
                "info": {"id": "m1", "sessionID": "session-1", "role": "user"},
                "parts": [
                    {"id": "p1", "messageID": "m1", "sessionID": "session-1",
                     "type": "text", "text": "a"},
                    {"id": "p2", "messageID": "m1", "sessionID": "session-1",
                     "type": "text", "text": "b"}
                ]
            }]
        });
        let _guard = store::tests::exclusive();
        store::install_cli(Arc::new(StubCli {
            exports: Mutex::new(vec![two_parts.clone()]),
        }));
        let (recorder, factory) = recorder();
        *recorder.fail_on.lock().unwrap() = Some(1);
        *recorder.fail_rollback.lock().unwrap() = true;
        let backend = OpenCodeBackend::with_factory(factory);
        let mut doc = document(two_parts);
        {
            let data = data_of_mut(&mut doc).unwrap();
            data.data["messages"][0]["parts"][0]["text"] = json!("a2");
            data.data["messages"][0]["parts"][1]["text"] = json!("b2");
        }
        let error = backend.commit(&mut doc).unwrap_err();
        store::reset_cli();
        assert!(error
            .message()
            .starts_with("OpenCode API 更新失败且补偿回滚不完整: "));
    }

    #[test]
    fn commit_refuses_a_server_without_the_part_patch_route() {
        let _guard = store::tests::exclusive();
        store::install_cli(Arc::new(StubCli {
            exports: Mutex::new(vec![payload("original")]),
        }));
        let (recorder, factory) = recorder();
        *recorder.supports.lock().unwrap() = false;
        let backend = OpenCodeBackend::with_factory(factory);
        let mut doc = document(payload("original"));
        let error = backend.commit(&mut doc).unwrap_err();
        store::reset_cli();
        assert_eq!(
            error.message(),
            "当前 OpenCode server 不支持官方 part 更新 API"
        );
    }

    #[test]
    fn a_busy_session_refuses_in_place_edits() {
        let _guard = store::tests::exclusive();
        store::install_cli(Arc::new(StubCli {
            exports: Mutex::new(vec![payload("original")]),
        }));
        let (recorder, factory) = recorder();
        *recorder.busy.lock().unwrap() = true;
        let backend = OpenCodeBackend::with_factory(factory);
        let mut doc = document(payload("original"));
        let error = backend.commit(&mut doc).unwrap_err();
        store::reset_cli();
        assert!(error.message().contains("正在运行，拒绝原地编辑"));
    }

    #[test]
    fn restore_snapshot_skips_writing_when_nothing_drifted() {
        let root = tempfile::tempdir().unwrap();
        let snapshot = root.path().join("snapshot.jsonl");
        // U+0085 等 Unicode 行分隔符必须按 JSON 读回，不能按行切分。
        let stored = payload("before\u{85}after");
        std::fs::write(
            &snapshot,
            format!("{}\n", serde_json::to_string(&stored).unwrap()),
        )
        .unwrap();
        let _guard = store::tests::exclusive();
        store::install_cli(Arc::new(StubCli {
            exports: Mutex::new(vec![stored.clone()]),
        }));
        let (recorder, factory) = recorder();
        let backend = OpenCodeBackend::with_factory(factory);
        backend
            .restore_snapshot(&snapshot, &document(stored))
            .unwrap();
        store::reset_cli();
        assert!(recorder.patches.lock().unwrap().is_empty());
    }

    #[test]
    fn restore_snapshot_refuses_a_changed_part_set() {
        let root = tempfile::tempdir().unwrap();
        let snapshot = root.path().join("snapshot.jsonl");
        std::fs::write(
            &snapshot,
            serde_json::to_string(&payload("before")).unwrap(),
        )
        .unwrap();
        let mut current = payload("before");
        current["messages"][0]["parts"][0]["id"] = json!("other-part");
        let _guard = store::tests::exclusive();
        store::install_cli(Arc::new(StubCli {
            exports: Mutex::new(vec![current]),
        }));
        let (_, factory) = recorder();
        let backend = OpenCodeBackend::with_factory(factory);
        let error = backend
            .restore_snapshot(&snapshot, &document(payload("before")))
            .unwrap_err();
        store::reset_cli();
        assert_eq!(
            error.message(),
            "OpenCode 快照包含消息增删，当前官方 API 无法安全原地恢复"
        );
    }

    #[test]
    fn stats_report_message_count_and_serialized_size() {
        let doc = document(payload("x"));
        let stats = OpenCodeBackend::new().stats(&doc).unwrap();
        assert_eq!(stats["count"], json!(1));
        assert!(stats["size"].as_u64().unwrap() > 0);
    }
}
