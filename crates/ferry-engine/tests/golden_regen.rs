//! 黄金基线的再生入口：`tests/golden/**` 由本测试产出，也由本测试校验。
//!
//! ```bash
//! cargo test -p ferry-engine --test golden_regen                      # 只校验
//! FERRY_GOLDEN_REGEN=1 cargo test -p ferry-engine --test golden_regen # 覆盖写入
//! ```
//!
//! 各 agent 的 `*_golden.rs` 只对照**自己那一家**，且会按
//! `_normalized.environment_dependent_fields` 抹平字段；本测试是唯一一处
//! 「整套基线逐字节可再生」的入口：同一份 fixture 物化到临时沙箱，跑真实的
//! reader / scanner，按固定序列化参数渲染，与磁盘上的文件逐字节比较。
//!
//! 序列化参数是基线格式的一部分：键按字典序、非 ASCII 不转义、缩进 2 空格、
//! 末尾一个换行。`serde_json` 开了 `preserve_order`，因此排序必须显式做。
//!
//! 幂等：连续再生两次 `git diff tests/golden/` 必须为空。所有物化产物的 mtime
//! 统一钉在 [`FIXED_MTIME`]，扫描行里的 `updated` 才只取决于 fixture 内容。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use ferry_engine::adapters::contracts::{ScanCache, ScanRow};
use ferry_engine::adapters::{claude, codex, grok, opencode, pi};
use ferry_engine::jsonutil::FileStat;
use ferry_engine::model::Session;
use serde_json::{Map, Value};

/// 物化产物统一的 mtime（2026-07-25T00:00:00Z）。
const FIXED_MTIME: u64 = 1_784_937_600;
const SANDBOX_MARKER: &str = "<home>";
const AGENTS: [&str; 5] = ["claude", "codex", "opencode", "pi", "grok"];

/// 扫描行中由运行环境（而非 fixture 内容）决定的字段。
fn environment_dependent(agent: &str) -> &'static [&'static str] {
    match agent {
        // opencode 扫描行不带文件路径（path 恒为 ""、size 恒为 0），
        // updated/created 来自 SQLite 列。
        "opencode" => &["updated", "own_updated"],
        _ => &["path", "updated", "own_updated", "size", "own_size"],
    }
}

const NOTE: &str = "path 中的沙箱根已替换为 <home>；updated/own_updated 之所以稳定，是因为物化 fixture 时把 mtime 统一设成 fixed_mtime_seconds。Rust 侧对照真实环境时，这些字段应按各自环境重新计算。";

/// codex `state_5.sqlite` 的当前结构。
///
/// `IF NOT EXISTS`：每个 case 前都会 reset，库总是空的；保留幂等写法，让同一个
/// 沙箱被复用时不会在第二个 case 上炸掉。
const CODEX_STATE_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS threads (
    id TEXT PRIMARY KEY, rollout_path TEXT NOT NULL,
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
    source TEXT NOT NULL, model_provider TEXT NOT NULL, cwd TEXT NOT NULL,
    title TEXT NOT NULL, sandbox_policy TEXT NOT NULL,
    approval_mode TEXT NOT NULL, tokens_used INTEGER NOT NULL DEFAULT 0,
    has_user_event INTEGER NOT NULL DEFAULT 0, archived INTEGER NOT NULL DEFAULT 0,
    cli_version TEXT NOT NULL DEFAULT '', first_user_message TEXT NOT NULL DEFAULT '',
    agent_path TEXT, thread_source TEXT, preview TEXT NOT NULL DEFAULT '',
    recency_at INTEGER NOT NULL DEFAULT 0, history_mode TEXT NOT NULL DEFAULT 'legacy'
);
CREATE TABLE IF NOT EXISTS thread_spawn_edges (
    parent_thread_id TEXT NOT NULL, child_thread_id TEXT NOT NULL PRIMARY KEY,
    status TEXT NOT NULL
);
";

// ---------------------------------------------------------------------------
// 沙箱
// ---------------------------------------------------------------------------

/// 一次运行共用的假 HOME；每个 case 前把对应 agent 的存储清空重建。
struct Sandbox {
    home: PathBuf,
    _root: tempfile::TempDir,
}

impl Sandbox {
    fn enter() -> Self {
        let root = tempfile::tempdir().expect("临时 HOME 可创建");
        let home = fs::canonicalize(root.path()).expect("临时 HOME 可规范化");
        let sandbox = Self { home, _root: root };
        sandbox.apply_env();
        sandbox
    }

    fn opencode_db(&self) -> PathBuf {
        self.home.join("opencode").join("storage.db")
    }

    /// 读取路径的模块全部按环境实时解析，因此这里改一次环境就够了。
    fn apply_env(&self) {
        let home = &self.home;
        let entries = [
            ("HOME", home.clone()),
            ("USERPROFILE", home.clone()),
            ("XDG_DATA_HOME", home.join(".local").join("share")),
            ("XDG_CONFIG_HOME", home.join(".config")),
            ("FERRY_DATA_DIR", home.join(".ferry")),
            ("FERRY_BACKUP_DIR", home.join(".ferry").join("backups")),
            ("FERRY_OPENCODE_DB", self.opencode_db()),
            ("GROK_HOME", home.join(".grok")),
            ("PI_CODING_AGENT_SESSION_DIR", home.join("pi-sessions")),
        ];
        // SAFETY: 整个测试二进制只有一个 #[test]，没有并发改写环境的线程。
        unsafe {
            for (key, value) in entries {
                std::env::set_var(key, value);
            }
            // Pi 的 settings 探测与 codex 的 registry 都可能读到用户真实目录。
            std::env::remove_var("PI_CODING_AGENT_DIR");
            std::env::remove_var("CODEX_HOME");
        }
        fs::create_dir_all(self.opencode_db().parent().expect("库有父目录"))
            .expect("opencode 目录可创建");
        fs::create_dir_all(home.join(".ferry")).expect("数据目录可创建");
    }

    fn store_root(&self, agent: &str) -> PathBuf {
        match agent {
            "claude" => self.home.join(".claude"),
            "codex" => self.home.join(".codex"),
            "opencode" => self
                .opencode_db()
                .parent()
                .expect("库有父目录")
                .to_path_buf(),
            "pi" => self.home.join("pi-sessions"),
            "grok" => self.home.join(".grok"),
            other => panic!("未知 agent: {other}"),
        }
    }

    fn reset(&self, agent: &str) {
        let root = self.store_root(agent);
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("存储根可重建");
    }

    fn home_text(&self) -> String {
        self.home.to_string_lossy().into_owned()
    }
}

// ---------------------------------------------------------------------------
// 通用助手
// ---------------------------------------------------------------------------

/// 扫描期不缓存：每次都真解析，保证再生的是解析结果本身。
struct NullCache;

impl ScanCache for NullCache {
    fn get(&self, _path: &Path, _stat: &FileStat) -> Option<Option<ScanRow>> {
        None
    }
    fn put(&self, _path: &Path, _stat: &FileStat, _meta: Option<ScanRow>) {}
    fn get_digest(&self, _path: &Path, _stat: &FileStat) -> Option<String> {
        None
    }
    fn put_digest(&self, _path: &Path, _stat: &FileStat, _digest: &str) {}
    fn flush(&self) {}
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("仓库根可解析")
}

fn read_json(path: &Path) -> Value {
    let text = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("读取 {} 失败: {error}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("{} 不是合法 JSON: {error}", path.display()))
}

fn read_jsonl(path: &Path) -> Vec<Value> {
    let text = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("读取 {} 失败: {error}", path.display()));
    text.split('\n')
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("JSONL 行是合法 JSON"))
        .collect()
}

/// 把 mtime 钉死；目录则递归钉死其下全部成员。
fn freeze(path: &Path) {
    let pinned = UNIX_EPOCH + Duration::from_secs(FIXED_MTIME);
    if path.is_dir() {
        let mut members: Vec<PathBuf> = fs::read_dir(path)
            .expect("目录可读")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect();
        members.sort();
        for member in members {
            freeze(&member);
        }
    }
    let times = fs::FileTimes::new()
        .set_accessed(pinned)
        .set_modified(pinned);
    let handle = if path.is_dir() {
        fs::File::open(path)
    } else {
        fs::File::options().write(true).open(path)
    };
    if let Ok(handle) = handle {
        let _ = handle.set_times(times);
    }
}

fn copy_frozen(source: &Path, target: &Path) {
    fs::create_dir_all(target.parent().expect("目标有父目录")).expect("目标目录可创建");
    let fixture = fs::read_to_string(source)
        .unwrap_or_else(|error| panic!("读取 {} 失败: {error}", source.display()))
        .replace("\r\n", "\n");
    fs::write(target, fixture)
        .unwrap_or_else(|error| panic!("物化 {} 失败: {error}", source.display()));
    freeze(target);
}

/// 递归把沙箱绝对路径换成稳定字面量。
fn normalize(value: &Value, home: &str) -> Value {
    match value {
        Value::String(text) => {
            let normalized = text.replace(home, SANDBOX_MARKER);
            Value::String(if normalized.contains(SANDBOX_MARKER) {
                normalized.replace('\\', "/")
            } else {
                normalized
            })
        }
        Value::Array(items) => {
            Value::Array(items.iter().map(|item| normalize(item, home)).collect())
        }
        Value::Object(entries) => Value::Object(
            entries
                .iter()
                .map(|(key, item)| (key.clone(), normalize(item, home)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// 递归按键排序（`preserve_order` 下 `Map` 保插入序，排序必须显式做）。
fn sort_keys(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(sort_keys).collect()),
        Value::Object(entries) => {
            let mut keys: Vec<&String> = entries.keys().collect();
            keys.sort();
            let mut sorted = Map::new();
            for key in keys {
                sorted.insert(key.clone(), sort_keys(&entries[key]));
            }
            Value::Object(sorted)
        }
        other => other.clone(),
    }
}

/// 基线文件的渲染参数：sort_keys + 不转义非 ASCII + indent 2 + 末尾换行。
fn render(value: &Value) -> String {
    let mut text = serde_json::to_string_pretty(&sort_keys(value)).expect("payload 可序列化");
    text.push('\n');
    text
}

/// fixture 的 manifest 记录的原生文件名，让物化布局贴近真实 capture。
fn native_stem(case_dir: &Path, fallback: &str) -> String {
    let manifest_path = case_dir.join("manifest.json");
    if !manifest_path.exists() {
        return fallback.to_string();
    }
    let manifest = read_json(&manifest_path);
    if let Some(source) = manifest
        .get("source_paths")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(Value::as_str)
    {
        return Path::new(source)
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| fallback.to_string());
    }
    manifest
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| fallback.to_string())
}

// ---------------------------------------------------------------------------
// 各家的物化 + 读取
// ---------------------------------------------------------------------------

/// 物化一个 case，返回 `(canonical Session, 扫描行)`。
fn dump_case(sandbox: &Sandbox, agent: &str, case_dir: &Path) -> (Session, Vec<ScanRow>) {
    sandbox.reset(agent);
    let case = case_dir
        .file_name()
        .expect("case 有目录名")
        .to_string_lossy();
    match agent {
        "claude" => {
            let target = sandbox
                .store_root("claude")
                .join("projects")
                .join(case.as_ref())
                .join(format!("{}.jsonl", native_stem(case_dir, case.as_ref())));
            copy_frozen(&case_dir.join("session.jsonl"), &target);
            let session = claude::reader::read(&target.to_string_lossy()).expect("claude 可读");
            (
                session,
                claude::scanner::scan(&NullCache).expect("claude 可扫描"),
            )
        }
        "codex" => {
            let home = sandbox.store_root("codex");
            let stem = native_stem(case_dir, &format!("rollout-{case}"));
            let target = home
                .join("sessions")
                .join("2026")
                .join("07")
                .join("25")
                .join(format!("{stem}.jsonl"));
            copy_frozen(&case_dir.join("session.jsonl"), &target);
            write_codex_registry(&home.join("state_5.sqlite"), case_dir, &target);
            codex::reader::clear_cache();
            let session = codex::reader::read(&target.to_string_lossy(), None).expect("codex 可读");
            let rows = codex::scanner::scan(&NullCache).expect("codex 可扫描");
            codex::reader::clear_cache();
            (session, rows)
        }
        "opencode" => {
            let fixture = read_json(&case_dir.join("session.json"));
            write_opencode_database(&sandbox.opencode_db(), &fixture);
            let session_id = fixture["session"]["id"]
                .as_str()
                .expect("fixture 有会话 id");
            let session = opencode::reader::read(session_id).expect("opencode 可读");
            (
                session,
                opencode::scanner::scan(&NullCache).expect("opencode 可扫描"),
            )
        }
        "pi" => {
            // Pi fixture 无 manifest，会话 id 只能从 v3 头部记录里取。
            let records = read_jsonl(&case_dir.join("session.jsonl"));
            let stem = records
                .first()
                .and_then(|header| header.get("id"))
                .and_then(Value::as_str)
                .unwrap_or(case.as_ref())
                .to_string();
            let target = sandbox
                .store_root("pi")
                .join(case.as_ref())
                .join(format!("{stem}.jsonl"));
            copy_frozen(&case_dir.join("session.jsonl"), &target);
            let session = pi::reader::read(&target.to_string_lossy()).expect("pi 可读");
            (session, pi::scanner::scan(&NullCache).expect("pi 可扫描"))
        }
        "grok" => {
            let summary = read_json(&case_dir.join("summary.json"));
            let bundle_id = summary
                .get("info")
                .and_then(|info| info.get("id"))
                .and_then(Value::as_str)
                .unwrap_or(case.as_ref())
                .to_string();
            let target = sandbox
                .store_root("grok")
                .join("sessions")
                .join(case.as_ref())
                .join(&bundle_id);
            fs::create_dir_all(&target).expect("bundle 目录可创建");
            let mut members: Vec<PathBuf> = fs::read_dir(case_dir)
                .expect("fixture 目录可读")
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.is_file())
                .collect();
            members.sort();
            for member in members {
                copy_frozen(
                    &member,
                    &target.join(member.file_name().expect("成员有文件名")),
                );
            }
            freeze(&target);
            let session = grok::reader::read(&target).expect("grok 可读");
            (
                session,
                grok::scanner::scan(&NullCache).expect("grok 可扫描"),
            )
        }
        other => panic!("未知 agent: {other}"),
    }
}

/// 按 fixture 的 `registration.json` 合成 codex 会话注册库。
///
/// 引擎只从这里读 `thread_spawn_edges`（父子边）与 `threads`（closure 指纹），
/// fixture 未提供边，所以表建出来但为空；`rollout_path` 重写成物化后的真实路径。
fn write_codex_registry(db_path: &Path, case_dir: &Path, rollout: &Path) {
    let registration = case_dir.join("registration.json");
    if !registration.exists() {
        return;
    }
    let threads = read_json(&registration)
        .get("threads")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let db = rusqlite::Connection::open(db_path).expect("注册库可创建");
    db.execute_batch(CODEX_STATE_SCHEMA).expect("注册库建表");
    for thread in threads {
        let field = |key: &str| {
            thread
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        db.execute(
            "INSERT OR REPLACE INTO threads (id, rollout_path, created_at, updated_at, source,\
             model_provider, cwd, title, sandbox_policy, approval_mode, cli_version,\
             first_user_message) VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
            rusqlite::params![
                field("id"),
                rollout.to_string_lossy().into_owned(),
                FIXED_MTIME as i64,
                FIXED_MTIME as i64,
                "cli",
                "openai",
                field("cwd"),
                field("title"),
                "workspace-write",
                "on-request",
                field("cli_version"),
                field("first_user_message"),
            ],
        )
        .expect("注册行可写");
    }
    drop(db);
    freeze(db_path);
}

/// fixture 的 `session.json` 就是三张表的导出行，按当前列集合还原成 SQLite 库。
fn write_opencode_database(db_path: &Path, fixture: &Value) {
    let _ = fs::remove_file(db_path);
    fs::create_dir_all(db_path.parent().expect("库有父目录")).expect("库目录可创建");
    let session_columns: Vec<&str> = opencode::store::CURRENT_DB_COLUMNS[0].1.to_vec();
    let columns = session_columns
        .iter()
        .map(|name| format!("\"{name}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let db = rusqlite::Connection::open(db_path).expect("会话库可创建");
    db.execute_batch(&format!(
        "CREATE TABLE session ({columns});\
         CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT, data TEXT, \
         time_created INTEGER);\
         CREATE TABLE part (id TEXT PRIMARY KEY, message_id TEXT, session_id TEXT, \
         data TEXT, time_created INTEGER);"
    ))
    .expect("会话库建表");

    let session = &fixture["session"];
    let placeholders = vec!["?"; session_columns.len()].join(",");
    let values: Vec<rusqlite::types::Value> = session_columns
        .iter()
        .map(|name| sqlite_value(session.get(*name)))
        .collect();
    db.execute(
        &format!("INSERT INTO session ({columns}) VALUES ({placeholders})"),
        rusqlite::params_from_iter(values),
    )
    .expect("会话行可写");

    // time_created 用 fixture 里的下标，保证 store 的 ORDER BY 复现原顺序。
    for (index, row) in fixture["messages"]
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .enumerate()
    {
        db.execute(
            "INSERT INTO message (id, session_id, data, time_created) VALUES (?,?,?,?)",
            rusqlite::params![
                row["id"].as_str().unwrap_or_default(),
                row["session_id"].as_str().unwrap_or_default(),
                row["data"].as_str().unwrap_or_default(),
                index as i64,
            ],
        )
        .expect("消息行可写");
    }
    for (index, row) in fixture["parts"]
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .enumerate()
    {
        db.execute(
            "INSERT INTO part (id, message_id, session_id, data, time_created) VALUES (?,?,?,?,?)",
            rusqlite::params![
                row["id"].as_str().unwrap_or_default(),
                row["message_id"].as_str().unwrap_or_default(),
                row["session_id"].as_str().unwrap_or_default(),
                row["data"].as_str().unwrap_or_default(),
                index as i64,
            ],
        )
        .expect("part 行可写");
    }
    drop(db);
    freeze(db_path);
}

fn sqlite_value(value: Option<&Value>) -> rusqlite::types::Value {
    use rusqlite::types::Value as Sql;
    match value {
        None | Some(Value::Null) => Sql::Null,
        Some(Value::Bool(flag)) => Sql::Integer(i64::from(*flag)),
        Some(Value::Number(number)) => number
            .as_i64()
            .map(Sql::Integer)
            .or_else(|| number.as_f64().map(Sql::Real))
            .unwrap_or(Sql::Null),
        Some(Value::String(text)) => Sql::Text(text.clone()),
        Some(other) => Sql::Text(other.to_string()),
    }
}

// ---------------------------------------------------------------------------
// 主流程
// ---------------------------------------------------------------------------

fn cases(agent: &str) -> Vec<PathBuf> {
    let root = repository_root()
        .join("tests/fixtures/agent_formats")
        .join(agent);
    let mut dirs: Vec<PathBuf> = fs::read_dir(&root)
        .unwrap_or_else(|error| panic!("{} 不可读: {error}", root.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();
    dirs
}

#[test]
fn regenerate_or_verify_golden_baseline() {
    let regenerate = std::env::var("FERRY_GOLDEN_REGEN")
        .map(|value| !value.is_empty() && value != "0")
        .unwrap_or(false);

    let sandbox = Sandbox::enter();
    // 方言与损耗目录的登记是 `build()` 的副作用，读取前必须先装配。
    ferry_engine::adapters::registry::create_registry().expect("内置 adapter 可装配");

    let golden_root = repository_root().join("tests/golden");
    let mut stale: Vec<String> = Vec::new();
    let mut total = 0usize;

    for agent in AGENTS {
        for case_dir in cases(agent) {
            let case = case_dir
                .file_name()
                .expect("case 有目录名")
                .to_string_lossy()
                .into_owned();
            let (session, rows) = dump_case(&sandbox, agent, &case_dir);
            let home = sandbox.home_text();

            let canonical = normalize(
                &serde_json::to_value(&session).expect("Session 可序列化"),
                &home,
            );
            let scan_rows = normalize(
                &Value::Array(rows.into_iter().map(Value::Object).collect()),
                &home,
            );
            let mut normalized = Map::new();
            normalized.insert("sandbox_root_marker".into(), Value::from(SANDBOX_MARKER));
            normalized.insert("fixed_mtime_seconds".into(), Value::from(FIXED_MTIME));
            normalized.insert(
                "environment_dependent_fields".into(),
                Value::Array(
                    environment_dependent(agent)
                        .iter()
                        .map(|field| Value::from(*field))
                        .collect(),
                ),
            );
            normalized.insert("note".into(), Value::from(NOTE));
            let mut scan = Map::new();
            scan.insert("_normalized".into(), Value::Object(normalized));
            scan.insert("rows".into(), scan_rows);

            for (kind, payload) in [("canonical", canonical), ("scan", Value::Object(scan))] {
                let path = golden_root
                    .join(kind)
                    .join(agent)
                    .join(format!("{case}.json"));
                let text = render(&payload);
                total += 1;
                if regenerate {
                    fs::create_dir_all(path.parent().expect("有父目录")).expect("基线目录可创建");
                    fs::write(&path, &text).expect("基线可写");
                    continue;
                }
                let current = fs::read_to_string(&path)
                    .ok()
                    .map(|value| value.replace("\r\n", "\n"));
                if current.as_deref() != Some(text.as_str()) {
                    stale.push(format!("{kind}/{agent}/{case}.json"));
                }
            }
        }
    }

    assert_eq!(total, 26, "黄金基线共 13 个 case × 2 个产物");
    assert!(
        stale.is_empty(),
        "这些黄金文件与当前引擎产出不一致（确认是有意变更后跑 \
         FERRY_GOLDEN_REGEN=1 cargo test -p ferry-engine --test golden_regen 再生）:\n{}",
        stale.join("\n")
    );
}
