//! 统一操作计划生成。
//!
//! 三条 kind 分支共同的形状：能力门禁 → 解析索引记录 → 生成预览 →
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
    validate_edit_input, validate_metadata_input, validate_migration_input,
};

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
}

fn require_str(value: &Value, key: &str) -> EngineResult<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| EngineError::key_error(key))
}
