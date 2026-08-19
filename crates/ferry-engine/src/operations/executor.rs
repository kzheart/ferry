//! 已审批操作的原生写入执行。
//!
//! 语义事实源：`engine/operations/executor.py`。
//!
//! 硬约束：
//! - 每条分支都要在写之前重新解析索引并比 `base_revision`（§2.4）；
//! - 探针失败还原快照、`result.ok=false`，但 operation 仍算 `applied`（§2.4 第 24 条）；
//! - migration 的二次门禁：`rolled_back or structure.ok is not True → RuntimeError`（第 25 条）；
//! - delete 三重门禁 + 单条失败不中断 + 成功后立刻 `index.evict`（第 26 条）。

use std::path::Path;

use serde_json::{Map, Value};

use crate::adapters::contracts::SessionEditor;
use crate::adapters::shared::editing::EditDocument;
use crate::errors::DomainError;
use crate::operations::delete::SessionDeletionService;
use crate::operations::edit::{EditOperationHandler, MutationFinisher};
use crate::operations::metadata;
use crate::operations::metadata_store::metadata_key;
use crate::operations::migrate::MigrationService;
use crate::operations::plan_store::OperationPlan;
use crate::operations::planner::protected_cause;
use crate::operations::types::{EngineError, EngineResult, Ports, Resolver};
use crate::operations::verification;

pub struct OperationExecutor {
    ports: Ports,
    index: Resolver,
    migration: MigrationService,
    edit: EditOperationHandler,
}

impl OperationExecutor {
    pub fn new(ports: Ports, index: Resolver) -> Self {
        Self {
            migration: MigrationService::new(Ports::clone(&ports)),
            edit: EditOperationHandler::new(Ports::clone(&ports), Resolver::clone(&index)),
            ports,
            index,
        }
    }

    pub fn execute(&self, operation: &OperationPlan) -> EngineResult<Value> {
        match operation.kind.as_str() {
            "edit" => self.apply_edit(operation).map(Value::Object),
            "migration" => self.apply_migration(operation).map(Value::Object),
            "metadata" => self.apply_metadata(operation).map(Value::Object),
            "delete" => self.apply_delete(operation).map(Value::Object),
            other => {
                let mut params = Map::new();
                params.insert("kind".into(), Value::from(other));
                Err(DomainError::new(
                    "agent.request_invalid",
                    "AgentRequestError",
                    "operation kind 非法",
                    params,
                )
                .into())
            }
        }
    }

    fn apply_edit(&self, operation: &OperationPlan) -> EngineResult<Map<String, Value>> {
        let params = operation.input()?;
        let probe = params
            .get("probe")
            .and_then(Value::as_bool)
            .ok_or_else(|| EngineError::key_error("probe"))?;
        if probe {
            let tool = params
                .get("tool")
                .and_then(Value::as_str)
                .ok_or_else(|| EngineError::key_error("tool"))?;
            self.ports.adapter(tool)?.require_verifier("probe")?;
        }
        self.edit.apply(operation, self)
    }

    fn apply_migration(&self, operation: &OperationPlan) -> EngineResult<Map<String, Value>> {
        let params = operation.input()?;
        let source_tool = str_param(&params, "source_tool")?;
        let target_tool = str_param(&params, "target_tool")?;
        let reference = str_param(&params, "ref")?;
        let record = match self.index.resolve(&source_tool, &reference) {
            Ok(record) => record,
            Err(error) if error.error_type == "AgentReferenceError" => {
                return Err(DomainError::concurrent_modification(
                    "会话在迁移计划生成后已变化，请重新计划",
                )
                .into())
            }
            Err(error) => return Err(error.into()),
        };
        if record.revision != operation.base_revision {
            return Err(DomainError::concurrent_modification(
                "会话在迁移计划生成后已变化，请重新计划",
            )
            .into());
        }
        let session = self.index.read_indexed_session(&record)?;
        let result = self.migration.apply(
            &source_tool,
            &target_tool,
            session,
            None,
            params
                .get("probe")
                .and_then(Value::as_bool)
                .ok_or_else(|| EngineError::key_error("probe"))?,
            params.get("max_turn").and_then(Value::as_i64),
            params.get("probe_model").and_then(Value::as_str),
            None,
        )?;
        // 二次门禁：回滚过、或结构验收不是恒等 true，都不许当成功收尾。
        let structure_ok = result
            .get("validation")
            .and_then(|validation| validation.get("structure"))
            .and_then(|structure| structure.get("ok"))
            == Some(&Value::Bool(true));
        if result.get("rolled_back").is_some_and(is_truthy) || !structure_ok {
            return Err(EngineError::runtime("迁移写入后的结构校验失败，产物已回滚"));
        }
        Ok(result)
    }

    fn apply_metadata(&self, operation: &OperationPlan) -> EngineResult<Map<String, Value>> {
        let params = operation.input()?;
        let tool = str_param(&params, "tool")?;
        let reference = str_param(&params, "ref")?;
        let session_id = str_param(&params, "session_id")?;
        let record = match self.index.resolve(&tool, &reference) {
            Ok(record) => record,
            Err(error) if error.error_type == "AgentReferenceError" => {
                return Err(DomainError::concurrent_modification(
                    "会话在元数据计划生成后已变化，请重新计划",
                )
                .into())
            }
            Err(error) => return Err(error.into()),
        };
        if record.revision != operation.base_revision {
            return Err(DomainError::concurrent_modification(
                "会话在元数据计划生成后已变化，请重新计划",
            )
            .into());
        }
        if record.row.get("id").and_then(Value::as_str) != Some(session_id.as_str()) {
            return Err(DomainError::concurrent_modification(
                "会话标识在元数据计划生成后已变化，请重新计划",
            )
            .into());
        }
        let expected = params
            .get("metadata_before")
            .and_then(Value::as_object)
            .cloned()
            .ok_or_else(|| EngineError::key_error("metadata_before"))?;
        let patch = params
            .get("patch")
            .and_then(Value::as_object)
            .cloned()
            .ok_or_else(|| EngineError::key_error("patch"))?;
        let applied =
            metadata::compare_and_set_entry(&tool, &session_id, &expected, &patch, &self.ports)?;
        let mut result = Map::new();
        result.insert("metadata".into(), Value::Object(applied));
        Ok(result)
    }

    fn apply_delete(&self, operation: &OperationPlan) -> EngineResult<Map<String, Value>> {
        let params = operation.input()?;
        let tool = str_param(&params, "tool")?;
        // 计划期的保护审查挡不住批准前才被 pin/archive/打标签的会话：元数据与
        // 会话内容 revision 无关，逐条 revision 比对不可能发现这种变化。
        let metadata_rows = metadata::list_all(&self.ports)?;
        let deletion = SessionDeletionService::new(Ports::clone(&self.ports));
        let mut succeeded = Vec::new();
        let mut skipped = Vec::new();
        let mut failed = Vec::new();
        let targets = params
            .get("targets")
            .and_then(Value::as_array)
            .ok_or_else(|| EngineError::key_error("targets"))?
            .clone();
        for target in &targets {
            let reference = target
                .get("ref")
                .and_then(Value::as_str)
                .ok_or_else(|| EngineError::key_error("ref"))?;
            match self.delete_one(&tool, target, reference, &metadata_rows, &deletion) {
                Ok(Some(skip)) => skipped.push(skip),
                Ok(None) => {
                    let mut entry = Map::new();
                    entry.insert("tool".into(), Value::from(tool.as_str()));
                    entry.insert("ref".into(), Value::from(reference));
                    succeeded.push(Value::Object(entry));
                }
                // 单条失败不中断批处理。
                Err(error) => {
                    let mut entry = Map::new();
                    entry.insert("tool".into(), Value::from(tool.as_str()));
                    entry.insert("ref".into(), Value::from(reference));
                    entry.insert(
                        "error".into(),
                        Value::from(error.message().chars().take(500).collect::<String>()),
                    );
                    failed.push(Value::Object(entry));
                }
            }
        }
        let mut result = Map::new();
        result.insert("succeeded".into(), Value::Array(succeeded));
        result.insert("skipped".into(), Value::Array(skipped));
        result.insert("failed".into(), Value::Array(failed));
        Ok(result)
    }

    /// 三重门禁：保护规则复查 → ref 可解析 → session_id + revision 双比对。
    ///
    /// `Ok(Some(skip))` = 被跳过；`Ok(None)` = 已删除。
    fn delete_one(
        &self,
        tool: &str,
        target: &Value,
        reference: &str,
        metadata_rows: &Map<String, Value>,
        deletion: &SessionDeletionService,
    ) -> EngineResult<Option<Value>> {
        let session_id = target
            .get("session_id")
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::key_error("session_id"))?;
        let empty = Value::Object(Map::new());
        let metadata_row = metadata_rows
            .get(&metadata_key(tool, session_id))
            .unwrap_or(&empty);
        if let Some(protection) = protected_cause(metadata_row) {
            let mut skip = Map::new();
            skip.insert("tool".into(), Value::from(tool));
            skip.insert("ref".into(), Value::from(reference));
            skip.insert("cause".into(), Value::from("protected"));
            skip.insert("protection".into(), Value::from(protection));
            return Ok(Some(Value::Object(skip)));
        }
        let record = match self.index.resolve(tool, reference) {
            Ok(record) => record,
            Err(error) if error.error_type == "AgentReferenceError" => {
                let changed =
                    error.params().get("reason").and_then(Value::as_str) == Some("session_changed");
                let mut skip = Map::new();
                skip.insert("tool".into(), Value::from(tool));
                skip.insert("ref".into(), Value::from(reference));
                skip.insert(
                    "cause".into(),
                    Value::from(if changed { "changed" } else { "not_found" }),
                );
                return Ok(Some(Value::Object(skip)));
            }
            Err(error) => return Err(error.into()),
        };
        let revision = target
            .get("revision")
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::key_error("revision"))?;
        if record.row.get("id").and_then(Value::as_str) != Some(session_id)
            || record.revision != revision
        {
            let mut skip = Map::new();
            skip.insert("tool".into(), Value::from(tool));
            skip.insert("ref".into(), Value::from(reference));
            skip.insert("cause".into(), Value::from("changed"));
            return Ok(Some(Value::Object(skip)));
        }
        deletion.delete(tool, &record.canonical_ref)?;
        // 删除后索引定点摘除并推 removal delta，不必等下一轮重扫。
        self.index.evict(tool, &record.canonical_ref)?;
        Ok(None)
    }
}

impl MutationFinisher for OperationExecutor {
    fn finish(
        &self,
        tool: &str,
        editor: &dyn SessionEditor,
        mut result: Map<String, Value>,
        document: &EditDocument,
        snapshot: &Path,
        probe: bool,
    ) -> EngineResult<Map<String, Value>> {
        if !probe {
            return Ok(result);
        }
        let report = match self.probe_edited(tool, editor, document, &result) {
            Ok(report) => report,
            Err(error) if verification::is_probe_timeout(&error) => {
                verification::timeout_report(tool, error.message())
            }
            Err(error) => return Err(error),
        };
        let passed = report.get("status") == Some(&Value::from("passed"));
        result.insert("probe".into(), report);
        if passed {
            return Ok(result);
        }
        // 探针失败：还原快照、标记失败，但 operation 仍然是 applied。
        editor.restore_snapshot(snapshot, document)?;
        result.insert("ok".into(), Value::Bool(false));
        result.insert("error".into(), Value::from("隔离探针未通过,已自动还原快照"));
        Ok(result)
    }
}

impl OperationExecutor {
    fn probe_edited(
        &self,
        tool: &str,
        editor: &dyn SessionEditor,
        document: &EditDocument,
        result: &Map<String, Value>,
    ) -> EngineResult<Value> {
        let adapter = self.ports.adapter(tool)?;
        let verifier = adapter.require_verifier("probe")?;
        let report = verifier.probe_edited(editor, document, result, None)?;
        Ok(verification::report_to_value(&report))
    }
}

fn str_param(params: &Value, key: &str) -> EngineResult<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| EngineError::key_error(key))
}

/// Python 的真值判定：`result.get("rolled_back")` 只要是真值就算回滚过。
fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64() != Some(0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(entries) => !entries.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn truthiness_matches_python() {
        assert!(!is_truthy(&json!(null)));
        assert!(!is_truthy(&json!(false)));
        assert!(!is_truthy(&json!(0)));
        assert!(!is_truthy(&json!("")));
        assert!(!is_truthy(&json!([])));
        assert!(!is_truthy(&json!({})));
        assert!(is_truthy(&json!(true)));
        assert!(is_truthy(&json!("x")));
    }
}
