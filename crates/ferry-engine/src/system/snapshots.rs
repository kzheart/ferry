//! Filesystem snapshot store shared by native session implementations.

use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

use super::paths::home_dir;

/// `~/.ferry`
pub fn default_data_dir() -> PathBuf {
    home_dir().join(".ferry")
}

/// `~/.ferry/backups`
pub fn default_backup_dir() -> PathBuf {
    default_data_dir().join("backups")
}

/// 快照根目录；`FERRY_BACKUP_DIR` 可覆盖，测试据此隔离出 tmp 目录。
pub fn backup_dir() -> PathBuf {
    match std::env::var("FERRY_BACKUP_DIR") {
        Ok(value) if !value.is_empty() => PathBuf::from(value),
        _ => default_backup_dir(),
    }
}

/// Ferry 自有状态根目录（配置与状态库同处 `~/.ferry`，与备份快照分开）。
///
/// `FERRY_DATA_DIR` 可覆盖，测试据此隔离；与 runtime 的
/// `FERRY_RUNTIME_DATA_DIR` 指向同一个 `~/.ferry`。
pub fn data_dir() -> PathBuf {
    match std::env::var("FERRY_DATA_DIR") {
        Ok(value) if !value.is_empty() => PathBuf::from(value),
        _ => default_data_dir(),
    }
}

/// 纳秒级 ID 避免同一秒内「编辑前快照」和「还原前保护」互相覆盖。
fn new_dest(stem: &str) -> io::Result<PathBuf> {
    let root = backup_dir();
    std::fs::create_dir_all(&root)?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    Ok(root.join(format!("{stem}-{nanos}.jsonl")))
}

fn write_meta(
    dest: &Path,
    reason_code: &str,
    tool: &str,
    source: &str,
    extra: Option<&Map<String, Value>>,
) -> io::Result<()> {
    let mut payload = Map::new();
    payload.insert("reason_code".into(), Value::from(reason_code));
    payload.insert("tool".into(), Value::from(tool));
    payload.insert("source".into(), Value::from(source));
    if let Some(extra) = extra {
        for (key, value) in extra {
            payload.insert(key.clone(), value.clone());
        }
    }
    // `with_suffix(".meta.json")` 的等价形式：替换最后一段扩展名。
    let meta = dest.with_extension("meta.json");
    std::fs::write(&meta, serde_json::to_string(&Value::Object(payload))?)
}

/// 复制一份文件快照，写同名 `.meta.json` 记录创建原因。
pub fn snapshot_file(
    path: &Path,
    reason_code: &str,
    tool: &str,
    extra: Option<&Map<String, Value>>,
) -> io::Result<PathBuf> {
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let dest = new_dest(&stem)?;
    std::fs::copy(path, &dest)?;
    write_meta(&dest, reason_code, tool, &path.to_string_lossy(), extra)?;
    Ok(dest)
}

/// 把内存中的会话导出内容落成快照（数据库型工具使用）。
pub fn snapshot_payload(
    stem: &str,
    payload: &str,
    reason_code: &str,
    tool: &str,
    source: &str,
    extra: Option<&Map<String, Value>>,
) -> io::Result<PathBuf> {
    let dest = new_dest(stem)?;
    std::fs::write(&dest, payload)?;
    write_meta(&dest, reason_code, tool, source, extra)?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_payload_writes_a_sibling_meta_file() {
        let temp = tempfile::tempdir().unwrap();
        // 环境变量是进程级的：走 crate 级的环境锁，析构自动恢复。
        let env =
            crate::system::paths::testing::EnvGuard::acquire().set("FERRY_BACKUP_DIR", temp.path());
        let dest = snapshot_payload(
            "session.a",
            "{\"a\":1}\n",
            "snapshot.before_edit",
            "claude",
            "/tmp/session.a.jsonl",
            None,
        )
        .unwrap();
        drop(env);

        assert_eq!(dest.extension().unwrap(), "jsonl");
        assert!(dest
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("session.a-"));
        let meta = dest.with_extension("meta.json");
        let payload: Value = serde_json::from_str(&std::fs::read_to_string(meta).unwrap()).unwrap();
        assert_eq!(payload["reason_code"], Value::from("snapshot.before_edit"));
        assert_eq!(payload["tool"], Value::from("claude"));
        assert_eq!(payload["source"], Value::from("/tmp/session.a.jsonl"));
    }
}
