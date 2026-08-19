//! Grok 适配器的黄金对照（WP-C5）。
//!
//! 把 `tests/fixtures/agent_formats/grok/**` 物化到临时 `GROK_HOME`（mtime 统一
//! 钉到 `FIXED_MTIME`），跑 Rust 侧 scanner / reader，再与 Python 引擎产出的
//! `tests/golden/{canonical,scan}/grok/*.json` 逐字段比对。
//!
//! 对照口径与 `tests/golden/README.md` 一致：
//! - canonical：整棵 `Session` 逐字段相等（值为 null 的字段也必须出现）；
//! - scan：`_normalized.environment_dependent_fields` 里的 `path` 按沙箱根抹平
//!   成 `<home>`，其余字段（含 `updated` / `size`，它们由 fixture 内容决定）
//!   逐字段相等。

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ferry_engine::adapters::grok::{reader, scanner};
use serde_json::Value;

/// 与 `scripts/dump-canonical-fixtures.py` 的 `FIXED_MTIME` 一致。
const FIXED_MTIME: i64 = 1_784_937_600;
const SANDBOX_MARKER: &str = "<home>";
const CASES: [&str; 4] = [
    "case-01-plain",
    "case-02-tools",
    "case-03-rewind",
    "case-04-chat-fallback",
];

/// `GROK_HOME` 是进程级状态；同一个测试二进制里的用例必须串行。
static ENVIRONMENT: Mutex<()> = Mutex::new(());

struct HomeGuard(Option<String>);

impl Drop for HomeGuard {
    fn drop(&mut self) {
        // SAFETY: 全部改 GROK_HOME 的用例都持有 ENVIRONMENT 锁，无并发写。
        unsafe {
            match self.0.take() {
                Some(previous) => std::env::set_var("GROK_HOME", previous),
                None => std::env::remove_var("GROK_HOME"),
            }
        }
    }
}

fn set_grok_home(path: &Path) -> HomeGuard {
    let previous = std::env::var("GROK_HOME").ok();
    // SAFETY: 见 HomeGuard。
    unsafe { std::env::set_var("GROK_HOME", path) };
    HomeGuard(previous)
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("仓库根可解析")
}

fn read_json(path: &Path) -> Value {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("读取 {} 失败: {error}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("{} 不是合法 JSON: {error}", path.display()))
}

/// 把 mtime 钉死，让扫描行里回落 mtime 的字段稳定。
fn freeze(path: &Path) {
    let time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(FIXED_MTIME as u64);
    if let Ok(handle) = std::fs::File::options().write(true).open(path) {
        let _ = handle.set_modified(time);
    }
}

/// 物化一个 case：`<home>/.grok/sessions/<case>/<bundle-id>/`。
fn materialize(sandbox: &Path, case: &str) -> PathBuf {
    let case_dir = repository_root()
        .join("tests/fixtures/agent_formats/grok")
        .join(case);
    let summary = read_json(&case_dir.join("summary.json"));
    let bundle_id = summary
        .get("info")
        .and_then(|info| info.get("id"))
        .and_then(Value::as_str)
        .unwrap_or(case)
        .to_string();
    let target = sandbox.join(".grok/sessions").join(case).join(&bundle_id);
    std::fs::create_dir_all(&target).expect("建立沙箱 bundle 目录");
    let mut members: Vec<PathBuf> = std::fs::read_dir(&case_dir)
        .expect("fixture 目录可读")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect();
    members.sort();
    for member in members {
        let name = member.file_name().expect("fixture 成员有文件名");
        std::fs::copy(&member, target.join(name)).expect("拷贝 fixture 成员");
        freeze(&target.join(name));
    }
    target
}

/// 递归把沙箱绝对路径换成 `<home>`。
fn normalize(value: &Value, sandbox: &str) -> Value {
    match value {
        Value::String(text) => Value::from(text.replace(sandbox, SANDBOX_MARKER)),
        Value::Array(items) => {
            Value::Array(items.iter().map(|item| normalize(item, sandbox)).collect())
        }
        Value::Object(entries) => Value::Object(
            entries
                .iter()
                .map(|(key, item)| (key.clone(), normalize(item, sandbox)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn golden(kind: &str, case: &str) -> Value {
    read_json(&repository_root().join(format!("tests/golden/{kind}/grok/{case}.json")))
}

#[test]
fn grok_canonical_sessions_match_the_python_baseline() {
    let _lock = ENVIRONMENT
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    for case in CASES {
        let sandbox = tempfile::tempdir().expect("建立沙箱");
        let _guard = set_grok_home(&sandbox.path().join(".grok"));
        let bundle = materialize(sandbox.path(), case);
        let session = reader::read(&bundle).unwrap_or_else(|error| {
            panic!("{case}: 读取失败 {}", error.message());
        });
        let actual = serde_json::to_value(&session).expect("Session 可序列化");
        let expected = golden("canonical", case);
        assert_eq!(actual, expected, "{case}: canonical 与黄金基线不一致");
    }
}

#[test]
fn grok_scan_rows_match_the_python_baseline() {
    let _lock = ENVIRONMENT
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    for case in CASES {
        let sandbox = tempfile::tempdir().expect("建立沙箱");
        let _guard = set_grok_home(&sandbox.path().join(".grok"));
        materialize(sandbox.path(), case);
        let rows = scanner::scan(&scanner::NullScanCache).expect("扫描成功");
        // 扫描行里的 path 直接来自 `GROK_HOME` 拼接，不做 realpath；但 macOS 的
        // `/var` 是指向 `/private/var` 的符号链接，两种形态都抹一遍才稳。
        let mut actual = Value::Array(rows.into_iter().map(Value::Object).collect());
        for root in [
            sandbox
                .path()
                .canonicalize()
                .unwrap_or_else(|_| sandbox.path().to_path_buf()),
            sandbox.path().to_path_buf(),
        ] {
            actual = normalize(&actual, &root.to_string_lossy());
        }
        let expected = golden("scan", case);
        assert_eq!(actual, expected["rows"], "{case}: scan 行与黄金基线不一致");
    }
}

/// case-03 的死分支（被 rewind 截掉的 prompt）不得出现在 canonical 里。
#[test]
fn grok_rewound_branch_is_invisible() {
    let _lock = ENVIRONMENT
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let sandbox = tempfile::tempdir().expect("建立沙箱");
    let _guard = set_grok_home(&sandbox.path().join(".grok"));
    let bundle = materialize(sandbox.path(), "case-03-rewind");
    let session = reader::read(&bundle).expect("读取成功");
    let texts: Vec<&str> = session
        .messages
        .iter()
        .flat_map(|message| message.blocks.iter())
        .map(|block| block.text.as_str())
        .collect();
    assert_eq!(texts, ["first", "live"]);
    assert!(!texts.contains(&"dead prompt"));
    assert!(!texts.contains(&"dead"));
}

/// case-04 没有 updates.jsonl，必须走 chat_history 回退而不是报格式变更。
#[test]
fn grok_chat_history_fallback_carries_the_session() {
    let _lock = ENVIRONMENT
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let sandbox = tempfile::tempdir().expect("建立沙箱");
    let _guard = set_grok_home(&sandbox.path().join(".grok"));
    let bundle = materialize(sandbox.path(), "case-04-chat-fallback");
    assert!(!bundle.join("updates.jsonl").exists());
    let session = reader::read(&bundle).expect("读取成功");
    // source_id 来自 chat 行自己的 id，而不是 updates 的 prompt 键。
    let ids: Vec<Option<&str>> = session
        .messages
        .iter()
        .map(|message| message.source_id.as_deref())
        .collect();
    assert_eq!(ids, [Some("u1"), Some("a1")]);
    let rows = scanner::scan(&scanner::NullScanCache).expect("扫描成功");
    assert_eq!(
        rows[0]["authoritative_members"],
        serde_json::json!(["summary.json", "chat_history.jsonl"])
    );
}
