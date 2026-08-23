//! Operation 生命周期的端到端复刻（对照 `tests/test_operations.py` 的关键场景）。
//!
//! 覆盖：冻结计划 + 一次性批准、cancel 零写入、TTL 惰性过期、崩溃恢复、
//! 审计序列、metadata 独立 CAS、edit 事务的两道 revision 门禁与快照还原、
//! delete 三重门禁 + evict 回调。
//!
//! 这里用假 adapter / 假索引把 operations 单独拎出来跑，不依赖 WP-C/WP-D。

use std::any::Any;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Map, Value};

use ferry_engine::adapters::contracts::{
    AgentAdapter, AgentManifest, Fingerprint, ModelCatalog, ModelDiscovery, NativeSessionReference,
    ScanCache, ScanRow, SessionBrowser, SessionEditor, SessionLifecycle, SessionVerifier,
};
use ferry_engine::adapters::shared::editing::EditDocument;
use ferry_engine::errors::{DomainError, DomainResult};
use ferry_engine::events::Event;
use ferry_engine::model::Session;
use ferry_engine::operations::metadata;
use ferry_engine::operations::service::OperationService;
use ferry_engine::operations::types::{
    EngineError, EngineResult, IndexedSession, OperationPorts, Ports, ResolvedMessageLocator,
    Resolver, SessionResolver,
};
use ferry_engine::storage::database::{Clock, StateDatabase};

// ---------------------------------------------------------------------------
// 假件
// ---------------------------------------------------------------------------

#[derive(Default)]
struct TestClock {
    now: Mutex<i64>,
}

impl TestClock {
    fn advance(&self, millis: i64) {
        *self.now.lock().unwrap() += millis;
    }
}

impl Clock for TestClock {
    fn now_ms(&self) -> i64 {
        *self.now.lock().unwrap()
    }
}

#[derive(Default)]
struct EditorState {
    commits: AtomicUsize,
    load_calls: AtomicUsize,
    revision: Mutex<String>,
    fail_commit: AtomicBool,
    last_ops: Mutex<Vec<Value>>,
    replies: Mutex<Vec<Value>>,
    restored: Mutex<Vec<String>>,
}

struct FakeEditor {
    operations: Vec<&'static str>,
    state: Arc<EditorState>,
}

impl SessionEditor for FakeEditor {
    fn name(&self) -> &str {
        "claude"
    }

    fn operations(&self) -> &[&str] {
        &self.operations
    }

    fn load(&self, reference: &str) -> DomainResult<EditDocument> {
        self.state.load_calls.fetch_add(1, Ordering::SeqCst);
        Ok(EditDocument::new(
            "claude",
            reference,
            Box::new(()) as Box<dyn Any + Send>,
            Box::new(()) as Box<dyn Any + Send>,
            self.state.revision.lock().unwrap().clone(),
        ))
    }

    fn apply_ops(&self, _doc: &mut EditDocument, ops: &[Value]) -> DomainResult<Vec<Event>> {
        self.state
            .last_ops
            .lock()
            .unwrap()
            .extend(ops.iter().cloned());
        Ok(ops
            .iter()
            .map(|_| Event::new("edit.turn_deleted", Map::new()))
            .collect())
    }

    fn replace_reply(
        &self,
        _doc: &mut EditDocument,
        turn: &Value,
        reply: &Value,
    ) -> DomainResult<Vec<Event>> {
        if !self.operations.contains(&"replace-assistant-reply") {
            return Err(DomainError::operation_unsupported(
                self.name(),
                "replace-assistant-reply",
                Some("inplace"),
            ));
        }
        self.state.replies.lock().unwrap().push(reply.clone());
        let mut params = Map::new();
        params.insert("turn".into(), turn.clone());
        Ok(vec![Event::new("edit.reply_replaced", params)])
    }

    fn validate(&self, _doc: &EditDocument) -> DomainResult<()> {
        Ok(())
    }

    fn stats(&self, _doc: &EditDocument) -> DomainResult<Map<String, Value>> {
        let mut stats = Map::new();
        stats.insert("count".into(), Value::from(2));
        Ok(stats)
    }

    fn commit(&self, _doc: &mut EditDocument) -> DomainResult<Map<String, Value>> {
        if self.state.fail_commit.load(Ordering::SeqCst) {
            return Err(DomainError::agent_request_invalid("commit failed"));
        }
        self.state.commits.fetch_add(1, Ordering::SeqCst);
        let mut result = Map::new();
        result.insert("path".into(), Value::from("/tmp/transcript.jsonl"));
        Ok(result)
    }

    fn snapshot(
        &self,
        _doc: &EditDocument,
        _reason_code: &str,
        _extra: Option<&Map<String, Value>>,
    ) -> DomainResult<Option<PathBuf>> {
        Ok(Some(PathBuf::from("snapshot-before-agent-edit")))
    }

    fn restore_snapshot(&self, snapshot: &Path, _doc: &EditDocument) -> DomainResult<()> {
        self.state
            .restored
            .lock()
            .unwrap()
            .push(snapshot.to_string_lossy().into_owned());
        Ok(())
    }

    fn saved_revision(
        &self,
        _result: &Map<String, Value>,
        doc: &EditDocument,
    ) -> DomainResult<String> {
        Ok(doc.revision.clone())
    }
}

struct FakeVerifier;

impl SessionVerifier for FakeVerifier {
    fn prompt_session(
        &self,
        _session_id: &str,
        _cwd: Option<&str>,
        _prompt: &str,
        _model: Option<&str>,
        _timeout: u64,
    ) -> DomainResult<Map<String, Value>> {
        Ok(Map::new())
    }
}

#[derive(Default)]
struct LifecycleState {
    deleted: Mutex<Vec<String>>,
    fail: AtomicBool,
}

struct FakeLifecycle {
    state: Arc<LifecycleState>,
}

impl SessionLifecycle for FakeLifecycle {
    fn resume_descriptor(&self, _session_id: &str, _cwd: &str) -> DomainResult<Map<String, Value>> {
        Ok(Map::new())
    }

    fn cleanup(&self, _session_id: &str, _dest: &Path) -> DomainResult<()> {
        Ok(())
    }

    fn validation_ref(&self, session_id: &str, _dest: &Path) -> DomainResult<String> {
        Ok(session_id.to_string())
    }

    fn delete(&self, _adapter: &AgentAdapter, reference: &str) -> DomainResult<Map<String, Value>> {
        if self.state.fail.load(Ordering::SeqCst) {
            return Err(DomainError::agent_request_invalid("删除失败"));
        }
        self.state
            .deleted
            .lock()
            .unwrap()
            .push(reference.to_string());
        Ok(Map::new())
    }
}

struct FakeBrowser;

impl SessionBrowser for FakeBrowser {
    fn scan(&self, _cache: &dyn ScanCache) -> DomainResult<Vec<ScanRow>> {
        Ok(Vec::new())
    }
    fn read(&self, reference: &str) -> DomainResult<Session> {
        Ok(Session::new("claude", reference, "/tmp"))
    }
    fn read_agent(&self, reference: &str) -> DomainResult<Session> {
        self.read(reference)
    }
    fn resolve_ref(&self, reference: &str) -> DomainResult<String> {
        Ok(reference.to_string())
    }
    fn fingerprint(&self, _reference: &str) -> DomainResult<Fingerprint> {
        Ok(Value::from("fingerprint"))
    }
    fn agent_fingerprint(&self, reference: &str) -> DomainResult<Fingerprint> {
        self.fingerprint(reference)
    }
    fn canonicalize(&self, _row: &ScanRow) -> Option<NativeSessionReference> {
        None
    }
    fn validate_read_scope(&self, _reference: &NativeSessionReference) -> DomainResult<()> {
        Ok(())
    }
}

struct FakeModels;

impl ModelCatalog for FakeModels {
    fn discover(&self) -> DomainResult<ModelDiscovery> {
        Ok(ModelDiscovery::default())
    }
    fn fallback(&self) -> Vec<Map<String, Value>> {
        Vec::new()
    }
}

fn manifest(edit_operations: &[&str]) -> AgentManifest {
    AgentManifest {
        id: "claude".into(),
        display_name: "Claude Code".into(),
        icon: "claude".into(),
        source_path: "~/.claude/projects".into(),
        // 顺序必须是 AGENT_CAPABILITIES 的有序子集。
        capabilities: ["browse", "resume", "edit", "delete", "prompt", "models"]
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        edit_operations: edit_operations
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        executables: vec!["claude".into()],
        fallback_bin_dirs: Vec::new(),
    }
}

struct FakePorts {
    adapter: AgentAdapter,
    state_dir: PathBuf,
}

impl OperationPorts for FakePorts {
    fn adapter(&self, tool: &str) -> DomainResult<AgentAdapter> {
        if tool == "claude" {
            Ok(self.adapter.clone())
        } else {
            Err(DomainError::tool_unknown(tool))
        }
    }

    fn adapters(&self) -> Vec<String> {
        vec!["claude".into(), "opencode".into()]
    }

    fn state_dir(&self) -> PathBuf {
        self.state_dir.clone()
    }
}

#[derive(Default)]
struct IndexState {
    revision: Mutex<String>,
    session_id: Mutex<String>,
    missing: AtomicBool,
    evicted: Mutex<Vec<String>>,
}

struct FakeResolver {
    state: Arc<IndexState>,
}

impl SessionResolver for FakeResolver {
    fn resolve(&self, tool: &str, reference: &str) -> DomainResult<IndexedSession> {
        if self.state.missing.load(Ordering::SeqCst) {
            let mut params = Map::new();
            params.insert("reason".into(), Value::from("session_changed"));
            return Err(DomainError::new(
                "agent.reference_invalid",
                "AgentReferenceError",
                "会话引用已失效",
                params,
            ));
        }
        let mut row = Map::new();
        row.insert(
            "id".into(),
            Value::from(self.state.session_id.lock().unwrap().clone()),
        );
        row.insert("title".into(), Value::from("标题"));
        row.insert("dir".into(), Value::from("/tmp/project"));
        row.insert("size".into(), Value::from(1024));
        row.insert("updated".into(), Value::from(1_700_000_000_000_i64));
        Ok(IndexedSession {
            tool: tool.to_string(),
            opaque_ref: reference.to_string(),
            canonical_ref: "/tmp/transcript.jsonl".to_string(),
            revision: self.state.revision.lock().unwrap().clone(),
            row,
        })
    }

    fn resolve_message_locator(
        &self,
        _record: &IndexedSession,
        locator: &str,
    ) -> DomainResult<ResolvedMessageLocator> {
        Ok(ResolvedMessageLocator {
            native_locator: format!("native::{locator}"),
            editable: true,
        })
    }

    fn evict(&self, _tool: &str, canonical_ref: &str) -> DomainResult<()> {
        self.state
            .evicted
            .lock()
            .unwrap()
            .push(canonical_ref.to_string());
        Ok(())
    }

    fn read_indexed_session(&self, record: &IndexedSession) -> DomainResult<Session> {
        Ok(Session::new("claude", &record.canonical_ref, "/tmp"))
    }
}

// ---------------------------------------------------------------------------
// 装配
// ---------------------------------------------------------------------------

struct Harness {
    _dir: tempfile::TempDir,
    state_dir: PathBuf,
    ports: Ports,
    index: Resolver,
    clock: Arc<TestClock>,
    editor: Arc<EditorState>,
    lifecycle: Arc<LifecycleState>,
    index_state: Arc<IndexState>,
}

impl Harness {
    fn new() -> Self {
        Self::with_edit_operations(&["delete-turn", "rewrite", "replace-assistant-reply"])
    }

    fn with_edit_operations(edit_operations: &'static [&'static str]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().to_path_buf();
        let editor_state = Arc::new(EditorState {
            revision: Mutex::new("revision-1".into()),
            ..EditorState::default()
        });
        let lifecycle_state = Arc::new(LifecycleState::default());
        let adapter = AgentAdapter::builder()
            .browser(Arc::new(FakeBrowser))
            .editor(Arc::new(FakeEditor {
                operations: edit_operations.to_vec(),
                state: Arc::clone(&editor_state),
            }))
            .verifier(Arc::new(FakeVerifier))
            .lifecycle(Arc::new(FakeLifecycle {
                state: Arc::clone(&lifecycle_state),
            }))
            .models(Arc::new(FakeModels))
            .build(manifest(edit_operations))
            .expect("假 adapter 装配失败");
        let index_state = Arc::new(IndexState {
            revision: Mutex::new("index-revision-1".into()),
            session_id: Mutex::new("private-id".into()),
            ..IndexState::default()
        });
        Self {
            state_dir: state_dir.clone(),
            ports: Arc::new(FakePorts {
                adapter,
                state_dir: state_dir.clone(),
            }),
            index: Arc::new(FakeResolver {
                state: Arc::clone(&index_state),
            }),
            clock: Arc::new(TestClock {
                now: Mutex::new(1_000),
            }),
            editor: editor_state,
            lifecycle: lifecycle_state,
            index_state,
            _dir: dir,
        }
    }

    fn service(&self) -> OperationService {
        OperationService::with_clock(
            Ports::clone(&self.ports),
            Resolver::clone(&self.index),
            Arc::clone(&self.clock) as Arc<dyn Clock>,
        )
    }

    fn database(&self) -> StateDatabase {
        StateDatabase::open(self.state_dir.join("ferry-state.sqlite3"), false).unwrap()
    }

    fn commits(&self) -> usize {
        self.editor.commits.load(Ordering::SeqCst)
    }
}

fn edit_plan(ops: Value) -> Value {
    json!({
        "kind": "edit",
        "tool": "claude",
        "ref": "fsr_abcdefgh",
        "ops": ops,
    })
}

fn default_ops() -> Value {
    json!([{"op": "delete-turn", "turn": 1}])
}

/// 等待终态的上限。并行跑测试时机器可能被别的编译任务挤住，给足余量：
/// 超时会让 `wait` 静默回落到当前状态，把真实断言变成时序噪声。
const WAIT_TIMEOUT: Duration = Duration::from_secs(60);

/// 断言 apply 已入队，并等待终态。
fn apply_and_wait(service: &OperationService, plan_id: &Value) -> EngineResult<Value> {
    let accepted = service.apply(plan_id)?;
    assert!(
        matches!(
            accepted["status"].as_str(),
            Some("queued" | "applying" | "applied")
        ),
        "accepted={accepted}"
    );
    service.wait(plan_id, Some(WAIT_TIMEOUT))
}

fn audit_events(service: &OperationService, plan_id: &Value) -> Vec<String> {
    service
        .audit(plan_id)
        .unwrap()
        .into_iter()
        .map(|entry| entry.event)
        .collect()
}

// ---------------------------------------------------------------------------
// 场景
// ---------------------------------------------------------------------------

#[test]
fn plan_freezes_input_and_apply_only_uses_plan_id() {
    let harness = Harness::new();
    let service = harness.service();
    let plan = service.plan(&edit_plan(default_ops())).unwrap();

    assert_eq!(plan["status"], json!("planned"));
    assert_eq!(plan["base_revision"], json!("index-revision-1"));
    assert_eq!(plan["document_revision"], json!("revision-1"));
    assert!(plan["input_digest"].as_str().unwrap().len() == 64);
    assert!(plan["preview_digest"].as_str().unwrap().len() == 64);
    assert_eq!(plan["risk"], json!("high"));
    assert_eq!(plan["affected_refs"], json!(["fsr_abcdefgh"]));

    let applied = apply_and_wait(&service, &plan["plan_id"]).unwrap();
    assert_eq!(applied["status"], json!("applied"));
    assert_eq!(
        applied["result"]["snapshot"],
        json!("snapshot-before-agent-edit")
    );
    assert_eq!(applied["result"]["ok"], json!(true));
    assert_eq!(harness.commits(), 1);
    assert_eq!(
        service.status(&plan["plan_id"]).unwrap()["status"],
        json!("applied")
    );
}

#[test]
fn audit_sequence_is_planned_queued_applying_applied() {
    let harness = Harness::new();
    let service = harness.service();
    let secret = "Bearer operation-audit-secret";
    let plan = service
        .plan(&edit_plan(json!([{
            "op": "replace-assistant-reply", "turn": 1,
            "reply": {"items": [{"kind": "text", "text": secret}]},
        }])))
        .unwrap();
    apply_and_wait(&service, &plan["plan_id"]).unwrap();

    let encoded = serde_json::to_string(&Value::Array(
        service
            .audit(&plan["plan_id"])
            .unwrap()
            .iter()
            .map(ferry_engine::operations::state_store::AuditEntry::to_value)
            .collect(),
    ))
    .unwrap();
    assert!(!encoded.contains(secret), "审计不得落原文: {encoded}");
    assert_eq!(
        audit_events(&service, &plan["plan_id"]),
        ["planned", "queued", "applying", "applied"]
    );
}

#[test]
fn plan_can_only_be_applied_once() {
    let harness = Harness::new();
    let service = harness.service();
    let plan = service.plan(&edit_plan(default_ops())).unwrap();

    apply_and_wait(&service, &plan["plan_id"]).unwrap();
    let error = service.apply(&plan["plan_id"]).unwrap_err();
    assert_eq!(error.error_type(), "AgentRequestError");
    assert_eq!(error.message(), "operation plan 当前状态不可执行");
    assert_eq!(harness.commits(), 1);
}

#[test]
fn cancelled_plan_never_writes() {
    let harness = Harness::new();
    let service = harness.service();
    let plan = service.plan(&edit_plan(default_ops())).unwrap();

    let cancelled = service.cancel(&plan["plan_id"]).unwrap();
    assert_eq!(cancelled["status"], json!("cancelled"));
    let error = service.apply(&plan["plan_id"]).unwrap_err();
    assert_eq!(error.message(), "operation plan 当前状态不可执行");

    assert_eq!(harness.commits(), 0);
    assert_eq!(
        service.status(&plan["plan_id"]).unwrap()["status"],
        json!("cancelled")
    );
    assert_eq!(
        audit_events(&service, &plan["plan_id"]),
        ["planned", "cancelled"]
    );
}

#[test]
fn expired_plan_cannot_be_applied() {
    let harness = Harness::new();
    let service = harness.service();
    let plan = service.plan(&edit_plan(default_ops())).unwrap();

    harness.clock.advance(10 * 60 * 1000 + 1);

    assert_eq!(
        service.status(&plan["plan_id"]).unwrap()["status"],
        json!("expired")
    );
    let error = service.apply(&plan["plan_id"]).unwrap_err();
    assert_eq!(error.message(), "operation plan 当前状态不可执行");
    assert_eq!(harness.commits(), 0);
    assert_eq!(
        audit_events(&service, &plan["plan_id"]),
        ["planned", "expired"]
    );
}

#[test]
fn restart_marks_interrupted_apply_failed() {
    let harness = Harness::new();
    let plan = {
        let service = harness.service();
        service.plan(&edit_plan(default_ops())).unwrap()
    };
    // 模拟「引擎在 applying 中途被杀」。
    assert!(harness
        .database()
        .operations
        .claim(plan["plan_id"].as_str().unwrap(), 2_000)
        .unwrap());

    // 新的 OperationService 打开状态库时跑一次崩溃恢复。
    let service = harness.service();
    let status = service.status(&plan["plan_id"]).unwrap();
    assert_eq!(status["status"], json!("failed"));
    assert_eq!(status["error_type"], json!("EngineRestarted"));
}

#[test]
fn metadata_paths_never_trigger_crash_recovery() {
    let harness = Harness::new();
    let service = harness.service();
    let plan = service.plan(&edit_plan(default_ops())).unwrap();
    assert!(harness
        .database()
        .operations
        .claim(plan["plan_id"].as_str().unwrap(), 2_000)
        .unwrap());

    // metadata.list_all 用 recover_interrupted=false 的连接。
    assert!(metadata::list_all(&harness.ports).unwrap().is_empty());
    assert_eq!(
        service.status(&plan["plan_id"]).unwrap()["status"],
        json!("applying")
    );
}

#[test]
fn metadata_plan_applies_with_independent_cas() {
    let harness = Harness::new();
    let service = harness.service();
    let plan = service
        .plan(&json!({
            "kind": "metadata", "tool": "claude", "ref": "fsr_abcdefgh",
            "patch": {"name": "新名称"},
        }))
        .unwrap();

    assert_eq!(plan["kind"], json!("metadata"));
    assert_eq!(plan["risk"], json!("low"));
    assert_eq!(plan["preview"]["before"], json!({}));
    assert_eq!(plan["preview"]["after_patch"], json!({"name": "新名称"}));

    let applied = apply_and_wait(&service, &plan["plan_id"]).unwrap();
    assert_eq!(applied["result"]["metadata"], json!({"name": "新名称"}));
    assert_eq!(
        metadata::list_all(&harness.ports).unwrap()["claude\u{0}private-id"],
        json!({"name": "新名称"})
    );
}

#[test]
fn metadata_plan_rejects_concurrent_metadata_change() {
    let harness = Harness::new();
    let service = harness.service();
    let plan = service
        .plan(&json!({
            "kind": "metadata", "tool": "claude", "ref": "fsr_abcdefgh",
            "patch": {"name": "新名称"},
        }))
        .unwrap();

    let mut patch = Map::new();
    patch.insert("name".into(), Value::from("并发名称"));
    metadata::set_entry("claude", "private-id", &patch, &harness.ports).unwrap();

    let error = apply_and_wait(&service, &plan["plan_id"]).unwrap_err();
    assert_eq!(error.error_type(), "ConcurrentModificationError");
    assert_eq!(error.message(), "会话元数据在审批后已变化");
    assert_eq!(
        service.status(&plan["plan_id"]).unwrap()["status"],
        json!("failed")
    );
    assert_eq!(
        metadata::list_all(&harness.ports).unwrap()["claude\u{0}private-id"],
        json!({"name": "并发名称"})
    );
}

#[test]
fn apply_rejects_changed_index_revision() {
    let harness = Harness::new();
    let service = harness.service();
    let plan = service.plan(&edit_plan(default_ops())).unwrap();
    *harness.index_state.revision.lock().unwrap() = "index-revision-2".into();

    let error = apply_and_wait(&service, &plan["plan_id"]).unwrap_err();
    assert_eq!(error.error_type(), "ConcurrentModificationError");
    assert_eq!(error.message(), "会话在操作计划生成后已变化，请重新计划");

    let status = service.status(&plan["plan_id"]).unwrap();
    assert_eq!(status["status"], json!("failed"));
    assert_eq!(status["error_type"], json!("ConcurrentModificationError"));
    // 类名之外还要带上人话原因，否则宿主只能显示一个异常类名。
    assert_eq!(
        status["error_message"],
        json!("会话在操作计划生成后已变化，请重新计划")
    );
    let audit: Vec<Value> = service
        .audit(&plan["plan_id"])
        .unwrap()
        .iter()
        .map(|entry| entry.to_value())
        .collect();
    let failed = audit
        .iter()
        .find(|entry| entry["event"] == json!("failed"))
        .expect("必须有 failed 审计");
    assert_eq!(
        failed["details"]["error_message"],
        json!("会话在操作计划生成后已变化，请重新计划")
    );
    assert_eq!(harness.commits(), 0);
}

#[test]
fn apply_rejects_changed_document_revision() {
    let harness = Harness::new();
    let service = harness.service();
    let plan = service.plan(&edit_plan(default_ops())).unwrap();
    *harness.editor.revision.lock().unwrap() = "revision-2".into();

    let error = apply_and_wait(&service, &plan["plan_id"]).unwrap_err();
    assert_eq!(error.error_type(), "ConcurrentModificationError");
    assert_eq!(error.message(), "源会话在预览后已变化，请重新预览");
    assert_eq!(
        service.status(&plan["plan_id"]).unwrap()["status"],
        json!("failed")
    );
    assert_eq!(harness.commits(), 0);
    // ConcurrentModificationError 不还原快照。
    assert!(harness.editor.restored.lock().unwrap().is_empty());
}

#[test]
fn commit_failure_restores_snapshot_and_marks_plan_failed() {
    let harness = Harness::new();
    let service = harness.service();
    let plan = service.plan(&edit_plan(default_ops())).unwrap();
    harness.editor.fail_commit.store(true, Ordering::SeqCst);

    let error = apply_and_wait(&service, &plan["plan_id"]).unwrap_err();
    assert_eq!(error.message(), "commit failed");
    assert_eq!(
        harness.editor.restored.lock().unwrap().clone(),
        ["snapshot-before-agent-edit"]
    );
    assert_eq!(
        service.status(&plan["plan_id"]).unwrap()["status"],
        json!("failed")
    );
}

#[test]
fn replace_reply_requires_the_editor_operation() {
    let harness = Harness::with_edit_operations(&["delete-turn", "rewrite"]);
    let service = harness.service();
    let error = service
        .plan(&edit_plan(json!([{
            "op": "replace-assistant-reply", "turn": 1,
            "reply": {"items": [{"kind": "text", "text": "x"}]},
        }])))
        .unwrap_err();
    assert_eq!(error.error_type(), "OperationUnsupportedError");
    assert_eq!(harness.commits(), 0);
}

#[test]
fn delete_plan_is_read_only_and_apply_deletes_permanently() {
    let harness = Harness::new();
    let service = harness.service();
    let plan = service
        .plan(&json!({"kind": "delete", "tool": "claude", "refs": ["fsr_abcdefgh"]}))
        .unwrap();

    assert_eq!(plan["kind"], json!("delete"));
    assert_eq!(plan["risk"], json!("high"));
    assert_eq!(plan["preview"]["permanent"], json!(true));
    assert_eq!(plan["preview"]["totals"]["count"], json!(1));
    assert_eq!(plan["preview"]["excluded"], json!([]));
    assert_eq!(plan["affected_refs"], json!(["fsr_abcdefgh"]));
    assert!(harness.lifecycle.deleted.lock().unwrap().is_empty());

    let applied = apply_and_wait(&service, &plan["plan_id"]).unwrap();
    let result = &applied["result"];
    assert_eq!(
        result["succeeded"],
        json!([{"tool": "claude", "ref": "fsr_abcdefgh"}])
    );
    assert_eq!(result["skipped"], json!([]));
    assert_eq!(result["failed"], json!([]));
    assert!(!serde_json::to_string(result)
        .unwrap()
        .contains("recovery_id"));
    assert_eq!(
        harness.lifecycle.deleted.lock().unwrap().clone(),
        ["/tmp/transcript.jsonl"]
    );
    // 删除成功立即 evict，推 removal delta。
    assert_eq!(
        harness.index_state.evicted.lock().unwrap().clone(),
        ["/tmp/transcript.jsonl"]
    );
}

#[test]
fn delete_apply_skips_sessions_changed_after_plan() {
    let harness = Harness::new();
    let service = harness.service();
    let plan = service
        .plan(&json!({"kind": "delete", "tool": "claude", "refs": ["fsr_abcdefgh"]}))
        .unwrap();
    *harness.index_state.revision.lock().unwrap() = "index-revision-2".into();

    let applied = apply_and_wait(&service, &plan["plan_id"]).unwrap();
    let result = &applied["result"];
    assert_eq!(result["succeeded"], json!([]));
    assert_eq!(
        result["skipped"],
        json!([{"tool": "claude", "ref": "fsr_abcdefgh", "cause": "changed"}])
    );
    assert!(harness.lifecycle.deleted.lock().unwrap().is_empty());
}

#[test]
fn delete_apply_skips_sessions_protected_after_plan() {
    let harness = Harness::new();
    let service = harness.service();
    let plan = service
        .plan(&json!({"kind": "delete", "tool": "claude", "refs": ["fsr_abcdefgh"]}))
        .unwrap();

    let mut patch = Map::new();
    patch.insert("pinned".into(), Value::Bool(true));
    metadata::set_entry("claude", "private-id", &patch, &harness.ports).unwrap();

    let applied = apply_and_wait(&service, &plan["plan_id"]).unwrap();
    assert_eq!(
        applied["result"]["skipped"],
        json!([{
            "tool": "claude", "ref": "fsr_abcdefgh",
            "cause": "protected", "protection": "pinned",
        }])
    );
    assert!(harness.lifecycle.deleted.lock().unwrap().is_empty());
}

#[test]
fn delete_plan_excludes_protected_sessions_at_plan_time() {
    let harness = Harness::new();
    let mut patch = Map::new();
    patch.insert("pinned".into(), Value::Bool(true));
    metadata::set_entry("claude", "private-id", &patch, &harness.ports).unwrap();
    let service = harness.service();

    let plan = service
        .plan(&json!({"kind": "delete", "tool": "claude", "refs": ["fsr_abcdefgh"]}))
        .unwrap();

    assert_eq!(plan["preview"]["totals"]["count"], json!(0));
    assert_eq!(plan["preview"]["excluded"][0]["cause"], json!("pinned"));
    // targets 为空 → apply 什么都不删。
    let applied = apply_and_wait(&service, &plan["plan_id"]).unwrap();
    assert_eq!(applied["result"]["succeeded"], json!([]));
    assert!(harness.lifecycle.deleted.lock().unwrap().is_empty());
}

#[test]
fn delete_batch_keeps_going_after_a_single_failure() {
    let harness = Harness::new();
    let service = harness.service();
    let plan = service
        .plan(&json!({"kind": "delete", "tool": "claude", "refs": ["fsr_abcdefgh"]}))
        .unwrap();
    harness.lifecycle.fail.store(true, Ordering::SeqCst);

    let applied = apply_and_wait(&service, &plan["plan_id"]).unwrap();
    assert_eq!(applied["status"], json!("applied"));
    assert_eq!(
        applied["result"]["failed"],
        json!([{"tool": "claude", "ref": "fsr_abcdefgh", "error": "删除失败"}])
    );
    assert_eq!(applied["result"]["succeeded"], json!([]));
}

#[test]
fn unknown_operation_kind_is_rejected() {
    let harness = Harness::new();
    let service = harness.service();
    let error = service.plan(&json!({"kind": "unknown"})).unwrap_err();
    assert_eq!(error.error_type(), "AgentRequestError");
    assert_eq!(error.message(), "operation kind 非法");
}

#[test]
fn concurrent_apply_only_commits_once() {
    let harness = Harness::new();
    let service = Arc::new(harness.service());
    let plan = service.plan(&edit_plan(default_ops())).unwrap();
    let plan_id = plan["plan_id"].clone();

    let handles: Vec<_> = (0..2)
        .map(|_| {
            let service = Arc::clone(&service);
            let plan_id = plan_id.clone();
            std::thread::spawn(move || service.apply(&plan_id).map(|_| ()))
        })
        .collect();
    let outcomes: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    service.wait(&plan_id, Some(WAIT_TIMEOUT)).unwrap();

    assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
    let failure = outcomes
        .iter()
        .find_map(|outcome| outcome.as_ref().err())
        .expect("必须恰好有一次失败");
    assert_eq!(failure.error_type(), "AgentRequestError");
    assert_eq!(harness.commits(), 1);
    assert_eq!(
        service.status(&plan_id).unwrap()["status"],
        json!("applied")
    );
}

#[test]
fn plan_survives_service_restart() {
    let harness = Harness::new();
    let plan = {
        let service = harness.service();
        service.plan(&edit_plan(default_ops())).unwrap()
    };

    let service = harness.service();
    assert_eq!(
        service.status(&plan["plan_id"]).unwrap()["status"],
        json!("planned")
    );
    let applied = apply_and_wait(&service, &plan["plan_id"]).unwrap();
    assert_eq!(applied["status"], json!("applied"));
    assert_eq!(harness.commits(), 1);
}

#[test]
fn unknown_plan_id_is_rejected() {
    let harness = Harness::new();
    let service = harness.service();
    for bad in [json!("nope"), json!(1), json!(null)] {
        let error = service.status(&bad).unwrap_err();
        assert_eq!(error.message(), "plan_id 非法", "bad={bad}");
    }
    let error: EngineError = service.status(&json!("op_missing")).unwrap_err();
    assert_eq!(error.message(), "operation plan 不存在或已因重启失效");
}
