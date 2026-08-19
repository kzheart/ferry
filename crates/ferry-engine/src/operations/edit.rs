//! 会话编辑计划与写入处理。
//!
//! 编辑事务顺序是硬约束（§2.4 第 23 条）：
//! `load → 比 expected_revision → mutate → validate → snapshot（无快照拒写）
//!  → commit → saved_revision`；`ConcurrentModificationError` **不**还原快照，
//! 其他任何异常都必须还原。
//!
//! `bounded_json` / `finalize_dto` / `truncate_text` / `python_json` 只有一份
//! 实现（`crate::sessions::safety`），本模块直接复用。注意 `python_json` 用的是
//! **带空格**的分隔符（`", "` / `": "`），与 `jsonutil::canonical_json` 的无空格
//! 分隔符不是一回事：体积判定用前者，摘要用后者，两者不可互换。

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::adapters::contracts::{AgentAdapter, SessionEditor};
use crate::adapters::shared::editing::{EditDocument, SNAPSHOT_BEFORE_EDIT};
use crate::errors::DomainError;
use crate::events::Event;
use crate::operations::plan_store::OperationPlan;
use crate::operations::types::{
    AssistantReply, EngineError, EngineResult, IndexedSession, Ports, Resolver,
};
use crate::operations::validation::validate_ops;
use crate::sessions::safety::{bounded_json, finalize_dto, python_json_len, truncate_text};

pub use crate::sessions::safety::MAX_AGENT_DTO_BYTES;

/// 预览的中间结果。
#[derive(Clone, Debug, PartialEq)]
pub struct PreviewOutcome {
    pub before: Map<String, Value>,
    pub after: Map<String, Value>,
    pub changes: Vec<Event>,
    pub revision: String,
}

/// 写入事务的产物：结果 DTO、事务里的文档、已创建的快照。
pub struct MutationOutcome {
    pub result: Map<String, Value>,
    pub document: EditDocument,
    pub snapshot: PathBuf,
}

/// `_finish_mutation` 的回调面：executor 实现它，edit 只负责在正确的时机调用。
pub trait MutationFinisher {
    #[allow(clippy::too_many_arguments)]
    fn finish(
        &self,
        tool: &str,
        editor: &dyn SessionEditor,
        result: Map<String, Value>,
        document: &EditDocument,
        snapshot: &Path,
        probe: bool,
    ) -> EngineResult<Map<String, Value>>;
}

fn is_locator_stale(error: &EngineError) -> bool {
    error.error_type() == "LocatorStaleError"
}

/// 加载文档：`load_preview` 存在就优先（codex / opencode 提供只读路径）。
fn load_document(
    editor: &dyn SessionEditor,
    reference: &str,
    prefer_preview_loader: bool,
) -> EngineResult<EditDocument> {
    if prefer_preview_loader {
        if let Some(document) = editor.load_preview(reference) {
            return Ok(document?);
        }
    }
    Ok(editor.load(reference)?)
}

/// 只读预览：load → stats → mutate → validate → stats。
pub fn preview_mutation(
    editor: &dyn SessionEditor,
    reference: &str,
    mutate: impl FnOnce(&mut EditDocument) -> EngineResult<Vec<Event>>,
    prefer_preview_loader: bool,
) -> EngineResult<PreviewOutcome> {
    let mut document = load_document(editor, reference, prefer_preview_loader)?;
    let before = editor.stats(&document)?;
    let changes = mutate(&mut document)?;
    editor.validate(&document)?;
    let after = editor.stats(&document)?;
    Ok(PreviewOutcome {
        before,
        after,
        changes,
        revision: document.revision.clone(),
    })
}

/// 写入事务；顺序与 `apply_mutation` 逐行对齐。
pub fn apply_mutation(
    editor: &dyn SessionEditor,
    reference: &str,
    mutate: impl FnOnce(&mut EditDocument) -> EngineResult<Vec<Event>>,
    expected_revision: Option<&str>,
) -> EngineResult<MutationOutcome> {
    let mut document = editor.load(reference)?;
    if let Some(expected) = expected_revision {
        if document.revision != expected {
            return Err(
                DomainError::concurrent_modification("源会话在预览后已变化，请重新预览").into(),
            );
        }
    }
    let before = editor.stats(&document)?;
    let changes = mutate(&mut document)?;
    editor.validate(&document)?;
    // 快照记下它救的是哪次编辑，还原界面才能说清「会失去什么」。
    let mut extra = Map::new();
    extra.insert("changes".into(), events_to_value(&changes));
    extra.insert("before".into(), Value::Object(before));
    extra.insert("after".into(), Value::Object(editor.stats(&document)?));
    let snapshot = editor
        .snapshot(&document, SNAPSHOT_BEFORE_EDIT, Some(&extra))?
        .ok_or_else(|| EngineError::runtime("原地编辑无法创建恢复快照，已取消写入"))?;

    match commit_and_finalize(editor, &mut document, &changes, &snapshot) {
        Ok(result) => Ok(MutationOutcome {
            result,
            document,
            snapshot,
        }),
        Err(error) => {
            // ConcurrentModificationError 不还原：源已被别人改过，旧快照会盖掉新内容。
            if !error.is_concurrent_modification() {
                editor.restore_snapshot(&snapshot, &document)?;
            }
            Err(error)
        }
    }
}

fn commit_and_finalize(
    editor: &dyn SessionEditor,
    document: &mut EditDocument,
    changes: &[Event],
    snapshot: &Path,
) -> EngineResult<Map<String, Value>> {
    let mut result = editor.commit(document)?;
    let revision = editor.saved_revision(&result, document)?;
    result.insert("ok".into(), Value::Bool(true));
    result.insert("changes".into(), events_to_value(changes));
    result.insert("revision".into(), Value::from(revision));
    result.insert(
        "snapshot".into(),
        Value::from(snapshot.to_string_lossy().into_owned()),
    );
    Ok(result)
}

fn events_to_value(changes: &[Event]) -> Value {
    Value::Array(
        changes
            .iter()
            .map(|change| serde_json::to_value(change).unwrap_or(Value::Null))
            .collect(),
    )
}

/// 原生 op 的直通预览。
pub fn preview(
    editor: &dyn SessionEditor,
    reference: &str,
    ops: &[Value],
    prefer_preview_loader: bool,
) -> EngineResult<PreviewOutcome> {
    preview_mutation(
        editor,
        reference,
        |document| Ok(editor.apply_ops(document, ops)?),
        prefer_preview_loader,
    )
}

/// 原生 op 的直通写入；先做一次 in-place 支持性门禁。
pub fn apply(
    editor: &dyn SessionEditor,
    reference: &str,
    ops: &[Value],
    expected_revision: Option<&str>,
) -> EngineResult<MutationOutcome> {
    let supported = ops.iter().all(|operation| {
        operation
            .get("op")
            .and_then(Value::as_str)
            .is_some_and(|name| editor.operations().contains(&name))
    });
    if !supported {
        let names: Vec<&str> = ops
            .iter()
            .map(|operation| operation.get("op").and_then(Value::as_str).unwrap_or("?"))
            .collect();
        return Err(DomainError::operation_unsupported(
            editor.name(),
            &names.join(","),
            Some("inplace"),
        )
        .into());
    }
    apply_mutation(
        editor,
        reference,
        |document| Ok(editor.apply_ops(document, ops)?),
        expected_revision,
    )
}

// ---------------------------------------------------------------------------
// EditOperationHandler
// ---------------------------------------------------------------------------

pub struct EditOperationHandler {
    ports: Ports,
    index: Resolver,
}

impl EditOperationHandler {
    pub fn new(ports: Ports, index: Resolver) -> Self {
        Self { ports, index }
    }

    /// 解析定位符 + in-place 支持性门禁；返回原生 op 列表。
    pub fn ensure_supported(
        &self,
        record: &IndexedSession,
        ops: &[Value],
    ) -> EngineResult<Vec<Value>> {
        let adapter = self.ports.adapter(&record.tool)?;
        let editor = adapter.require_editor()?;
        let native_ops = match self.resolve_ops(record, ops) {
            Ok(native_ops) => native_ops,
            Err(error) if is_locator_stale(&error) => {
                return Err(Self::public_locator_error(ops).into())
            }
            Err(error) => return Err(error),
        };
        Self::require_inplace_support(&adapter, editor, &native_ops)?;
        Ok(native_ops)
    }

    /// 批准后的写入。
    pub fn apply(
        &self,
        operation: &OperationPlan,
        finisher: &dyn MutationFinisher,
    ) -> EngineResult<Map<String, Value>> {
        let params = operation.input()?;
        let tool = string_param(&params, "tool")?;
        let reference = string_param(&params, "ref")?;
        let record = match self.index.resolve(&tool, &reference) {
            Ok(record) => record,
            Err(error) if error.error_type == "AgentReferenceError" => {
                return Err(DomainError::concurrent_modification(
                    "会话在操作计划生成后已变化，请重新计划",
                )
                .into())
            }
            Err(error) => return Err(error.into()),
        };
        if record.revision != operation.base_revision {
            return Err(DomainError::concurrent_modification(
                "会话在操作计划生成后已变化，请重新计划",
            )
            .into());
        }
        let adapter = self.ports.adapter(&tool)?;
        let editor = adapter.require_editor()?;
        let requested_ops = params
            .get("ops")
            .and_then(Value::as_array)
            .ok_or_else(|| EngineError::key_error("ops"))?
            .clone();
        let native_ops = self.ensure_supported(&record, &requested_ops)?;
        let probe = params
            .get("probe")
            .and_then(Value::as_bool)
            .ok_or_else(|| EngineError::key_error("probe"))?;
        let expected_revision = operation.document_revision.as_deref();

        let has_replacement = native_ops.iter().any(is_replace_reply);
        let outcome = if has_replacement {
            apply_mutation(
                editor,
                &record.canonical_ref,
                |document| Self::run_mixed_mutation(editor, document, &native_ops),
                expected_revision,
            )
        } else {
            apply(
                editor,
                &record.canonical_ref,
                &native_ops,
                expected_revision,
            )
        };
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(error) if is_locator_stale(&error) => {
                return Err(Self::public_locator_error(&requested_ops).into())
            }
            Err(error) => return Err(error),
        };
        finisher.finish(
            &tool,
            editor,
            outcome.result,
            &outcome.document,
            &outcome.snapshot,
            probe,
        )
    }

    /// `fml_` 轮次定位符与 rewrite locator 的解析。
    pub fn resolve_ops(&self, record: &IndexedSession, ops: &[Value]) -> EngineResult<Vec<Value>> {
        let mut resolved = Vec::with_capacity(ops.len());
        for operation in ops {
            let object = operation
                .as_object()
                .ok_or_else(|| EngineError::key_error("op"))?;
            let mut item = object.clone();
            let name = object
                .get("op")
                .and_then(Value::as_str)
                .ok_or_else(|| EngineError::key_error("op"))?;
            if name == "replace-assistant-reply" {
                if let Some(turn) = object.get("turn").and_then(Value::as_str) {
                    if turn.starts_with("fml_") {
                        let located = self.index.resolve_message_locator(record, turn)?;
                        item.insert("turn".into(), Value::from(located.native_locator));
                    }
                }
                resolved.push(Value::Object(item));
                continue;
            }
            if name == "rewrite" {
                let locator = object
                    .get("locator")
                    .and_then(Value::as_str)
                    .ok_or_else(|| EngineError::key_error("locator"))?;
                let message = self.index.resolve_message_locator(record, locator)?;
                if !message.editable {
                    let mut params = Map::new();
                    params.insert("field".into(), Value::from("locator"));
                    params.insert("locator".into(), Value::from(locator));
                    params.insert(
                        "hint".into(),
                        Value::from("仅使用 editable=true 的消息引用"),
                    );
                    return Err(DomainError::new(
                        "agent.request_invalid",
                        "AgentRequestError",
                        "目标消息不支持文本改写",
                        params,
                    )
                    .into());
                }
                item.insert("locator".into(), Value::from(message.native_locator));
            }
            resolved.push(Value::Object(item));
        }
        Ok(resolved)
    }

    /// 普通 op 与 replace-assistant-reply 分别过 editor 的能力声明。
    pub fn require_inplace_support(
        adapter: &AgentAdapter,
        editor: &dyn SessionEditor,
        ops: &[Value],
    ) -> EngineResult<()> {
        let ordinary: Vec<&Value> = ops.iter().filter(|item| !is_replace_reply(item)).collect();
        let replacements = ops.iter().any(is_replace_reply);
        let unsupported = ordinary.iter().any(|operation| {
            !operation
                .get("op")
                .and_then(Value::as_str)
                .is_some_and(|name| editor.operations().contains(&name))
        });
        if !ordinary.is_empty() && unsupported {
            let mut names: Vec<&str> = ordinary
                .iter()
                .filter_map(|operation| operation.get("op").and_then(Value::as_str))
                .collect();
            names.sort_unstable();
            names.dedup();
            return Err(DomainError::operation_unsupported(
                adapter.id(),
                &names.join(","),
                Some("inplace"),
            )
            .into());
        }
        if replacements && !editor.operations().contains(&"replace-assistant-reply") {
            return Err(DomainError::operation_unsupported(
                adapter.id(),
                "replace-assistant-reply",
                Some("inplace"),
            )
            .into());
        }
        Ok(())
    }

    /// 混合 mutation：逐条按 op 类型分派，change 记录按顺序累积。
    fn run_mixed_mutation(
        editor: &dyn SessionEditor,
        document: &mut EditDocument,
        ops: &[Value],
    ) -> EngineResult<Vec<Event>> {
        let mut changes = Vec::new();
        for operation in ops {
            if is_replace_reply(operation) {
                let turn = operation
                    .get("turn")
                    .cloned()
                    .ok_or_else(|| EngineError::key_error("turn"))?;
                let reply = AssistantReply::from_value(
                    operation
                        .get("reply")
                        .ok_or_else(|| EngineError::key_error("reply"))?,
                )?;
                changes.extend(editor.replace_reply(document, &turn, &reply.to_value())?);
            } else {
                changes.extend(editor.apply_ops(document, std::slice::from_ref(operation))?);
            }
        }
        Ok(changes)
    }

    /// 计划期预览；输出的是有体积上限的 Agent DTO。
    pub fn preview(&self, record: &IndexedSession, ops: &Value) -> EngineResult<Value> {
        let ops = validate_ops(ops)?;
        if python_json_len(&Value::Array(ops.clone())) > 64 * 1024 {
            return Err(DomainError::agent_request_invalid("ops 超过 64 KiB").into());
        }
        let has_replacement = ops.iter().any(is_replace_reply);
        let adapter = self.ports.adapter(&record.tool)?;
        let editor = adapter.require_editor()?;
        let native_ops = self.ensure_supported(record, &ops)?;
        let outcome = if has_replacement {
            preview_mutation(
                editor,
                &record.canonical_ref,
                |document| Self::run_mixed_mutation(editor, document, &native_ops),
                true,
            )
        } else {
            preview(editor, &record.canonical_ref, &native_ops, true)
        };
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(error) if is_locator_stale(&error) => {
                return Err(Self::public_locator_error(&ops).into())
            }
            Err(error) => return Err(error),
        };

        let mut dto = Map::new();
        dto.insert("tool".into(), Value::from(record.tool.as_str()));
        dto.insert("ref".into(), Value::from(record.opaque_ref.as_str()));
        dto.insert("mode".into(), Value::from("edit"));
        dto.insert("session_id".into(), Value::from(record.session_id()));
        dto.insert(
            "revision".into(),
            Value::from(truncate_text(&outcome.revision, 256).0),
        );
        dto.insert(
            "before".into(),
            bounded_json(&Value::Object(outcome.before), 12 * 1024),
        );
        dto.insert(
            "after".into(),
            bounded_json(&Value::Object(outcome.after), 12 * 1024),
        );
        dto.insert(
            "changes".into(),
            bounded_json(&events_to_value(&outcome.changes), 12 * 1024),
        );
        Ok(Value::Object(finalize_dto(dto)?))
    }

    /// 定位符失效时对外的公共错误：文案与 hint 逐字保留（§2.5 第 30 条）。
    pub fn public_locator_error(ops: &[Value]) -> DomainError {
        let authored = ops.iter().find(|operation| {
            operation.get("op").and_then(Value::as_str) == Some("replace-assistant-reply")
                && operation.get("turn").is_some_and(Value::is_string)
        });
        match authored {
            Some(operation) => {
                let mut params = Map::new();
                params.insert("field".into(), Value::from("turn"));
                params.insert("locator".into(), operation["turn"].clone());
                params.insert(
                    "hint".into(),
                    Value::from("重新读取会话，并原样使用 turns[].turn_locator"),
                );
                DomainError::locator_stale(Some("轮次定位信息与当前会话不匹配"), params)
            }
            None => {
                let locator = ops
                    .iter()
                    .find(|operation| {
                        operation.get("op").and_then(Value::as_str) == Some("rewrite")
                    })
                    .and_then(|operation| operation.get("locator").cloned())
                    .unwrap_or(Value::Null);
                let mut params = Map::new();
                params.insert("field".into(), Value::from("locator"));
                params.insert("locator".into(), locator);
                params.insert(
                    "hint".into(),
                    Value::from(
                        "重新调用 ferry_get_session_context，并原样使用 messages[].locator",
                    ),
                );
                DomainError::locator_stale(Some("消息定位信息与当前会话不匹配"), params)
            }
        }
    }
}

fn is_replace_reply(operation: &Value) -> bool {
    operation.get("op").and_then(Value::as_str) == Some("replace-assistant-reply")
}

fn string_param(params: &Value, key: &str) -> EngineResult<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| EngineError::key_error(key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 体积助手本体的用例在 `sessions::safety`；这里只钉住「edit 用的是**那一份**」。
    #[test]
    fn edit_dto_helpers_come_from_sessions_safety() {
        assert_eq!(
            crate::sessions::safety::python_json(&json!({"b": 1, "a": 2}), false),
            r#"{"b": 1, "a": 2}"#
        );
        let big = json!({"text": "x".repeat(70 * 1024)});
        let error = finalize_dto(big.as_object().unwrap().clone()).unwrap_err();
        assert_eq!(error.message(), "Agent DTO 超过 64 KiB");
        let bounded = bounded_json(
            &Value::Array((0..201).map(Value::from).collect()),
            32 * 1024,
        );
        assert_eq!(bounded["truncated"], json!(true));
    }

    #[test]
    fn locator_error_prefers_the_authored_turn_locator() {
        let ops = vec![
            json!({"op": "rewrite", "locator": "fml_x", "text": "t"}),
            json!({"op": "replace-assistant-reply", "turn": "fml_turn", "reply": {}}),
        ];
        let error = EditOperationHandler::public_locator_error(&ops);
        assert_eq!(error.code, "session.locator_stale");
        assert_eq!(error.message(), "轮次定位信息与当前会话不匹配");
        assert_eq!(error.params()["field"], json!("turn"));
        assert_eq!(error.params()["locator"], json!("fml_turn"));
        assert_eq!(
            error.params()["hint"],
            json!("重新读取会话，并原样使用 turns[].turn_locator")
        );

        let rewrite_only = vec![json!({"op": "rewrite", "locator": "fml_x", "text": "t"})];
        let error = EditOperationHandler::public_locator_error(&rewrite_only);
        assert_eq!(error.message(), "消息定位信息与当前会话不匹配");
        assert_eq!(error.params()["field"], json!("locator"));
        assert_eq!(error.params()["locator"], json!("fml_x"));
        assert_eq!(
            error.params()["hint"],
            json!("重新调用 ferry_get_session_context，并原样使用 messages[].locator")
        );
    }
}
