//! Codex 适配器的黄金对照（WP-C2）。
//!
//! 用 `tests/fixtures/agent_formats/codex/**` 在临时 HOME 里物化出真实的
//! `~/.codex` 布局（含由 `registration.json` 合成的 `state_5.sqlite`），跑 Rust 侧
//! reader / scanner，再与 `tests/golden/{canonical,scan}/codex/*.json` 逐字段比对。
//!
//! 物化方式与 `scripts/dump-canonical-fixtures.py` 一致：
//! - rollout 落到 `<home>/.codex/sessions/2026/07/25/<manifest stem>.jsonl`；
//! - 所有物化产物的 mtime 钉死在 `FIXED_MTIME`，扫描行里的 `updated` 才稳定；
//! - 输出里的沙箱绝对路径替换成字面量 `<home>`，与黄金文件同形。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, UNIX_EPOCH};

use ferry_engine::adapters::codex::{reader, scanner, tool_calls, tool_results};
use ferry_engine::adapters::contracts::{ScanCache, ScanRow};
use ferry_engine::jsonutil::FileStat;
use serde_json::{Map, Value};

/// 与 `scripts/dump-canonical-fixtures.py` 的 `FIXED_MTIME` 一致。
const FIXED_MTIME: u64 = 1_784_937_600;
const SANDBOX_MARKER: &str = "<home>";

/// codex `state_5.sqlite` 的当前结构（与 dump 脚本的 `CODEX_STATE_SCHEMA` 一致）。
const CODEX_STATE_SCHEMA: &str = "
CREATE TABLE threads (
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
CREATE TABLE thread_spawn_edges (
    parent_thread_id TEXT NOT NULL, child_thread_id TEXT NOT NULL PRIMARY KEY,
    status TEXT NOT NULL
);
";

/// HOME 是进程级状态，黄金对照必须串行跑。
static HOME_LOCK: Mutex<()> = Mutex::new(());

/// 黄金 dump 不需要跨 case 复用缓存，这里给一个永远 miss 的端口。
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

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/ferry-engine 的上两级是仓库根")
        .to_path_buf()
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).expect("黄金文件可读")).expect("黄金文件是 JSON")
}

/// 把物化产物的 mtime 钉死，保证扫描行里的 `updated` 稳定。
fn freeze(path: &Path) {
    let handle = fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("物化文件可写");
    handle
        .set_modified(UNIX_EPOCH + Duration::from_secs(FIXED_MTIME))
        .expect("mtime 可设置");
}

/// manifest 里记录的原生文件名（贴近真实 capture 的布局）。
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

/// 按 fixture 的 `registration.json` 合成 codex 会话注册库。
fn write_registry(db_path: &Path, case_dir: &Path, rollout: &Path) {
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
        let get = |key: &str| {
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
                get("id"),
                rollout.to_string_lossy().into_owned(),
                FIXED_MTIME as i64,
                FIXED_MTIME as i64,
                "cli",
                "openai",
                get("cwd"),
                get("title"),
                "workspace-write",
                "on-request",
                get("cli_version"),
                get("first_user_message"),
            ],
        )
        .expect("注册行可写");
    }
    drop(db);
    freeze(db_path);
}

/// 在临时 HOME 里物化一个 case，返回 rollout 路径。
fn materialize(home: &Path, case_dir: &Path) -> PathBuf {
    let stem = native_stem(
        case_dir,
        &format!(
            "rollout-{}",
            case_dir.file_name().unwrap().to_string_lossy()
        ),
    );
    let codex_home = home.join(".codex");
    let target = codex_home
        .join("sessions")
        .join("2026")
        .join("07")
        .join("25")
        .join(format!("{stem}.jsonl"));
    fs::create_dir_all(target.parent().expect("有父目录")).expect("会话目录可创建");
    fs::copy(case_dir.join("session.jsonl"), &target).expect("fixture 可拷贝");
    freeze(&target);
    write_registry(&codex_home.join("state_5.sqlite"), case_dir, &target);
    target
}

/// 把输出里残留的沙箱绝对路径换成稳定字面量。
fn normalize(value: &Value, home: &str) -> Value {
    match value {
        Value::String(text) => Value::String(text.replace(home, SANDBOX_MARKER)),
        Value::Array(items) => {
            Value::Array(items.iter().map(|item| normalize(item, home)).collect())
        }
        Value::Object(entries) => Value::Object(
            entries
                .iter()
                .map(|(key, item)| (key.clone(), normalize(item, home)))
                .collect::<Map<String, Value>>(),
        ),
        other => other.clone(),
    }
}

/// 逐字段 diff，给出比 `assert_eq!` 更可读的失败信息。
fn diff(path: &str, actual: &Value, expected: &Value, out: &mut Vec<String>) {
    match (actual, expected) {
        (Value::Object(actual_entries), Value::Object(expected_entries)) => {
            let mut keys: Vec<&String> = actual_entries.keys().collect();
            for key in expected_entries.keys() {
                if !actual_entries.contains_key(key) {
                    keys.push(key);
                }
            }
            keys.sort();
            keys.dedup();
            for key in keys {
                diff(
                    &format!("{path}.{key}"),
                    actual_entries.get(key).unwrap_or(&Value::Null),
                    expected_entries.get(key).unwrap_or(&Value::Null),
                    out,
                );
            }
        }
        (Value::Array(actual_items), Value::Array(expected_items)) => {
            if actual_items.len() != expected_items.len() {
                out.push(format!(
                    "{path}: 长度不同 actual={} expected={}",
                    actual_items.len(),
                    expected_items.len()
                ));
                return;
            }
            for (index, (actual_item, expected_item)) in
                actual_items.iter().zip(expected_items).enumerate()
            {
                diff(&format!("{path}[{index}]"), actual_item, expected_item, out);
            }
        }
        _ if actual == expected => {}
        _ => out.push(format!("{path}: actual={actual} expected={expected}")),
    }
}

fn assert_same(label: &str, actual: &Value, expected: &Value) {
    let mut differences = Vec::new();
    diff(label, actual, expected, &mut differences);
    assert!(
        differences.is_empty(),
        "{label} 与黄金基线不一致:\n{}",
        differences.join("\n")
    );
}

/// 两个 case 共用一份进程级 HOME，必须在同一个测试里串行跑完。
#[test]
fn codex_reader_and_scanner_match_the_golden_baseline() {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let root = repo_root();
    let fixtures = root.join("tests/fixtures/agent_formats/codex");
    let golden = root.join("tests/golden");
    let mut cases: Vec<PathBuf> = fs::read_dir(&fixtures)
        .expect("codex fixture 目录存在")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    cases.sort();
    assert_eq!(cases.len(), 2, "codex 共 2 个 fixture case");

    let previous_home = std::env::var("HOME").ok();
    let previous_profile = std::env::var("USERPROFILE").ok();

    for case_dir in cases {
        let case = case_dir.file_name().unwrap().to_string_lossy().into_owned();
        // 每个 case 单独物化、单独扫描，扫描行天然只含该 case 自己的会话树。
        let sandbox = tempfile::tempdir().expect("临时 HOME 可创建");
        let home = fs::canonicalize(sandbox.path()).expect("临时 HOME 可规范化");
        // SAFETY: 本测试持有 HOME_LOCK，进程内不会有别的线程同时改写这两个变量。
        unsafe {
            std::env::set_var("HOME", &home);
            std::env::set_var("USERPROFILE", &home);
        }
        reader::clear_cache();

        let rollout = materialize(&home, &case_dir);
        let home_text = home.to_string_lossy().into_owned();

        let session = reader::read(&rollout.to_string_lossy(), None)
            .unwrap_or_else(|error| panic!("{case} 读取失败: {error:?}"));
        let canonical = normalize(
            &serde_json::to_value(&session).expect("Session 可序列化"),
            &home_text,
        );
        assert_same(
            &format!("canonical/codex/{case}"),
            &canonical,
            &read_json(&golden.join(format!("canonical/codex/{case}.json"))),
        );

        let rows =
            scanner::scan(&NullCache).unwrap_or_else(|error| panic!("{case} 扫描失败: {error:?}"));
        let actual_rows = normalize(
            &Value::Array(rows.into_iter().map(Value::Object).collect()),
            &home_text,
        );
        let expected = read_json(&golden.join(format!("scan/codex/{case}.json")));
        assert_same(
            &format!("scan/codex/{case}"),
            &actual_rows,
            expected.get("rows").expect("黄金 scan 有 rows"),
        );

        // 闭包指纹：同一份存储重复计算必须稳定，且带上 registry 修订。
        let fingerprint = scanner::fingerprint(&rollout.to_string_lossy())
            .unwrap_or_else(|error| panic!("{case} 指纹失败: {error:?}"));
        assert!(fingerprint.starts_with("sha256:"), "{case}: {fingerprint}");
        assert!(
            fingerprint.contains(":sha256:"),
            "{case} 缺少 registry 指纹"
        );
        assert_eq!(
            fingerprint,
            scanner::fingerprint(&rollout.to_string_lossy()).unwrap()
        );

        reader::clear_cache();
    }

    // SAFETY: 同上，仍持有 HOME_LOCK。
    unsafe {
        match previous_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        match previous_profile {
            Some(value) => std::env::set_var("USERPROFILE", value),
            None => std::env::remove_var("USERPROFILE"),
        }
    }
}

/// JS 词法扫描器与 `parse_custom_call` 的 Python 对照表。
///
/// `tests/data/codex_js_oracle.json` 由 Python 引擎直接产出（逐条调用
/// `engine.adapters.codex.tool_calls._scan_tool_invocations` 与
/// `parse_custom_call`），覆盖字符串/注释遮蔽、嵌套括号、未闭合括号与注释、
/// 反引号与单引号字面量、标识符前缀、`apply_patch` 的四条候选取值路径、
/// `exec_command` 的列表命令与非对象实参、以及非 ASCII 源码（字符索引）。
#[test]
fn codex_js_scanner_matches_the_python_oracle() {
    let oracle =
        read_json(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/codex_js_oracle.json"));
    let cases = oracle.as_array().expect("对照表是数组");
    assert!(cases.len() >= 20, "对照表至少覆盖 20 个形态");

    for case in cases {
        let name = case["name"].as_str().expect("用例有名字");
        let source = case["source"].as_str().expect("用例有源码");

        let calls = tool_calls::scan_tool_invocations(source);
        let actual_calls = Value::Array(
            calls
                .iter()
                .map(|(tool, argument)| {
                    Value::Array(vec![
                        Value::from(tool.as_str()),
                        Value::from(argument.as_str()),
                    ])
                })
                .collect(),
        );
        assert_same(&format!("js/{name}/calls"), &actual_calls, &case["calls"]);

        let mut payload = Map::new();
        payload.insert("name".into(), case["native_name"].clone());
        payload.insert("input".into(), Value::from(source));
        let mut session = ferry_engine::model::Session::new("codex", "s", "");
        let call = tool_calls::parse_custom_call(&payload, &mut session);

        assert_same(
            &format!("js/{name}/tool_name"),
            &Value::from(call.name.as_str()),
            &case["tool_name"],
        );
        assert_same(
            &format!("js/{name}/op"),
            &call.op.as_deref().map_or(Value::Null, Value::from),
            &case["op"],
        );
        assert_same(&format!("js/{name}/input"), &call.input, &case["input"]);
        let losses = Value::Array(
            session
                .loss
                .iter()
                .map(|event| Value::from(event.code.as_str()))
                .collect(),
        );
        assert_same(&format!("js/{name}/loss"), &losses, &case["loss"]);
    }
}

/// 工具结果包络解析的 Python 对照表。
///
/// `tests/data/codex_result_oracle.json` 由 `engine.adapters.codex.tool_results
/// .parse_result` 逐条产出，覆盖：内嵌 JSON 信封判定、`stdout` 键优先于 `output`、
/// 显式/未知 status、exit_code 推导链、stderr 兜底、`Script completed` 包装块的
/// 剔除条件、unified-exec 头部恢复 exit_code 与 running、图片/文件/未知块透传。
#[test]
fn codex_tool_results_match_the_python_oracle() {
    let oracle = read_json(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/codex_result_oracle.json"),
    );
    let cases = oracle.as_array().expect("对照表是数组");
    assert!(cases.len() >= 18, "对照表至少覆盖 18 个形态");
    for case in cases {
        let name = case["name"].as_str().expect("用例有名字");
        let result = tool_results::parse_result(&case["raw"]);
        let actual = serde_json::to_value(&result).expect("ToolResult 可序列化");
        assert_same(&format!("result/{name}"), &actual, &case["result"]);
    }
}

/// `state_5.sqlite` 闭包指纹（`_registry_revision`）的 Python 对照。
///
/// `tests/data/codex_registry_oracle.json` 记录了建表/插入语句与 Python 侧算出的
/// sha256；Rust 侧用同一份 DDL 重建库再逐个 id 集合比对。这条断言同时锁住三件事：
/// `PRAGMA table_xinfo` 的可见列过滤、`sorted(rows, key=repr)` 的 Python 元组
/// repr 排序，以及 `json.dumps(sort_keys=True, ensure_ascii=False)` 的字节形态。
#[test]
fn codex_registry_revision_matches_the_python_oracle() {
    let oracle = read_json(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/codex_registry_oracle.json"),
    );
    let temp = tempfile::tempdir().expect("临时目录可创建");
    let db_path = temp.path().join("state_5.sqlite");
    let db = rusqlite::Connection::open(&db_path).expect("注册库可创建");
    db.execute_batch(oracle["schema"].as_str().expect("有 DDL"))
        .expect("建表");
    db.execute_batch(oracle["rows"].as_str().expect("有插入语句"))
        .expect("插入");
    drop(db);

    for case in oracle["cases"].as_array().expect("对照表是数组") {
        let ids: std::collections::HashSet<String> = case["ids"]
            .as_array()
            .expect("ids 是数组")
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        let actual = ferry_engine::adapters::codex::native::registry_revision(Some(&db_path), &ids)
            .expect("指纹计算不报错");
        assert_same(
            &format!("registry/{:?}", case["ids"]),
            &actual.map_or(Value::Null, Value::from),
            &case["revision"],
        );
    }
}
