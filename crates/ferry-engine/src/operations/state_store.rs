//! `operation_plans` / `operation_audit` 的 CAS 状态机与审计流水。
//!
//! 硬约束：
//! - 所有多语句写事务都是 `BEGIN IMMEDIATE`，状态变更与审计同一个事务（§2.3 第 17 条）；
//! - CAS 一律「条件 UPDATE + rowcount 判定」，一次性批准靠
//!   `WHERE status='planned'` 的 rowcount==1，绝不靠前置读（§2.3 第 19 条）；
//! - `finish` 失败先 rollback 再报错。

use std::sync::Arc;

use rusqlite::{params, Connection};
use serde_json::{Map, Value};

use crate::operations::plan_store::OperationPlan;
use crate::operations::types::{EngineError, EngineResult};
use crate::storage::database::StateConnector;

/// `operation_plans` 的一行。
#[derive(Clone, Debug, PartialEq)]
pub struct OperationPlanRow {
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
    pub status: String,
    pub result_json: Option<String>,
    pub error_type: Option<String>,
    pub updated_at: i64,
}

/// 一条审计记录。
#[derive(Clone, Debug, PartialEq)]
pub struct AuditEntry {
    pub event: String,
    pub at: i64,
    pub details: Value,
}

#[derive(Debug)]
pub struct OperationStore {
    connector: Arc<StateConnector>,
}

impl OperationStore {
    pub fn new(connector: Arc<StateConnector>) -> Self {
        Self { connector }
    }

    /// 崩溃恢复：`{queued, applying}` → `failed(EngineRestarted)`。
    ///
    /// 刻意保留 Python 的两个「缺陷」（方案 §5）：不写审计、不开 BEGIN IMMEDIATE。
    pub fn recover_interrupted(&self) -> EngineResult<()> {
        self.connector.with_connection(|connection| {
            connection.execute(
                "UPDATE operation_plans
                 SET status = 'failed',
                     error_type = 'EngineRestarted',
                     updated_at = CAST(strftime('%s', 'now') AS INTEGER) * 1000
                 WHERE status IN ('queued', 'applying')",
                [],
            )?;
            Ok(())
        })
    }

    fn audit_row(
        connection: &Connection,
        plan_id: &str,
        event: &str,
        at: i64,
        details: &Map<String, Value>,
    ) -> EngineResult<()> {
        let details_json =
            crate::storage::database::canonical_json(&Value::Object(details.clone()))?;
        connection.execute(
            "INSERT INTO operation_audit(plan_id, event, at, details_json)
             VALUES (?, ?, ?, ?)",
            params![plan_id, event, at, details_json],
        )?;
        Ok(())
    }

    /// 落盘一个新计划并写 `planned` 审计（同事务）。
    pub fn store_plan(&self, plan: &OperationPlan, updated_at: i64) -> EngineResult<()> {
        self.connector.with_connection(|connection| {
            connection.execute_batch("BEGIN IMMEDIATE")?;
            connection.execute(
                "INSERT INTO operation_plans(
                     plan_id, kind, input_json, preview_json,
                     input_digest, preview_digest, base_revision,
                     document_revision, created_at, expires_at,
                     status, result_json, error_type, updated_at
                 ) VALUES (
                     ?, ?, ?, ?,
                     ?, ?, ?,
                     ?, ?, ?,
                     'planned', NULL, NULL, ?
                 )",
                params![
                    plan.plan_id,
                    plan.kind,
                    plan.input_json,
                    plan.preview_json,
                    plan.input_digest,
                    plan.preview_digest,
                    plan.base_revision,
                    plan.document_revision,
                    plan.created_at,
                    plan.expires_at,
                    updated_at,
                ],
            )?;
            // 审计只存摘要与元信息，绝不落原文（测试逐字断言 secret 不出现）。
            let mut details = Map::new();
            details.insert("kind".into(), Value::from(plan.kind.as_str()));
            details.insert(
                "input_digest".into(),
                Value::from(plan.input_digest.as_str()),
            );
            details.insert(
                "preview_digest".into(),
                Value::from(plan.preview_digest.as_str()),
            );
            details.insert(
                "base_revision".into(),
                Value::from(plan.base_revision.as_str()),
            );
            details.insert("expires_at".into(), Value::from(plan.expires_at));
            Self::audit_row(connection, &plan.plan_id, "planned", updated_at, &details)?;
            connection.execute_batch("COMMIT")?;
            Ok(())
        })
    }

    pub fn get(&self, plan_id: &str) -> EngineResult<Option<OperationPlanRow>> {
        self.connector.with_connection(|connection| {
            let mut statement =
                connection.prepare("SELECT * FROM operation_plans WHERE plan_id = ?")?;
            let mut rows = statement.query(params![plan_id])?;
            let Some(row) = rows.next()? else {
                return Ok(None);
            };
            Ok(Some(OperationPlanRow {
                plan_id: row.get("plan_id")?,
                kind: row.get("kind")?,
                input_json: row.get("input_json")?,
                preview_json: row.get("preview_json")?,
                input_digest: row.get("input_digest")?,
                preview_digest: row.get("preview_digest")?,
                base_revision: row.get("base_revision")?,
                document_revision: row.get("document_revision")?,
                created_at: row.get("created_at")?,
                expires_at: row.get("expires_at")?,
                status: row.get("status")?,
                result_json: row.get("result_json")?,
                error_type: row.get("error_type")?,
                updated_at: row.get("updated_at")?,
            }))
        })
    }

    pub fn expire(&self, plan_id: &str, now: i64) -> EngineResult<bool> {
        self.transition(plan_id, "planned", "expired", now, None, "expired")
    }

    pub fn claim(&self, plan_id: &str, now: i64) -> EngineResult<bool> {
        self.transition(plan_id, "planned", "applying", now, None, "applying")
    }

    /// 一次性批准的唯一闸门：rowcount==1 才算抢到。
    pub fn enqueue(&self, plan_id: &str, now: i64) -> EngineResult<bool> {
        self.transition(plan_id, "planned", "queued", now, None, "queued")
    }

    pub fn claim_queued(&self, plan_id: &str, now: i64) -> EngineResult<bool> {
        self.transition(plan_id, "queued", "applying", now, None, "applying")
    }

    pub fn cancel(&self, plan_id: &str, expected: &str, now: i64) -> EngineResult<bool> {
        self.transition(plan_id, expected, "cancelled", now, None, "cancelled")
    }

    pub fn transition(
        &self,
        plan_id: &str,
        expected: &str,
        status: &str,
        now: i64,
        error_type: Option<&str>,
        event: &str,
    ) -> EngineResult<bool> {
        self.connector.with_connection(|connection| {
            connection.execute_batch("BEGIN IMMEDIATE")?;
            let changed = connection.execute(
                "UPDATE operation_plans
                 SET status = ?, error_type = ?, updated_at = ?
                 WHERE plan_id = ? AND status = ?",
                params![status, error_type, now, plan_id, expected],
            )?;
            if changed > 0 {
                let mut details = Map::new();
                if let Some(error_type) = error_type {
                    details.insert("error_type".into(), Value::from(error_type));
                }
                Self::audit_row(connection, plan_id, event, now, &details)?;
            }
            connection.execute_batch("COMMIT")?;
            Ok(changed == 1)
        })
    }

    /// 成功收尾：`applying → applied`，同事务写 `applied` 审计（只放摘要）。
    pub fn finish(
        &self,
        plan_id: &str,
        result_json: &str,
        result_digest: &str,
        now: i64,
    ) -> EngineResult<()> {
        self.connector.with_connection(|connection| {
            connection.execute_batch("BEGIN IMMEDIATE")?;
            let changed = connection.execute(
                "UPDATE operation_plans
                 SET status = 'applied', result_json = ?,
                     error_type = NULL, updated_at = ?
                 WHERE plan_id = ? AND status = 'applying'",
                params![result_json, now, plan_id],
            )?;
            if changed != 1 {
                // 先 rollback 再报错：否则事务悬着，后续写全部撞 SQLITE_BUSY。
                connection.execute_batch("ROLLBACK")?;
                return Err(EngineError::runtime("Operation 状态提交失败"));
            }
            let mut details = Map::new();
            details.insert("result_digest".into(), Value::from(result_digest));
            Self::audit_row(connection, plan_id, "applied", now, &details)?;
            connection.execute_batch("COMMIT")?;
            Ok(())
        })
    }

    /// 失败收尾：`error_type` 存的是 Python 异常类名（§2.4 第 22 条）。
    pub fn fail(&self, plan_id: &str, error_type: &str, now: i64) -> EngineResult<()> {
        if !self.transition(
            plan_id,
            "applying",
            "failed",
            now,
            Some(error_type),
            "failed",
        )? {
            return Err(EngineError::runtime("Operation 失败状态提交失败"));
        }
        Ok(())
    }

    pub fn audit(&self, plan_id: &str) -> EngineResult<Vec<AuditEntry>> {
        self.connector.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT event, at, details_json
                 FROM operation_audit
                 WHERE plan_id = ?
                 ORDER BY sequence",
            )?;
            let rows = statement.query_map(params![plan_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            let mut entries = Vec::new();
            for row in rows {
                let (event, at, details_json) = row?;
                let details = serde_json::from_str(&details_json)
                    .map_err(|error| EngineError::value_error(error.to_string()))?;
                entries.push(AuditEntry { event, at, details });
            }
            Ok(entries)
        })
    }
}

impl AuditEntry {
    /// 与 Python `audit()` 逐字段一致的 DTO。
    pub fn to_value(&self) -> Value {
        let mut payload = Map::new();
        payload.insert("event".into(), Value::from(self.event.as_str()));
        payload.insert("at".into(), Value::from(self.at));
        payload.insert("details".into(), self.details.clone());
        Value::Object(payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::database::StateDatabase;

    fn database() -> (tempfile::TempDir, StateDatabase) {
        let dir = tempfile::tempdir().unwrap();
        let database = StateDatabase::open(dir.path().join("ferry-state.sqlite3"), false).unwrap();
        (dir, database)
    }

    fn sample_plan(plan_id: &str) -> OperationPlan {
        OperationPlan {
            plan_id: plan_id.to_string(),
            kind: "edit".into(),
            input_json: r#"{"kind":"edit"}"#.into(),
            preview_json: "{}".into(),
            input_digest: "digest-in".into(),
            preview_digest: "digest-pre".into(),
            base_revision: "rev-1".into(),
            document_revision: Some("doc-1".into()),
            created_at: 1_000,
            expires_at: 2_000,
        }
    }

    #[test]
    fn enqueue_is_a_one_shot_cas() {
        let (_dir, database) = database();
        database
            .operations
            .store_plan(&sample_plan("op_one"), 1_000)
            .unwrap();

        assert!(database.operations.enqueue("op_one", 1_001).unwrap());
        assert!(!database.operations.enqueue("op_one", 1_002).unwrap());
        assert_eq!(
            database.operations.get("op_one").unwrap().unwrap().status,
            "queued"
        );
    }

    #[test]
    fn audit_sequence_matches_the_python_lifecycle() {
        let (_dir, database) = database();
        database
            .operations
            .store_plan(&sample_plan("op_two"), 1_000)
            .unwrap();
        assert!(database.operations.enqueue("op_two", 1_001).unwrap());
        assert!(database.operations.claim_queued("op_two", 1_002).unwrap());
        database
            .operations
            .finish("op_two", "{}", "digest", 1_003)
            .unwrap();

        let events: Vec<String> = database
            .operations
            .audit("op_two")
            .unwrap()
            .into_iter()
            .map(|entry| entry.event)
            .collect();
        assert_eq!(events, ["planned", "queued", "applying", "applied"]);
    }

    #[test]
    fn cancelled_queued_plan_records_three_events_and_never_applies() {
        let (_dir, database) = database();
        database
            .operations
            .store_plan(&sample_plan("op_three"), 1_000)
            .unwrap();
        assert!(database.operations.enqueue("op_three", 1_001).unwrap());
        assert!(database
            .operations
            .cancel("op_three", "queued", 1_002)
            .unwrap());
        // 已取消的计划不能再被 worker claim（worker 见状态非 queued 即静默退出）。
        assert!(!database.operations.claim_queued("op_three", 1_003).unwrap());

        let events: Vec<String> = database
            .operations
            .audit("op_three")
            .unwrap()
            .into_iter()
            .map(|entry| entry.event)
            .collect();
        assert_eq!(events, ["planned", "queued", "cancelled"]);
    }

    #[test]
    fn finish_rolls_back_before_reporting_failure() {
        let (_dir, database) = database();
        database
            .operations
            .store_plan(&sample_plan("op_four"), 1_000)
            .unwrap();

        let error = database
            .operations
            .finish("op_four", "{}", "digest", 1_001)
            .unwrap_err();
        assert_eq!(error.message(), "Operation 状态提交失败");
        // rollback 生效的证据：后续写事务仍能正常拿到锁。
        assert!(database.operations.enqueue("op_four", 1_002).unwrap());
    }

    #[test]
    fn recover_interrupted_only_touches_queued_and_applying() {
        let (_dir, database) = database();
        for (plan_id, terminal) in [("op_q", false), ("op_a", false), ("op_done", true)] {
            database
                .operations
                .store_plan(&sample_plan(plan_id), 1_000)
                .unwrap();
            if terminal {
                assert!(database.operations.enqueue(plan_id, 1_001).unwrap());
                assert!(database.operations.claim_queued(plan_id, 1_002).unwrap());
                database
                    .operations
                    .finish(plan_id, "{}", "digest", 1_003)
                    .unwrap();
            }
        }
        assert!(database.operations.enqueue("op_q", 1_001).unwrap());
        assert!(database.operations.claim("op_a", 1_001).unwrap());

        database.operations.recover_interrupted().unwrap();

        for plan_id in ["op_q", "op_a"] {
            let row = database.operations.get(plan_id).unwrap().unwrap();
            assert_eq!(row.status, "failed");
            assert_eq!(row.error_type.as_deref(), Some("EngineRestarted"));
        }
        assert_eq!(
            database.operations.get("op_done").unwrap().unwrap().status,
            "applied"
        );
        // 崩溃恢复刻意不写审计（方案 §5）。
        let events: Vec<String> = database
            .operations
            .audit("op_q")
            .unwrap()
            .into_iter()
            .map(|entry| entry.event)
            .collect();
        assert_eq!(events, ["planned", "queued"]);
    }

    #[test]
    fn audit_details_carry_digests_not_raw_input() {
        let (_dir, database) = database();
        let mut plan = sample_plan("op_five");
        plan.input_json = r#"{"secret":"Bearer operation-audit-secret"}"#.into();
        database.operations.store_plan(&plan, 1_000).unwrap();

        let encoded = serde_json::to_string(&Value::Array(
            database
                .operations
                .audit("op_five")
                .unwrap()
                .iter()
                .map(AuditEntry::to_value)
                .collect(),
        ))
        .unwrap();
        assert!(!encoded.contains("operation-audit-secret"), "{encoded}");
        assert!(encoded.contains("digest-in"));
    }
}
