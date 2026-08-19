//! Claude 适配器的黄金对照（WP-C1 / WP-G）。
//!
//! `tests/golden/canonical/claude/*.json` 与 `tests/golden/scan/claude/*.json`
//! 由 Python 引擎 dump（见 `scripts/dump-canonical-fixtures.py`）。这里把
//! `tests/fixtures/agent_formats/claude/<case>` 物化到临时 HOME 沙箱，跑 Rust 的
//! scanner / reader，再与黄金文件逐字段比对。
//!
//! 环境相关字段按黄金文件 `_normalized.environment_dependent_fields` 清单处理：
//! `path` 把沙箱根替换成 `<home>` 后仍然逐字比对（存储布局是契约），
//! `updated` / `own_updated` 取决于物化时刻，比对前整树抹平；
//! `size` / `own_size` 只取决于 fixture 内容，照常比对。

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use ferry_engine::adapters::claude::{adapter, reader, scanner};
use ferry_engine::adapters::contracts::{ScanCache, ScanRow};
use ferry_engine::jsonutil::FileStat;
use serde_json::Value;

const CASES: &[&str] = &["case-01-plain", "case-02-tools"];
/// 物化 fixture 时钉死的 mtime（与 dump 脚本的 `FIXED_MTIME` 一致）。
const FIXED_MTIME: i64 = 1_784_937_600;
/// 与运行时刻绑定、不参与比对的字段。
const VOLATILE: &[&str] = &["updated", "own_updated"];

/// HOME 是进程级状态，改写它的用例必须串行。
static HOME_LOCK: Mutex<()> = Mutex::new(());

struct HomeSandbox {
    _guard: MutexGuard<'static, ()>,
    previous: Option<String>,
    root: tempfile::TempDir,
}

impl HomeSandbox {
    fn enter() -> Self {
        let guard = HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let root = tempfile::tempdir().expect("临时 HOME 可创建");
        let previous = std::env::var("HOME").ok();
        unsafe { std::env::set_var("HOME", root.path()) };
        Self {
            _guard: guard,
            previous,
            root,
        }
    }

    fn path(&self) -> &Path {
        self.root.path()
    }
}

impl Drop for HomeSandbox {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => unsafe { std::env::set_var("HOME", value) },
            None => unsafe { std::env::remove_var("HOME") },
        }
    }
}

/// 扫描期不缓存：每次都真解析，保证对照的是解析结果本身。
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

fn fixture_dir(case: &str) -> PathBuf {
    repository_root()
        .join("tests/fixtures/agent_formats/claude")
        .join(case)
}

fn golden(kind: &str, case: &str) -> Value {
    let path = repository_root()
        .join("tests/golden")
        .join(kind)
        .join("claude")
        .join(format!("{case}.json"));
    serde_json::from_str(&std::fs::read_to_string(&path).expect("黄金文件可读"))
        .expect("黄金文件是合法 JSON")
}

/// 对齐 dump 脚本的 `_native_stem`：优先复用 manifest 记录的原生文件名。
fn native_stem(case: &str) -> String {
    let manifest: Value = serde_json::from_str(
        &std::fs::read_to_string(fixture_dir(case).join("manifest.json")).expect("manifest 可读"),
    )
    .expect("manifest 是合法 JSON");
    manifest["source_paths"][0]
        .as_str()
        .map(|source| {
            Path::new(source)
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
        .or_else(|| {
            manifest["session_id"]
                .as_str()
                .map(std::string::ToString::to_string)
        })
        .unwrap_or_else(|| case.to_string())
}

/// `<home>/.claude/projects/<case>/<stem>.jsonl`
fn materialize(home: &Path, case: &str) -> PathBuf {
    let target = home
        .join(".claude/projects")
        .join(case)
        .join(format!("{}.jsonl", native_stem(case)));
    std::fs::create_dir_all(target.parent().expect("目标有父目录")).expect("沙箱目录可创建");
    std::fs::copy(fixture_dir(case).join("session.jsonl"), &target).expect("fixture 可复制");
    freeze(&target);
    target
}

/// 把 mtime 钉到 dump 脚本使用的固定值（`utimes(2)`，无需额外依赖）。
#[cfg(unix)]
fn freeze(path: &Path) {
    use std::ffi::CString;
    let raw = CString::new(path.as_os_str().as_encoded_bytes()).expect("路径不含 NUL");
    let times = [
        libc::timeval {
            tv_sec: FIXED_MTIME as libc::time_t,
            tv_usec: 0,
        },
        libc::timeval {
            tv_sec: FIXED_MTIME as libc::time_t,
            tv_usec: 0,
        },
    ];
    unsafe { libc::utimes(raw.as_ptr(), times.as_ptr()) };
}

#[cfg(not(unix))]
fn freeze(_path: &Path) {}

/// 把沙箱绝对路径换成黄金文件里的稳定字面量。
fn normalize_home(value: &Value, home: &str) -> Value {
    match value {
        Value::String(text) => Value::String(text.replace(home, "<home>")),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| normalize_home(item, home))
                .collect(),
        ),
        Value::Object(entries) => Value::Object(
            entries
                .iter()
                .map(|(key, item)| (key.clone(), normalize_home(item, home)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// 递归剔除随运行环境变化的字段。
fn drop_volatile(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(drop_volatile).collect()),
        Value::Object(entries) => Value::Object(
            entries
                .iter()
                .filter(|(key, _)| !VOLATILE.contains(&key.as_str()))
                .map(|(key, item)| (key.clone(), drop_volatile(item)))
                .collect(),
        ),
        other => other.clone(),
    }
}

#[test]
fn claude_canonical_sessions_match_the_python_baseline() {
    adapter::build().expect("claude adapter 可装配（同时注册方言）");
    let sandbox = HomeSandbox::enter();
    for case in CASES {
        let path = materialize(sandbox.path(), case);
        let session = reader::read(&path.to_string_lossy()).expect("fixture 可读");
        let actual = serde_json::to_value(&session).expect("Session 可序列化");
        let expected = golden("canonical", case);
        assert_eq!(actual, expected, "canonical/{case} 与黄金基线不一致");
    }
}

#[test]
fn claude_scan_rows_match_the_python_baseline() {
    adapter::build().expect("claude adapter 可装配");
    let sandbox = HomeSandbox::enter();
    let home = sandbox.path().to_string_lossy().into_owned();
    for case in CASES {
        // 每个 case 单独物化、单独扫描，扫描行天然只含该 case 的会话树。
        let projects = sandbox.path().join(".claude/projects");
        let _ = std::fs::remove_dir_all(&projects);
        materialize(sandbox.path(), case);

        let rows = scanner::scan(&NullCache).expect("扫描可完成");
        let actual = normalize_home(
            &Value::Array(rows.into_iter().map(Value::Object).collect()),
            &home,
        );
        let expected = golden("scan", case);

        // 黄金文件声明的环境相关字段清单必须与本测试的假设一致。
        assert_eq!(
            expected["_normalized"]["environment_dependent_fields"],
            serde_json::json!(["path", "updated", "own_updated", "size", "own_size"]),
            "scan/{case} 的归一化清单发生了变化"
        );
        assert_eq!(
            expected["_normalized"]["fixed_mtime_seconds"],
            serde_json::json!(FIXED_MTIME)
        );

        assert_eq!(
            drop_volatile(&actual),
            drop_volatile(&expected["rows"]),
            "scan/{case} 与黄金基线不一致"
        );
        // mtime 已在物化时钉死，因此这两个字段也应当逐值命中。
        assert_eq!(
            actual[0]["updated"], expected["rows"][0]["updated"],
            "scan/{case} 的 updated 未命中钉死的 mtime"
        );
        assert_eq!(actual[0]["own_updated"], expected["rows"][0]["own_updated"]);
    }
}
