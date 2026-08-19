//! 基于文件修订信息的扫描缓存。
//!
//! 语义事实源：`engine/sessions/scan_cache.py`。
//!
//! 磁盘格式（`~/.ferry/scan-cache.json`，`version: 6`）与 Python 引擎**互认**：
//! 同一个文件两个引擎可以交替读写，字段名与取值口径逐个对齐。
//!
//! ```jsonc
//! {
//!   "<path>":  {"version": 6, "mtime": <st_mtime_ns>, "size": <bytes>,
//!               "meta": <扫描行 | null>},
//!   "digests": {"<path>": {"dev": .., "ino": .., "mtime": <st_mtime_ns>,
//!                          "size": .., "sha256": "<hex>"}}
//! }
//! ```

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use serde_json::{Map, Value};

use crate::adapters::contracts::{ScanCache as ScanCachePort, ScanRow};
use crate::jsonutil::FileStat;
use crate::system::paths::home_dir;

const DIGESTS_KEY: &str = "digests";
/// 缓存条目格式版本；与 Python 侧的默认值必须一致。
pub const SCAN_CACHE_VERSION: i64 = 6;

/// 条目合并规则：同一个 key 取 mtime 较新的那份。
fn newer(candidate: &Value, current: Option<&Value>) -> bool {
    let Some(Value::Object(current)) = current else {
        return true;
    };
    let mtime =
        |entry: &Map<String, Value>| entry.get("mtime").and_then(Value::as_i64).unwrap_or(-1);
    let candidate = match candidate {
        Value::Object(entry) => mtime(entry),
        _ => -1,
    };
    candidate >= mtime(current)
}

fn merge(base: &Map<String, Value>, incoming: &Map<String, Value>) -> Map<String, Value> {
    let mut merged: Map<String, Value> = base
        .iter()
        .filter(|(key, _)| key.as_str() != DIGESTS_KEY)
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    for (key, value) in incoming {
        if key == DIGESTS_KEY {
            continue;
        }
        if newer(value, merged.get(key)) {
            merged.insert(key.clone(), value.clone());
        }
    }
    let mut digests = base
        .get(DIGESTS_KEY)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(incoming_digests) = incoming.get(DIGESTS_KEY).and_then(Value::as_object) {
        for (key, value) in incoming_digests {
            if newer(value, digests.get(key)) {
                digests.insert(key.clone(), value.clone());
            }
        }
    }
    if !digests.is_empty() {
        merged.insert(DIGESTS_KEY.into(), Value::Object(digests));
    }
    merged
}

pub struct ScanCache {
    path: PathBuf,
    version: i64,
    /// `None` = 尚未从磁盘装载（对齐 Python 的懒加载）。
    data: Mutex<Option<Map<String, Value>>>,
}

impl ScanCache {
    /// 默认落在 `~/.ferry/scan-cache.json`。
    pub fn new(path: Option<PathBuf>) -> Self {
        Self::with_version(path, SCAN_CACHE_VERSION)
    }

    pub fn with_version(path: Option<PathBuf>, version: i64) -> Self {
        Self {
            path: path.unwrap_or_else(default_cache_path),
            version,
            data: Mutex::new(None),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn read_disk(&self) -> Map<String, Value> {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default()
    }

    fn loaded(&self) -> MutexGuard<'_, Option<Map<String, Value>>> {
        let mut guard = self
            .data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if guard.is_none() {
            // 先释放锁做 IO 会引入重复读；条目量以千计，读一次是毫秒级。
            *guard = Some(self.read_disk());
        }
        guard
    }
}

fn default_cache_path() -> PathBuf {
    home_dir().join(".ferry").join("scan-cache.json")
}

impl ScanCachePort for ScanCache {
    fn get(&self, path: &Path, stat: &FileStat) -> Option<Option<ScanRow>> {
        let guard = self.loaded();
        let data = guard.as_ref().expect("loaded() 保证已装载");
        let hit = data
            .get(&path.to_string_lossy().into_owned())?
            .as_object()?;
        let matches = hit.get("version").and_then(Value::as_i64) == Some(self.version)
            && hit.get("mtime").and_then(Value::as_i64) == Some(stat.mtime_ns as i64)
            && hit.get("size").and_then(Value::as_i64) == Some(stat.size as i64);
        if !matches {
            return None;
        }
        // 内层 None = 「已知不是会话」（Python 存的是 meta: null）。
        Some(hit.get("meta").and_then(Value::as_object).cloned())
    }

    fn put(&self, path: &Path, stat: &FileStat, meta: Option<ScanRow>) {
        let mut guard = self.loaded();
        let data = guard.as_mut().expect("loaded() 保证已装载");
        let mut entry = Map::new();
        entry.insert("version".into(), Value::from(self.version));
        entry.insert("mtime".into(), Value::from(stat.mtime_ns as i64));
        entry.insert("size".into(), Value::from(stat.size as i64));
        entry.insert(
            "meta".into(),
            meta.map(Value::Object).unwrap_or(Value::Null),
        );
        data.insert(path.to_string_lossy().into_owned(), Value::Object(entry));
    }

    fn get_digest(&self, path: &Path, stat: &FileStat) -> Option<String> {
        let guard = self.loaded();
        let data = guard.as_ref().expect("loaded() 保证已装载");
        let hit = data
            .get(DIGESTS_KEY)?
            .as_object()?
            .get(&path.to_string_lossy().into_owned())?
            .as_object()?;
        let matches = hit.get("dev").and_then(Value::as_i64) == Some(stat.dev as i64)
            && hit.get("ino").and_then(Value::as_i64) == Some(stat.ino as i64)
            && hit.get("mtime").and_then(Value::as_i64) == Some(stat.mtime_ns as i64)
            && hit.get("size").and_then(Value::as_i64) == Some(stat.size as i64);
        if !matches {
            return None;
        }
        hit.get("sha256")
            .and_then(Value::as_str)
            .map(str::to_string)
    }

    fn put_digest(&self, path: &Path, stat: &FileStat, digest: &str) {
        let mut guard = self.loaded();
        let data = guard.as_mut().expect("loaded() 保证已装载");
        let digests = data
            .entry(DIGESTS_KEY.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if !digests.is_object() {
            *digests = Value::Object(Map::new());
        }
        let mut entry = Map::new();
        entry.insert("dev".into(), Value::from(stat.dev as i64));
        entry.insert("ino".into(), Value::from(stat.ino as i64));
        entry.insert("mtime".into(), Value::from(stat.mtime_ns as i64));
        entry.insert("size".into(), Value::from(stat.size as i64));
        entry.insert("sha256".into(), Value::from(digest));
        digests
            .as_object_mut()
            .expect("上一步已保证是 object")
            .insert(path.to_string_lossy().into_owned(), Value::Object(entry));
    }

    fn flush(&self) {
        let mut guard = self
            .data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(current) = guard.as_ref() else {
            return;
        };
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // 直接整份覆盖会把别人刚写进去的条目丢掉：持锁做
        // 「读回磁盘最新 → 合并本实例增量 → 写回」。
        let merged = merge(&self.read_disk(), current);
        let name = self
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "scan-cache.json".to_string());
        // 进程内可能有并发扫描（预热线程 + RPC），临时文件必须按线程区分。
        let temp = self.path.with_file_name(format!(
            "{name}.{}.{}.tmp",
            std::process::id(),
            thread_marker()
        ));
        let payload = serde_json::to_string(&Value::Object(merged.clone())).unwrap_or_default();
        if std::fs::write(&temp, payload).is_ok() && std::fs::rename(&temp, &self.path).is_ok() {
            *guard = Some(merged);
        } else {
            let _ = std::fs::remove_file(&temp);
        }
    }
}

/// `threading.get_ident()` 的等价物：只要求进程内唯一。
fn thread_marker() -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::thread::current().id().hash(&mut hasher);
    hasher.finish()
}

static SHARED: LazyLock<Mutex<Option<Arc<ScanCache>>>> = LazyLock::new(|| Mutex::new(None));

/// 进程级共享缓存：预热扫描与 scan RPC 复用同一份，不再互相覆盖。
pub fn shared_cache() -> Arc<ScanCache> {
    let mut guard = SHARED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard
        .get_or_insert_with(|| Arc::new(ScanCache::new(None)))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn stat(mtime_ns: i64, size: u64) -> FileStat {
        FileStat {
            dev: 1,
            ino: 2,
            mtime_ns: i128::from(mtime_ns),
            size,
        }
    }

    #[test]
    fn entries_hit_only_on_exact_version_mtime_and_size() {
        let temp = tempfile::tempdir().unwrap();
        let cache = ScanCache::new(Some(temp.path().join("scan-cache.json")));
        let path = Path::new("/tmp/a.jsonl");
        let row = json!({"id": "a"}).as_object().unwrap().clone();
        cache.put(path, &stat(10, 20), Some(row.clone()));
        assert_eq!(cache.get(path, &stat(10, 20)), Some(Some(row)));
        assert_eq!(cache.get(path, &stat(11, 20)), None);
        assert_eq!(cache.get(path, &stat(10, 21)), None);
        assert_eq!(cache.get(Path::new("/tmp/b.jsonl"), &stat(10, 20)), None);
        // meta=null 表示「已知不是会话」：命中但内层为空。
        cache.put(path, &stat(30, 40), None);
        assert_eq!(cache.get(path, &stat(30, 40)), Some(None));
    }

    #[test]
    fn digests_require_the_full_stat_quadruple() {
        let temp = tempfile::tempdir().unwrap();
        let cache = ScanCache::new(Some(temp.path().join("scan-cache.json")));
        let path = Path::new("/tmp/a.jsonl");
        cache.put_digest(path, &stat(10, 20), "deadbeef");
        assert_eq!(
            cache.get_digest(path, &stat(10, 20)),
            Some("deadbeef".into())
        );
        assert_eq!(cache.get_digest(path, &stat(10, 21)), None);
        let mut other = stat(10, 20);
        other.ino = 9;
        assert_eq!(cache.get_digest(path, &other), None);
    }

    /// 磁盘格式必须与 Python 引擎互认：字段名与形状逐个对齐。
    #[test]
    fn disk_format_matches_the_python_layout() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("scan-cache.json");
        let cache = ScanCache::new(Some(file.clone()));
        let row = json!({"id": "a"}).as_object().unwrap().clone();
        cache.put(Path::new("/tmp/a.jsonl"), &stat(10, 20), Some(row));
        cache.put_digest(Path::new("/tmp/a.jsonl"), &stat(10, 20), "abc");
        cache.flush();

        let stored: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(
            stored["/tmp/a.jsonl"],
            json!({"version": 6, "mtime": 10, "size": 20, "meta": {"id": "a"}})
        );
        assert_eq!(
            stored["digests"]["/tmp/a.jsonl"],
            json!({"dev": 1, "ino": 2, "mtime": 10, "size": 20, "sha256": "abc"})
        );
        // 反过来：Python 写的文件能被读回。
        let reader = ScanCache::new(Some(file));
        assert!(reader
            .get(Path::new("/tmp/a.jsonl"), &stat(10, 20))
            .is_some());
    }

    #[test]
    fn version_mismatch_invalidates_every_entry() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("scan-cache.json");
        let old = ScanCache::with_version(Some(file.clone()), 5);
        old.put(Path::new("/tmp/a.jsonl"), &stat(10, 20), None);
        old.flush();
        let current = ScanCache::new(Some(file));
        assert_eq!(current.get(Path::new("/tmp/a.jsonl"), &stat(10, 20)), None);
    }

    #[test]
    fn flush_merges_instead_of_clobbering_concurrent_writers() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("scan-cache.json");
        let first = ScanCache::new(Some(file.clone()));
        let second = ScanCache::new(Some(file.clone()));
        first.put(Path::new("/a"), &stat(10, 1), None);
        second.put(Path::new("/b"), &stat(10, 1), None);
        first.flush();
        second.flush();
        let stored: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert!(stored.get("/a").is_some(), "先写的条目不能被后写者抹掉");
        assert!(stored.get("/b").is_some());

        // 同一 key：mtime 较新的一份胜出。
        let third = ScanCache::new(Some(file.clone()));
        third.put(Path::new("/a"), &stat(99, 7), None);
        third.flush();
        let stored: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(stored["/a"]["mtime"], Value::from(99));
    }

    #[test]
    fn corrupt_files_degrade_to_an_empty_cache() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("scan-cache.json");
        std::fs::write(&file, "not json").unwrap();
        let cache = ScanCache::new(Some(file));
        assert_eq!(cache.get(Path::new("/a"), &stat(1, 1)), None);
    }
}
