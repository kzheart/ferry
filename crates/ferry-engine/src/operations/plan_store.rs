//! Operation plan 的模型、摘要与 SQLite 装载。
//!
//! 硬约束：
//! - frozen plan：`input` / `preview` 落盘时先 `canonical_json`，摘要是对**该串**
//!   取的 sha256（§2.4 第 16 条），批准后一律只认 plan_id；
//! - TTL 10 分钟、惰性物化：只有 `planned` 且已过期才会写 `expired`（§2.4 第 21 条）；
//! - plan_id = `op_` + `token_urlsafe(18)`（24 字符 URL-safe base64 无填充）。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use rand::TryRngCore as _;
use serde_json::{Map, Value};

use crate::contracts::operations::{OPERATION_PLAN_ID_PREFIX, OPERATION_STATUSES};
use crate::errors::DomainError;
use crate::operations::types::{EngineError, EngineResult};
use crate::storage::database::{
    canonical_json, digest_json, state_database_path, Clock, StateDatabase,
};

pub const PLAN_TTL_MS: i64 = 10 * 60 * 1000;

/// `secrets.token_urlsafe(nbytes)` 的等价物：CSPRNG + URL-safe base64 无填充。
pub fn token_urlsafe(bytes: usize) -> String {
    let mut buffer = vec![0_u8; bytes];
    rand::rngs::OsRng
        .try_fill_bytes(&mut buffer)
        .expect("系统 CSPRNG 不可用");
    URL_SAFE_NO_PAD.encode(&buffer)
}

/// 冻结的操作计划。字段顺序与 `operation_plans` 的列顺序一致。
#[derive(Clone, Debug, PartialEq)]
pub struct OperationPlan {
    pub plan_id: String,
    pub kind: String,
    pub input_json: String,
    pub preview_json: String,
    pub input_digest: String,
    pub preview_digest: String,
    pub base_revision: String,
    pub document_revision: Option<String>,
    pub created_at: i64,
    pub expires_at: i64,
}

impl OperationPlan {
    pub fn input(&self) -> EngineResult<Value> {
        serde_json::from_str(&self.input_json)
            .map_err(|error| EngineError::value_error(error.to_string()))
    }

    pub fn preview(&self) -> EngineResult<Value> {
        serde_json::from_str(&self.preview_json)
            .map_err(|error| EngineError::value_error(error.to_string()))
    }
}

/// 计划的可变状态。
#[derive(Clone, Debug, PartialEq)]
pub struct OperationState {
    pub status: String,
    pub result_json: Option<String>,
    pub error_type: Option<String>,
    pub error_message: Option<String>,
    pub updated_at: i64,
}

impl OperationState {
    /// 等价 `__post_init__`：非法 status 直接拒绝。
    pub fn new(
        status: String,
        result_json: Option<String>,
        error_type: Option<String>,
        error_message: Option<String>,
        updated_at: i64,
    ) -> EngineResult<Self> {
        if !OPERATION_STATUSES.contains(&status.as_str()) {
            let mut params = Map::new();
            params.insert("status".into(), Value::from(status.as_str()));
            return Err(DomainError::new(
                "agent.request_invalid",
                "AgentRequestError",
                "operation status 非法",
                params,
            )
            .into());
        }
        Ok(Self {
            status,
            result_json,
            error_type,
            error_message,
            updated_at,
        })
    }
}

/// 计划存储：按 state_dir 惰性打开状态库。
///
/// 注意这里用的是**会触发崩溃恢复**的 `StateDatabase::open(.., true)`——只有
/// `OperationService` 走这条路径（§2.3 第 20 条）。
pub struct OperationPlanStore {
    state_dir: PathBuf,
    clock: Arc<dyn Clock>,
    cached: Mutex<Option<(PathBuf, Arc<StateDatabase>)>>,
}

impl OperationPlanStore {
    pub fn new(state_dir: PathBuf, clock: Arc<dyn Clock>) -> Self {
        Self {
            state_dir,
            clock,
            cached: Mutex::new(None),
        }
    }

    pub fn database(&self) -> EngineResult<Arc<StateDatabase>> {
        let path = state_database_path(&self.state_dir);
        let mut cached = self
            .cached
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some((cached_path, database)) = cached.as_ref() {
            if cached_path == &path {
                return Ok(Arc::clone(database));
            }
        }
        let database = Arc::new(StateDatabase::open(path.clone(), true)?);
        *cached = Some((path, Arc::clone(&database)));
        Ok(database)
    }

    pub fn now_ms(&self) -> i64 {
        self.clock.now_ms()
    }

    /// 冻结并落盘一个计划，返回对外的 public plan DTO。
    pub fn create(
        &self,
        operation_input: &Value,
        preview: &Value,
        base_revision: &str,
        document_revision: Option<&str>,
    ) -> EngineResult<Value> {
        let input_json = canonical_json(operation_input)?;
        let preview_json = canonical_json(preview)?;
        let created_at = self.now_ms();
        let kind = operation_input
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::key_error("kind"))?
            .to_string();
        let plan = OperationPlan {
            plan_id: format!("{OPERATION_PLAN_ID_PREFIX}{}", token_urlsafe(18)),
            kind,
            input_digest: digest_json(&input_json),
            preview_digest: digest_json(&preview_json),
            input_json,
            preview_json,
            base_revision: base_revision.to_string(),
            document_revision: document_revision.map(str::to_string),
            created_at,
            expires_at: created_at + PLAN_TTL_MS,
        };
        self.database()?.operations.store_plan(&plan, created_at)?;
        public_plan(&plan)
    }

    pub fn get(&self, plan_id: &Value) -> EngineResult<(OperationPlan, OperationState)> {
        let plan_id = plan_id
            .as_str()
            .filter(|value| value.starts_with(OPERATION_PLAN_ID_PREFIX))
            .ok_or_else(|| DomainError::agent_request_invalid("plan_id 非法"))?;
        let row = self.database()?.operations.get(plan_id)?.ok_or_else(|| {
            DomainError::agent_request_invalid("operation plan 不存在或已因重启失效")
        })?;
        Ok((
            OperationPlan {
                plan_id: row.plan_id,
                kind: row.kind,
                input_json: row.input_json,
                preview_json: row.preview_json,
                input_digest: row.input_digest,
                preview_digest: row.preview_digest,
                base_revision: row.base_revision,
                document_revision: row.document_revision,
                created_at: row.created_at,
                expires_at: row.expires_at,
            },
            OperationState::new(
                row.status,
                row.result_json,
                row.error_type,
                row.error_message,
                row.updated_at,
            )?,
        ))
    }

    /// 惰性过期物化：只有 `planned` 且已越过 `expires_at` 才写 `expired`。
    pub fn expire(
        &self,
        operation: &OperationPlan,
        state: &mut OperationState,
    ) -> EngineResult<()> {
        if state.status == "planned" && operation.expires_at < self.now_ms() {
            let updated_at = self.now_ms();
            self.database()?
                .operations
                .expire(&operation.plan_id, updated_at)?;
            state.status = "expired".into();
            state.updated_at = updated_at;
        }
        Ok(())
    }
}

impl std::fmt::Debug for OperationPlanStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OperationPlanStore")
            .field("state_dir", &self.state_dir)
            .finish_non_exhaustive()
    }
}

/// 对外的计划 DTO；字段与文案逐字对齐 Python `public_plan`。
pub fn public_plan(operation: &OperationPlan) -> EngineResult<Value> {
    let params = operation.input()?;
    let summary = match operation.kind.as_str() {
        "migration" => {
            let source = params
                .get("source_tool")
                .and_then(Value::as_str)
                .ok_or_else(|| EngineError::key_error("source_tool"))?;
            let target = params
                .get("target_tool")
                .and_then(Value::as_str)
                .ok_or_else(|| EngineError::key_error("target_tool"))?;
            format!("将 {source} 会话迁移到 {target}")
        }
        "metadata" => "修改会话元数据".to_string(),
        "delete" => "永久删除原始会话（不可恢复）".to_string(),
        _ => "修改原始会话（执行前自动创建可恢复快照）".to_string(),
    };
    let affected_refs = if let Some(reference) = params.get("ref") {
        vec![reference.clone()]
    } else if operation.kind == "delete" {
        params
            .get("targets")
            .and_then(Value::as_array)
            .ok_or_else(|| EngineError::key_error("targets"))?
            .iter()
            .map(|target| {
                target
                    .get("ref")
                    .cloned()
                    .ok_or_else(|| EngineError::key_error("ref"))
            })
            .collect::<EngineResult<Vec<Value>>>()?
    } else {
        Vec::new()
    };

    let mut payload = Map::new();
    payload.insert("plan_id".into(), Value::from(operation.plan_id.as_str()));
    payload.insert("kind".into(), Value::from(operation.kind.as_str()));
    payload.insert("status".into(), Value::from("planned"));
    payload.insert("preview".into(), operation.preview()?);
    payload.insert("summary".into(), Value::from(summary));
    payload.insert(
        "risk".into(),
        Value::from(if operation.kind == "metadata" {
            "low"
        } else {
            "high"
        }),
    );
    payload.insert("affected_refs".into(), Value::Array(affected_refs));
    payload.insert(
        "base_revision".into(),
        Value::from(operation.base_revision.as_str()),
    );
    payload.insert(
        "document_revision".into(),
        match &operation.document_revision {
            Some(revision) => Value::from(revision.as_str()),
            None => Value::Null,
        },
    );
    payload.insert(
        "input_digest".into(),
        Value::from(operation.input_digest.as_str()),
    );
    payload.insert(
        "preview_digest".into(),
        Value::from(operation.preview_digest.as_str()),
    );
    payload.insert("created_at".into(), Value::from(operation.created_at));
    payload.insert("expires_at".into(), Value::from(operation.expires_at));
    Ok(Value::Object(payload))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plan_ids_are_twenty_four_char_url_safe_tokens() {
        let token = token_urlsafe(18);
        assert_eq!(token.len(), 24);
        assert!(token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')));
        assert_ne!(token, token_urlsafe(18));
    }

    #[test]
    fn public_plan_matches_the_python_summary_table() {
        let base = OperationPlan {
            plan_id: "op_x".into(),
            kind: "edit".into(),
            input_json: r#"{"kind":"edit","ref":"fsr_a"}"#.into(),
            preview_json: r#"{"mode":"edit"}"#.into(),
            input_digest: "d1".into(),
            preview_digest: "d2".into(),
            base_revision: "rev".into(),
            document_revision: Some("doc".into()),
            created_at: 1,
            expires_at: 2,
        };
        let edit = public_plan(&base).unwrap();
        assert_eq!(
            edit["summary"],
            json!("修改原始会话（执行前自动创建可恢复快照）")
        );
        assert_eq!(edit["risk"], json!("high"));
        assert_eq!(edit["affected_refs"], json!(["fsr_a"]));
        assert_eq!(edit["status"], json!("planned"));

        let metadata = public_plan(&OperationPlan {
            kind: "metadata".into(),
            ..base.clone()
        })
        .unwrap();
        assert_eq!(metadata["summary"], json!("修改会话元数据"));
        assert_eq!(metadata["risk"], json!("low"));

        let migration = public_plan(&OperationPlan {
            kind: "migration".into(),
            input_json: r#"{"source_tool":"claude","target_tool":"opencode"}"#.into(),
            document_revision: None,
            ..base.clone()
        })
        .unwrap();
        assert_eq!(migration["summary"], json!("将 claude 会话迁移到 opencode"));
        assert_eq!(migration["affected_refs"], json!([]));
        assert_eq!(migration["document_revision"], json!(null));

        let delete = public_plan(&OperationPlan {
            kind: "delete".into(),
            input_json: r#"{"targets":[{"ref":"fsr_a"},{"ref":"fsr_b"}]}"#.into(),
            ..base
        })
        .unwrap();
        assert_eq!(delete["summary"], json!("永久删除原始会话（不可恢复）"));
        assert_eq!(delete["affected_refs"], json!(["fsr_a", "fsr_b"]));
    }

    #[test]
    fn state_rejects_unknown_status() {
        let error = OperationState::new("nope".into(), None, None, None, 0).unwrap_err();
        assert_eq!(error.error_type(), "AgentRequestError");
        assert_eq!(error.message(), "operation status 非法");
    }
}
