//! 迁移端到端：黄金 fixture 起源会话 →`operation.plan(migration)` → `operation.apply`。
//!
//! 与 `operations_lifecycle.rs` 的分工：那边用假 adapter 把 operations 单独拎出来
//! 跑状态机；这边**只用真件**——真组合根（`bootstrap::build_engine`）、真索引、真
//! 适配器、真状态库，验证 WP-B/C/D/E 拼起来之后迁移链路是通的。
//!
//! 断言的三处硬约束（方案 §2.4 第 25 条 / §WP-E）：
//! - `plan` 的 preview 是 schema v3；
//! - `apply` 的 `validation.structure.ok` 为真（`validate_written_tree` 的 re-read 验收）；
//! - 每条 apply 都落一条 `migration_history`。
//!
//! 三条组合：`claude → codex`、`codex → claude` 与 `claude → cursor`——这三个
//! 方向的 writer 都无外部 CLI 依赖，可完整关在沙箱 HOME 里（pi/grok/opencode 的写
//! 路径要拉起真实 CLI）。cursor 直接写 `state.vscdb`，沙箱用 `FERRY_CURSOR_DB` 把
//! 库指进 HOME，并预置一行「同一工作区已经聊过」的 header，迁入才认得工作区哈希。

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

use ferry_engine::bootstrap::build_engine;
use ferry_engine::server::rpc::EngineService;

/// 进程级环境是共享的；本二进制里改 HOME 的用例必须串行。
static ENVIRONMENT: Mutex<()> = Mutex::new(());

const APPLY_TIMEOUT: Duration = Duration::from_secs(120);
/// 迁移的三条组合。只选 writer 无外部 CLI 依赖的方向：pi/grok/opencode 的写路径
/// 都要拉起各自的真实 CLI 做验收，沙箱与 CI 都不能依赖它们。
const COMBOS: &[(&str, &str)] = &[
    ("claude", "codex"),
    ("codex", "claude"),
    ("claude", "cursor"),
];

struct Sandbox {
    _guard: std::sync::MutexGuard<'static, ()>,
    restore: Vec<(&'static str, Option<String>)>,
    root: tempfile::TempDir,
}

impl Sandbox {
    fn enter() -> Self {
        let guard = ENVIRONMENT
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let root = tempfile::tempdir().expect("临时沙箱可创建");
        let home = root.path().to_path_buf();
        let mut sandbox = Self {
            _guard: guard,
            restore: Vec::new(),
            root,
        };
        // 全部数据根都指向沙箱：绝不碰运行者的真实会话与状态库。
        sandbox.set("HOME", &home);
        sandbox.set("USERPROFILE", &home);
        sandbox.set("XDG_DATA_HOME", &home.join(".local/share"));
        sandbox.set("XDG_CONFIG_HOME", &home.join(".config"));
        sandbox.set("FERRY_DATA_DIR", &home.join(".ferry"));
        sandbox.set("FERRY_BACKUP_DIR", &home.join(".ferry/backups"));
        sandbox.set("FERRY_OPENCODE_DB", &home.join("opencode/storage.db"));
        // 不能让 cursor 落在平台默认位置：那会触发「Cursor 是否在运行」的进程门禁，
        // 开发机上开着 Cursor 就会把用例打成失败。
        sandbox.set("FERRY_CURSOR_DB", &home.join("cursor/state.vscdb"));
        sandbox.set("GROK_HOME", &home.join(".grok"));
        sandbox.set("PI_CODING_AGENT_SESSION_DIR", &home.join("pi-sessions"));
        // 这两个变量只要存在，就会把 codex / pi 的探测拽回运行者的真实目录，
        // 必须显式清掉。
        sandbox.unset("CODEX_HOME");
        sandbox.unset("PI_CODING_AGENT_DIR");
        std::fs::create_dir_all(home.join(".ferry")).expect("状态目录可创建");
        // 扫描根缓存记的是上一轮沙箱的 realpath，换 HOME 必须清掉。
        ferry_engine::adapters::contracts::clear_resolved_root_cache();
        sandbox
    }

    fn set(&mut self, key: &'static str, value: &Path) {
        self.restore.push((key, std::env::var(key).ok()));
        // SAFETY: 本用例持有 ENVIRONMENT 锁，进程内无并发写。
        unsafe { std::env::set_var(key, value) };
    }

    fn unset(&mut self, key: &'static str) {
        self.restore.push((key, std::env::var(key).ok()));
        // SAFETY: 见 set。
        unsafe { std::env::remove_var(key) };
    }

    fn home(&self) -> &Path {
        self.root.path()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        for (key, previous) in self.restore.drain(..).rev() {
            // SAFETY: 见 set。
            unsafe {
                match previous {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
        ferry_engine::adapters::contracts::clear_resolved_root_cache();
    }
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("仓库根可解析")
}

/// 把 claude 黄金 fixture 物化成真实存储布局：
/// `<home>/.claude/projects/<case>/<session-id>.jsonl`。
/// 预建 codex 的 `state_5.sqlite`：真实 codex 环境必有该库，register_tree 对缺库
/// 直接报错，所以沙箱必须补齐。schema 与 `tests/golden_regen.rs` 的
/// `CODEX_STATE_SCHEMA` 一致。
fn seed_codex_state(home: &Path) {
    let codex_dir = home.join(".codex");
    std::fs::create_dir_all(&codex_dir).expect("codex 目录可创建");
    let connection =
        rusqlite::Connection::open(codex_dir.join("state_5.sqlite")).expect("状态库可创建");
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS threads (
                 id TEXT PRIMARY KEY, rollout_path TEXT NOT NULL,
                 created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
                 source TEXT NOT NULL, model_provider TEXT NOT NULL, cwd TEXT NOT NULL,
                 title TEXT NOT NULL, sandbox_policy TEXT NOT NULL,
                 approval_mode TEXT NOT NULL, tokens_used INTEGER NOT NULL DEFAULT 0,
                 has_user_event INTEGER NOT NULL DEFAULT 0,
                 archived INTEGER NOT NULL DEFAULT 0,
                 cli_version TEXT NOT NULL DEFAULT '',
                 first_user_message TEXT NOT NULL DEFAULT '',
                 agent_path TEXT, thread_source TEXT, preview TEXT NOT NULL DEFAULT '',
                 recency_at INTEGER NOT NULL DEFAULT 0,
                 history_mode TEXT NOT NULL DEFAULT 'legacy'
             );
             CREATE TABLE IF NOT EXISTS thread_spawn_edges (
                 parent_thread_id TEXT NOT NULL,
                 child_thread_id TEXT NOT NULL PRIMARY KEY,
                 status TEXT NOT NULL
             );",
        )
        .expect("状态库 schema 可建");
}

/// 把 codex 黄金 fixture 物化成真实存储布局（对齐 dump 脚本的 prepare_codex）：
/// rollout JSONL 进 `.codex/sessions/2026/07/25/`，registration.json 的 threads
/// 行进 `state_5.sqlite`（rollout_path 重写为物化后的真实路径）。
fn seed_codex(home: &Path, case: &str) {
    let fixture = repository_root()
        .join("tests/fixtures/agent_formats/codex")
        .join(case);
    let manifest: Value = serde_json::from_str(
        &std::fs::read_to_string(fixture.join("manifest.json")).expect("manifest 可读"),
    )
    .expect("manifest 是 JSON");
    let stem = manifest["source_paths"][0]
        .as_str()
        .and_then(|source| Path::new(source).file_stem())
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| format!("rollout-{case}"));
    let target = home
        .join(".codex/sessions/2026/07/25")
        .join(format!("{stem}.jsonl"));
    std::fs::create_dir_all(target.parent().expect("有父目录")).expect("会话目录可创建");
    std::fs::copy(fixture.join("session.jsonl"), &target).expect("fixture 可复制");

    seed_codex_state(home);
    let registration = fixture.join("registration.json");
    if let Ok(text) = std::fs::read_to_string(&registration) {
        let payload: Value = serde_json::from_str(&text).expect("registration 是 JSON");
        let connection =
            rusqlite::Connection::open(home.join(".codex/state_5.sqlite")).expect("状态库可打开");
        for thread in payload["threads"].as_array().into_iter().flatten() {
            connection
                .execute(
                    "INSERT OR REPLACE INTO threads (id, rollout_path, created_at, \
                     updated_at, source, model_provider, cwd, title, sandbox_policy, \
                     approval_mode, cli_version, first_user_message) \
                     VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
                    rusqlite::params![
                        thread["id"].as_str().unwrap_or(""),
                        target.to_string_lossy(),
                        0_i64,
                        0_i64,
                        "cli",
                        "openai",
                        thread["cwd"].as_str().unwrap_or(""),
                        thread["title"].as_str().unwrap_or(""),
                        "workspace-write",
                        "on-request",
                        thread["cli_version"].as_str().unwrap_or(""),
                        thread["first_user_message"].as_str().unwrap_or(""),
                    ],
                )
                .expect("threads 行可写入");
        }
    }
}

fn seed_claude(home: &Path, case: &str) {
    let fixture = repository_root()
        .join("tests/fixtures/agent_formats/claude")
        .join(case);
    let manifest: Value = serde_json::from_str(
        &std::fs::read_to_string(fixture.join("manifest.json")).expect("manifest 可读"),
    )
    .expect("manifest 是 JSON");
    let stem = manifest["source_paths"][0]
        .as_str()
        .and_then(|source| Path::new(source).file_stem())
        .map(|stem| stem.to_string_lossy().into_owned())
        .or_else(|| {
            manifest["session_id"]
                .as_str()
                .map(std::string::ToString::to_string)
        })
        .unwrap_or_else(|| case.to_string());
    let target = home
        .join(".claude/projects")
        .join(case)
        .join(format!("{stem}.jsonl"));
    std::fs::create_dir_all(target.parent().expect("有父目录")).expect("会话目录可创建");
    std::fs::copy(fixture.join("session.jsonl"), &target).expect("fixture 可复制");
}

/// 预建 Cursor 的 `state.vscdb`：真实环境里这个库一直存在，Ferry 只往里加行、
/// 从不建库。再塞一条「同一工作区已经聊过」的既有会话：header 让迁入认得工作区哈希
/// （`workspace::resolve` 的第一条通路），composerData 则是 Cursor 自己写的**真实形态**
/// ——`context` 是一个"对象非空、叶子全空"的结构，外加一处别的会话的真实内容。
/// 少了这条样本，写入路径的模板分支根本不会被走到（BUG-1 就是这么漏掉的）。
fn seed_cursor(home: &Path, cwd: &str) {
    let directory = home.join("cursor");
    std::fs::create_dir_all(&directory).expect("cursor 目录可创建");
    let connection =
        rusqlite::Connection::open(directory.join("state.vscdb")).expect("cursor 库可创建");
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, \
             value BLOB);\
             CREATE TABLE IF NOT EXISTS cursorDiskKV (key TEXT UNIQUE ON CONFLICT REPLACE, \
             value BLOB);\
             CREATE TABLE IF NOT EXISTS composerHeaders (composerId TEXT PRIMARY KEY, \
             workspaceId TEXT, createdAt INTEGER, lastUpdatedAt INTEGER, isArchived INTEGER, \
             isSubagent INTEGER, recency INTEGER, checkpointAt INTEGER, value TEXT);",
        )
        .expect("cursor schema 可建");
    let head = json!({
        "type": "head", "composerId": "seed-existing", "name": "seed",
        "createdAt": 1_i64,
        "workspaceIdentifier": {"id": "ws-seed", "uri": {
            "$mid": 1, "scheme": "file", "fsPath": cwd, "path": cwd,
            "external": format!("file://{cwd}")}},
    });
    connection
        .execute(
            "INSERT OR REPLACE INTO composerHeaders (composerId, workspaceId, createdAt, \
             lastUpdatedAt, isArchived, isSubagent, recency, checkpointAt, value) \
             VALUES ('seed-existing', 'ws-seed', 1, 1, 0, 0, 1, NULL, ?)",
            [head.to_string()],
        )
        .expect("cursor header 行可写入");
    let existing = json!({
        "_v": 17, "composerId": "seed-existing", "name": "seed",
        "fullConversationHeadersOnly": [],
        "context": {
            "composers": [], "selectedCommits": [], "selectedPullRequests": [],
            "selectedImages": [], "selectedDocuments": [], "selectedVideos": [],
            "folderSelections": [],
            "fileSelections": [{"uri": {"$mid": 1, "path": format!("{cwd}/SEEDED.md"),
                                        "scheme": "file"},
                                "uuid": "seed-uuid"}],
            "selections": [], "terminalSelections": [], "selectedDocs": [],
            "externalLinks": [], "cursorRules": [], "cursorCommands": [],
            "gitPRDiffSelections": [], "subagentSelections": [], "browserSelections": [],
            "extraContext": [],
            "mentions": {"composers": {}, "gitDiff": [], "fileSelections": {},
                         "diffHistory": [], "consoleLogs": []}
        }
    });
    connection
        .execute(
            "INSERT OR REPLACE INTO cursorDiskKV (key, value) VALUES \
             ('composerData:seed-existing', ?)",
            [existing.to_string()],
        )
        .expect("cursor composerData 行可写入");
}

/// 迁入 Cursor 的产物必须是「Cursor 真的打得开」的形态。
///
/// 树形验收只证明 Ferry 读得回自己写的东西；`context` 缺席这类问题只有 Cursor 本体
/// 会炸（点开会话报 `Cannot read properties of undefined (reading 'fileSelections')`），
/// 所以这里按真实客户端的读法逐项检查。
fn assert_cursor_artifact(home: &Path, session_id: &str) {
    let connection = rusqlite::Connection::open_with_flags(
        home.join("cursor/state.vscdb"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("cursor 库可只读打开");
    let raw: String = connection
        .query_row(
            "SELECT value FROM cursorDiskKV WHERE key = ?",
            [format!("composerData:{session_id}")],
            |row| row.get(0),
        )
        .expect("composerData 必须落库");
    let data: Value = serde_json::from_str(&raw).expect("composerData 是 JSON");
    let context = data["context"]
        .as_object()
        .expect("context 缺席会让会话在 Cursor 里点不开");
    assert!(
        context.contains_key("fileSelections") && context.contains_key("mentions"),
        "context 形状不完整: {context:?}"
    );
    // 采样只取形状：既有会话的文件选择不能被抄进迁入的会话。
    assert!(!raw.contains("SEEDED.md"), "抄到了既有会话的 context 内容");
    // 空标题在 Cursor 列表里显示成 "New Agent"。
    let title = data["name"].as_str().unwrap_or_default();
    assert!(!title.is_empty(), "迁入的会话没有标题");
    let head: String = connection
        .query_row(
            "SELECT value FROM composerHeaders WHERE composerId = ?",
            [session_id],
            |row| row.get(0),
        )
        .expect("header 行必须落库");
    let head: Value = serde_json::from_str(&head).expect("head 是 JSON");
    assert_eq!(head["name"].as_str().unwrap_or_default(), title);
}

/// `operation.apply` 是异步入队；轮询 `operation.status` 直到终态。
fn wait_for_terminal(engine: &dyn EngineService, plan_id: &Value) -> Value {
    let deadline = Instant::now() + APPLY_TIMEOUT;
    loop {
        let status = engine.operation_status(plan_id).expect("status 可查询");
        match status["status"].as_str() {
            Some("applied" | "failed" | "cancelled" | "expired") => return status,
            _ => assert!(Instant::now() < deadline, "apply 超时: {status}"),
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn migration_input(source: &str, reference: &str, target: &str) -> Value {
    json!({
        "kind": "migration",
        "source_tool": source,
        "ref": reference,
        "target_tool": target,
    })
}

#[test]
fn migration_plans_and_applies_across_three_target_agents() {
    let sandbox = Sandbox::enter();
    seed_claude(sandbox.home(), "case-01-plain");
    seed_claude(sandbox.home(), "case-02-tools");
    seed_codex(sandbox.home(), "case-01-plain");

    let engine = build_engine(None).expect("组合根可装配");
    let records = engine.index().refresh().expect("扫库可完成");

    let mut applied_ids: Vec<String> = Vec::new();
    for (source_tool, target) in COMBOS {
        let record = records
            .iter()
            .find(|record| record.tool == *source_tool)
            .unwrap_or_else(|| panic!("沙箱里必须有 {source_tool} 会话"));
        let reference = record.opaque_ref.clone();
        if *target == "cursor" {
            // 迁入 cursor 要先有这个工作区的记录：迁移用源会话的 cwd 当目标工作区。
            seed_cursor(
                sandbox.home(),
                record
                    .row
                    .get("dir")
                    .and_then(Value::as_str)
                    .expect("扫描行必须带 dir"),
            );
        }
        let plan = engine
            .operation_plan(&migration_input(source_tool, &reference, target))
            .unwrap_or_else(|error| {
                panic!("{source_tool}→{target} plan 失败: {}", error.message())
            });

        assert_eq!(plan["status"], json!("planned"), "target={target}");
        assert_eq!(plan["kind"], json!("migration"));
        assert_eq!(plan["risk"], json!("high"));
        assert_eq!(
            plan["summary"],
            json!(format!("将 {source_tool} 会话迁移到 {target}"))
        );
        assert_eq!(plan["affected_refs"], json!([reference]));

        // `MigrationService::preview` 返回的是 `_prepare` 的 base 再挂一层 `preview`，
        // 所以 schema v3 在 `plan.preview.preview` 上，不是 `plan.preview` 顶层。
        let envelope = &plan["preview"];
        assert_eq!(envelope["src"], json!(*source_tool));
        assert_eq!(envelope["dst"], json!(*target));
        assert!(envelope["loss"].is_object(), "base 必须带 loss 统计");
        let preview = &envelope["preview"];
        assert_eq!(
            preview["schema_version"],
            json!(3),
            "{source_tool}→{target} 的 preview 不是 schema v3: {preview}"
        );
        assert_eq!(preview["target_tool"], json!(*target));

        let plan_id = plan["plan_id"].clone();
        let accepted = engine.operation_apply(&plan_id).expect("apply 可入队");
        assert!(
            matches!(
                accepted["status"].as_str(),
                Some("queued" | "applying" | "applied")
            ),
            "accepted={accepted}"
        );
        let final_state = wait_for_terminal(engine.as_ref(), &plan_id);
        assert_eq!(
            final_state["status"],
            json!("applied"),
            "{source_tool}→{target} apply 未成功: {final_state}"
        );

        let result = &final_state["result"];
        // `validate_written_tree` 的 re-read 验收：结构不过会先回滚再抛。
        assert_eq!(
            result["validation"]["structure"]["ok"],
            json!(true),
            "{source_tool}→{target} 结构验收失败: {result}"
        );
        assert!(result["validation"].get("runtime").is_none());
        assert!(result.get("probe").is_none());
        assert!(result.get("probe_model").is_none());
        assert!(!result["rolled_back"].as_bool().unwrap_or(false));

        let session_id = result["session_id"].as_str().expect("session_id 是字符串");
        assert!(!session_id.is_empty());
        let destination = PathBuf::from(result["dest"].as_str().expect("dest 是字符串"));
        assert!(
            destination.exists(),
            "{source_tool}→{target} 的产物不存在: {}",
            destination.display()
        );
        assert!(
            destination.starts_with(sandbox.home()),
            "产物写到了沙箱外: {}",
            destination.display()
        );
        // resume 描述符由目标 adapter 的 lifecycle 现算（BaseLifecycle 的字段集）。
        assert_eq!(result["resume"]["tool"], json!(*target));
        assert_eq!(result["resume"]["session_id"], json!(session_id));
        assert!(result["resume"]["args"].is_array());
        assert!(result["resume"]["display_command"].is_string());

        if *target == "cursor" {
            assert_cursor_artifact(sandbox.home(), session_id);
        }

        applied_ids.push(session_id.to_string());
    }

    // 每条 apply 落一条历史；`list_all` 倒序返回，条目字段就是 apply 的 result
    // 加上 `time` 与覆盖上去的 `id`（历史行用 `src`/`dst`，不是 `*_tool`）。
    let history = engine.migration_history().expect("历史可读");
    let entries = history.as_array().expect("历史是数组");
    assert_eq!(entries.len(), COMBOS.len(), "历史条数不对: {history}");
    let mut recorded: Vec<&str> = entries
        .iter()
        .map(|entry| entry["dst"].as_str().expect("dst 是字符串"))
        .collect();
    let mut expected: Vec<&str> = COMBOS.iter().map(|(_, target)| *target).collect();
    recorded.sort_unstable();
    expected.sort_unstable();
    assert_eq!(recorded, expected, "历史里的目标 agent 不齐: {history}");
    let mut sources: Vec<&str> = entries
        .iter()
        .map(|entry| entry["src"].as_str().expect("src 是字符串"))
        .collect();
    let mut expected_sources: Vec<&str> = COMBOS.iter().map(|(source, _)| *source).collect();
    sources.sort_unstable();
    expected_sources.sort_unstable();
    assert_eq!(
        sources, expected_sources,
        "历史里的源 agent 不齐: {history}"
    );
    for entry in entries {
        assert!(entry["id"]
            .as_str()
            .is_some_and(|id| id.starts_with("history_")));
        assert!(entry["time"].is_i64() || entry["time"].is_u64());
        assert_eq!(entry["validation"]["structure"]["ok"], json!(true));
        assert!(
            applied_ids
                .iter()
                .any(|id| entry["session_id"] == json!(id.as_str())),
            "历史里的 session_id 与 apply 结果对不上: {entry}"
        );
    }
}

#[test]
fn migration_plan_rejects_an_unknown_target_agent() {
    let sandbox = Sandbox::enter();
    seed_claude(sandbox.home(), "case-01-plain");

    let engine = build_engine(None).expect("组合根可装配");
    let records = engine.index().refresh().expect("扫库可完成");
    let reference = records
        .iter()
        .find(|record| record.tool == "claude")
        .expect("沙箱里必须有 claude 会话")
        .opaque_ref
        .clone();

    let mut input = Map::new();
    input.insert("kind".into(), json!("migration"));
    input.insert("source_tool".into(), json!("claude"));
    input.insert("ref".into(), json!(reference));
    input.insert("target_tool".into(), json!("not-an-agent"));
    let error = engine
        .operation_plan(&Value::Object(input))
        .expect_err("未知目标必须被拒");
    assert_eq!(error.error_type(), "AgentRequestError");
    // 被拒的计划一行都不能落库。
    assert_eq!(
        engine
            .migration_history()
            .expect("历史可读")
            .as_array()
            .map(Vec::len),
        Some(0)
    );
}
