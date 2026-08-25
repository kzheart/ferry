//! 已审批操作的原生写入执行。
//!
//! 硬约束：
//! - 每条分支都要在写之前重新解析索引并比 `base_revision`（§2.4）；
//! - migration 的二次门禁：`rolled_back or structure.ok is not True → RuntimeError`（第 25 条）。

use serde_json::{Map, Value};

use crate::errors::DomainError;
use crate::operations::edit::EditOperationHandler;
use crate::operations::metadata;
use crate::operations::migrate::MigrationService;
use crate::operations::plan_store::OperationPlan;
use crate::operations::types::{EngineError, EngineResult, Ports, Resolver};

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
        self.edit.apply(operation)
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
            params.get("max_turn").and_then(Value::as_i64),
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
