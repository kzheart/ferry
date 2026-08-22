//! 统一操作计划生成。
//!
//! 四条 kind 分支共同的形状：能力门禁 → 解析索引记录 → 生成预览 →
//! **再解析一次并比 revision**（预览期间源变了就拒绝）→ 交给 service 冻结落盘。
//!
//! 与 Python 的一处结构差异：Python 的 `_plan_*` 末尾直接调用注入的
//! `store_plan` 回调；Rust 侧改成返回 [`PreparedPlan`]，由
//! `OperationService` 在锁内落盘——语义等价（store_plan 本来就是最后一句），
//! 但省掉了 service↔planner 的循环引用。

use serde_json::{Map, Value};

use crate::contracts::operations::OPERATION_KINDS;
use crate::errors::DomainError;
use crate::operations::edit::EditOperationHandler;
use crate::operations::metadata;
use crate::operations::metadata_store::metadata_key;
use crate::operations::migrate::MigrationService;
use crate::operations::types::{EngineError, EngineResult, Ports, Resolver};
use crate::operations::validation::{
    validate_delete_input, validate_edit_input, validate_metadata_input, validate_migration_input,
};
use crate::sessions::safety::truncate_text;

/// 冻结前的计划素材。
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedPlan {
    pub input: Value,
    pub preview: Value,
    pub base_revision: String,
    pub document_revision: Option<String>,
}

const PLAN_RACE_MESSAGE: &str = "会话在生成操作计划时已变化，请重新计划";

pub struct OperationPlanner {
    ports: Ports,
    index: Resolver,
    migration: MigrationService,
    edit: EditOperationHandler,
}

impl OperationPlanner {
    pub fn new(ports: Ports, index: Resolver) -> Self {
        Self {
            migration: MigrationService::new(Ports::clone(&ports)),
            edit: EditOperationHandler::new(Ports::clone(&ports), Resolver::clone(&index)),
            ports,
            index,
        }
    }

    pub fn plan(&self, value: &Value) -> EngineResult<PreparedPlan> {
        if !value.is_object() {
            return Err(DomainError::agent_request_invalid("operation input 必须是 object").into());
        }
        let kind = value.get("kind").cloned().unwrap_or(Value::Null);
        let kind_name = kind.as_str().unwrap_or_default();
        if !OPERATION_KINDS.contains(&kind_name) {
            let mut params = Map::new();
            params.insert("kind".into(), kind);
            return Err(DomainError::new(
                "agent.request_invalid",
                "AgentRequestError",
                "operation kind 非法",
                params,
            )
            .into());
        }
        match kind_name {
            "edit" => self.plan_edit(value),
            "migration" => self.plan_migration(value),
            "metadata" => self.plan_metadata(value),
            "delete" => self.plan_delete(value),
            // OPERATION_KINDS 已经过滤过，走到这里说明契约与分支表脱节。
            _ => Err(EngineError::Internal {
                error_type: "AssertionError",
                message: "Operation contract kind 未绑定处理器".into(),
            }),
        }
    }

    fn plan_edit(&self, value: &Value) -> EngineResult<PreparedPlan> {
        let operation_input = validate_edit_input(value)?;
        let tool = require_str(&operation_input, "tool")?;
        let reference = require_str(&operation_input, "ref")?;
        let adapter = self.ports.adapter(&tool)?;
        adapter.require_editor()?;
        if operation_input
            .get("probe")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            adapter.require_verifier("probe")?;
        }
        let before = self.index.resolve(&tool, &reference)?;
        let ops = operation_input
            .get("ops")
            .ok_or_else(|| EngineError::key_error("ops"))?;
        let preview = self.edit.preview(&before, ops)?;
        let after = self.index.resolve(&tool, &reference)?;
        if before.revision != after.revision {
            return Err(DomainError::concurrent_modification(PLAN_RACE_MESSAGE).into());
        }
        let ops_array = ops
            .as_array()
            .ok_or_else(|| EngineError::key_error("ops"))?
            .clone();
        self.edit.ensure_supported(&after, &ops_array)?;
        let document_revision = preview
            .get("revision")
            .map(|revision| match revision {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            })
            .ok_or_else(|| EngineError::key_error("revision"))?;
        Ok(PreparedPlan {
            input: operation_input,
            preview,
            base_revision: after.revision,
            document_revision: Some(document_revision),
        })
    }

    fn plan_migration(&self, value: &Value) -> EngineResult<PreparedPlan> {
        let operation_input = validate_migration_input(value, &self.ports.adapters())?;
        let source_tool = require_str(&operation_input, "source_tool")?;
        let target_tool = require_str(&operation_input, "target_tool")?;
        self.ports
            .adapter(&source_tool)?
            .require_migration_source()?;
        let target_adapter = self.ports.adapter(&target_tool)?;
        let target = target_adapter.require_migration_target()?;
        target_adapter.require_browser()?;
        target_adapter.require_lifecycle("resume")?;
        if operation_input
            .get("probe")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            target_adapter.require_verifier("probe")?;
        }
        // 目标端写入门禁前置：宁可在 plan 阶段多花一次进程探测，也不要让用户走完
        // 四步、点了确认才看到「目标 App 正在运行」。
        target.preflight()?;
        let reference = require_str(&operation_input, "ref")?;
        let before = self.index.resolve(&source_tool, &reference)?;
        let session = self.index.read_indexed_session(&before)?;
        let preview = self.migration.preview(
            &source_tool,
            &target_tool,
            session,
            None,
            operation_input.get("max_turn").and_then(Value::as_i64),
            operation_input.get("probe_model").and_then(Value::as_str),
            None,
        )?;
        let after = match self.index.resolve(&source_tool, &reference) {
            Ok(after) => after,
            Err(error) if error.error_type == "AgentReferenceError" => {
                return Err(DomainError::concurrent_modification(PLAN_RACE_MESSAGE).into())
            }
            Err(error) => return Err(error.into()),
        };
        if before.revision != after.revision {
            return Err(DomainError::concurrent_modification(PLAN_RACE_MESSAGE).into());
        }
        Ok(PreparedPlan {
            input: operation_input,
            preview: Value::Object(preview),
            base_revision: after.revision,
            document_revision: None,
        })
    }

    fn plan_metadata(&self, value: &Value) -> EngineResult<PreparedPlan> {
        let mut operation_input = validate_metadata_input(value)?;
        let tool = require_str(&operation_input, "tool")?;
        let reference = require_str(&operation_input, "ref")?;
        let before = self.index.resolve(&tool, &reference)?;
        let session_id = before
            .row
            .get("id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| DomainError::agent_request_invalid("会话缺少可用的 metadata id"))?
            .to_string();
        let metadata_before = metadata::list_all(&self.ports)?
            .get(&metadata_key(&tool, &session_id))
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));
        let patch = operation_input
            .get("patch")
            .cloned()
            .ok_or_else(|| EngineError::key_error("patch"))?;
        {
            let object = operation_input
                .as_object_mut()
                .ok_or_else(|| EngineError::key_error("input"))?;
            object.insert("session_id".into(), Value::from(session_id.as_str()));
            object.insert("metadata_before".into(), metadata_before.clone());
        }
        let mut preview = Map::new();
        preview.insert("tool".into(), Value::from(tool.as_str()));
        preview.insert("ref".into(), Value::from(reference.as_str()));
        preview.insert("before".into(), metadata_before);
        preview.insert("after_patch".into(), patch);

        let after = self.index.resolve(&tool, &reference)?;
        if before.revision != after.revision {
            return Err(DomainError::concurrent_modification(PLAN_RACE_MESSAGE).into());
        }
        Ok(PreparedPlan {
            input: operation_input,
            preview: Value::Object(preview),
            base_revision: after.revision,
            document_revision: None,
        })
    }

    fn plan_delete(&self, value: &Value) -> EngineResult<PreparedPlan> {
        let mut operation_input = validate_delete_input(value, &self.ports.adapters())?;
        let tool = require_str(&operation_input, "tool")?;
        self.ports.adapter(&tool)?.require_lifecycle("delete")?;
        let metadata_rows = metadata::list_all(&self.ports)?;
        let mut sessions = Vec::new();
        let mut excluded = Vec::new();
        let mut targets = Vec::new();
        let mut total_size = 0_i64;
        let refs = operation_input
            .get("refs")
            .and_then(Value::as_array)
            .ok_or_else(|| EngineError::key_error("refs"))?
            .clone();
        for reference in &refs {
            let reference = reference
                .as_str()
                .ok_or_else(|| EngineError::key_error("refs"))?;
            let record = self.index.resolve(&tool, reference)?;
            let session_id = record
                .row
                .get("id")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| DomainError::agent_request_invalid("会话缺少可用的原生 ID"))?
                .to_string();
            let title = truncate_text(
                record
                    .row
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                512,
            )
            .0;
            let empty = Value::Object(Map::new());
            let metadata_row = metadata_rows
                .get(&metadata_key(&tool, &session_id))
                .unwrap_or(&empty);
            if let Some(cause) = protected_cause(metadata_row) {
                let mut entry = Map::new();
                entry.insert("tool".into(), Value::from(tool.as_str()));
                entry.insert("ref".into(), Value::from(record.opaque_ref.as_str()));
                entry.insert("title".into(), Value::from(title));
                entry.insert("cause".into(), Value::from(cause));
                excluded.push(Value::Object(entry));
                continue;
            }
            // targets 只收未被排除的会话：它就是 apply 的删除名单，被保护规则
            // 剔除的会话一旦混进来，预览说「已保护」而执行照删。
            let mut target = Map::new();
            target.insert("ref".into(), Value::from(record.opaque_ref.as_str()));
            target.insert("session_id".into(), Value::from(session_id.as_str()));
            target.insert("revision".into(), Value::from(record.revision.as_str()));
            targets.push(Value::Object(target));

            let size = record.row.get("size").and_then(Value::as_i64).unwrap_or(0);
            total_size += size;
            let mut entry = Map::new();
            entry.insert("tool".into(), Value::from(tool.as_str()));
            entry.insert("ref".into(), Value::from(record.opaque_ref.as_str()));
            entry.insert("session_id".into(), Value::from(record.session_id()));
            entry.insert("title".into(), Value::from(title));
            entry.insert(
                "project".into(),
                record.row.get("dir").cloned().unwrap_or(Value::Null),
            );
            entry.insert("size".into(), Value::from(size));
            entry.insert(
                "updated".into(),
                record.row.get("updated").cloned().unwrap_or(Value::Null),
            );
            sessions.push(Value::Object(entry));
        }

        {
            let object = operation_input
                .as_object_mut()
                .ok_or_else(|| EngineError::key_error("input"))?;
            object.insert("targets".into(), Value::Array(targets));
        }
        let mut totals = Map::new();
        totals.insert("count".into(), Value::from(sessions.len() as i64));
        totals.insert("size_bytes".into(), Value::from(total_size));
        let mut preview = Map::new();
        preview.insert("tool".into(), Value::from(tool.as_str()));
        preview.insert("sessions".into(), Value::Array(sessions));
        preview.insert("excluded".into(), Value::Array(excluded));
        preview.insert("totals".into(), Value::Object(totals));
        // 删除不可恢复：预览必须明说，审批卡靠它渲染永久删除警示。
        preview.insert("permanent".into(), Value::Bool(true));

        Ok(PreparedPlan {
            input: operation_input,
            preview: Value::Object(preview),
            base_revision: "batch".into(),
            document_revision: None,
        })
    }
}

/// 保护规则：pinned > archived > tagged。
pub fn protected_cause(metadata_row: &Value) -> Option<&'static str> {
    if metadata_row.get("pinned") == Some(&Value::Bool(true)) {
        return Some("pinned");
    }
    if metadata_row.get("archived") == Some(&Value::Bool(true)) {
        return Some("archived");
    }
    match metadata_row.get("tags") {
        Some(Value::Array(tags)) if !tags.is_empty() => Some("tagged"),
        _ => None,
    }
}

fn require_str(value: &Value, key: &str) -> EngineResult<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| EngineError::key_error(key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn protection_precedence_matches_python() {
        assert_eq!(
            protected_cause(&json!({"pinned": true, "archived": true})),
            Some("pinned")
        );
        assert_eq!(
            protected_cause(&json!({"archived": true})),
            Some("archived")
        );
        assert_eq!(protected_cause(&json!({"tags": ["a"]})), Some("tagged"));
        assert_eq!(protected_cause(&json!({"tags": []})), None);
        // 只有恒等 True 才算保护：字符串 "yes" 不是。
        assert_eq!(protected_cause(&json!({"pinned": "yes"})), None);
        assert_eq!(protected_cause(&json!({})), None);
    }
}
