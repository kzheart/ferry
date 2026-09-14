//! Grok Build 标题写回。
//!
//! Grok 的标题就是 bundle 里 `summary.json` 的 `generated_title`（与 `session_summary`
//! 恒等），dashboard / `/resume` / `-r <title>` 每次直接读它。手动 `/rename` 会同时置
//! `title_is_manual=true`——不写这个标志，grok 早期几轮的自动刷新会把标题改回去。
//! `grok sessions search` 走 `session_search.sqlite` 的副本，改完要同步索引行。

use std::fs::{self, File, OpenOptions};
use std::path::Path;
#[cfg(unix)]
use std::time::{Duration, Instant};

use serde_json::{Map, Value};

use crate::adapters::contracts::SessionRenamer;
use crate::adapters::shared::writing::python_json_dumps;
use crate::errors::{DomainError, DomainResult};
use crate::system::paths::{grok_home, home_dir, process_environ};

use super::adapter::resolve;
use super::scanner::sessions_root;
use super::store::read_text;
use super::writer::index_bundle;

pub struct GrokRenamer;

#[cfg(unix)]
const LOCK_WAIT: Duration = Duration::from_secs(3);

/// grok 用 flock 的 `<file>.lock` sidecar 串行化对 summary.json 的写入；这里拿同一把锁。
struct BundleLock {
    #[allow(dead_code)]
    file: File,
}

impl BundleLock {
    fn acquire(summary: &Path) -> DomainResult<Self> {
        let lock_path = summary.with_extension("json.lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(|error| {
                DomainError::internal(format!(
                    "打开 Grok 锁文件失败: {}: {error}",
                    lock_path.display()
                ))
            })?;
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd as _;
            let deadline = Instant::now() + LOCK_WAIT;
            loop {
                // SAFETY: 对一个有效、已打开的文件描述符调用 flock，是纯系统调用。
                let code = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
                if code == 0 {
                    break;
                }
                if Instant::now() >= deadline {
                    return Err(DomainError::session_store_unavailable(
                        "grok",
                        "summary.json 正被 grok 进程持锁，稍后再试",
                    ));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        Ok(Self { file })
    }
}

#[cfg(unix)]
impl Drop for BundleLock {
    fn drop(&mut self) {
        use std::os::unix::io::AsRawFd as _;
        // SAFETY: 释放本进程持有的 flock；失败无后果（关闭 fd 也会释放）。
        unsafe {
            libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

/// `~/.grok/active_sessions.json` 列出的会话正在被 grok 进程跑；它会在 turn 结束时
/// 用内存态整体覆写 summary.json，外部改名会被静默丢掉，所以直接拒绝。
fn assert_not_active(active_sessions: &Path, session_id: &str) -> DomainResult<()> {
    let Ok(text) = fs::read_to_string(active_sessions) else {
        return Ok(());
    };
    if !session_id.is_empty() && text.contains(session_id) {
        return Err(DomainError::session_store_unavailable(
            "grok",
            &format!("Grok 会话 {session_id} 正在运行，请结束后再改名"),
        ));
    }
    Ok(())
}

fn atomic_write(path: &Path, payload: &str) -> DomainResult<()> {
    use std::io::Write as _;
    let temporary = path.with_extension(format!("json.{}.tmp", std::process::id()));
    let outcome = (|| -> std::io::Result<()> {
        let mut file = File::create(&temporary)?;
        file.write_all(payload.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        if let Some(parent) = path.parent() {
            if let Ok(directory) = File::open(parent) {
                let _ = directory.sync_all();
            }
        }
        Ok(())
    })();
    if outcome.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    outcome.map_err(|error| {
        DomainError::internal(format!(
            "写入 Grok summary.json 失败: {}: {error}",
            path.display()
        ))
    })
}

pub fn rename_bundle(
    bundle: &Path,
    title: &str,
    active_sessions: &Path,
    sessions_root: Option<&Path>,
) -> DomainResult<Map<String, Value>> {
    let summary_path = bundle.join("summary.json");
    let text = read_text(&summary_path)
        .map_err(|_| DomainError::session_not_found("grok", &bundle.to_string_lossy()))?;
    let mut summary: Value = serde_json::from_str(&text)
        .map_err(|error| DomainError::internal(format!("Grok summary.json 不可解析: {error}")))?;
    let session_id = summary
        .get("info")
        .and_then(|info| info.get("id"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    assert_not_active(active_sessions, &session_id)?;

    let _lock = BundleLock::acquire(&summary_path)?;
    {
        let object = summary
            .as_object_mut()
            .ok_or_else(|| DomainError::internal("Grok summary.json 顶层不是 object"))?;
        object.insert("session_summary".into(), Value::from(title));
        object.insert("generated_title".into(), Value::from(title));
        object.insert("title_is_manual".into(), Value::Bool(true));
    }
    atomic_write(&summary_path, &python_json_dumps(&summary))?;

    let mut notes: Vec<Value> = Vec::new();
    if let Some(root) = sessions_root {
        if let Err(error) = index_bundle(bundle, root) {
            notes.push(Value::from(format!(
                "标题已改，但 grok sessions search 的索引未能同步: {}",
                error.message()
            )));
        }
    }

    let mut result = Map::new();
    result.insert("title".into(), Value::from(title));
    result.insert("session_id".into(), Value::from(session_id));
    result.insert(
        "saved_as".into(),
        Value::from(summary_path.to_string_lossy().into_owned()),
    );
    result.insert("title_is_manual".into(), Value::Bool(true));
    result.insert("notes".into(), Value::Array(notes));
    Ok(result)
}

impl SessionRenamer for GrokRenamer {
    fn rename(&self, reference: &str, title: &str) -> DomainResult<Map<String, Value>> {
        let bundle = resolve(reference)?;
        let home = grok_home(&process_environ(), &home_dir());
        let root = sessions_root();
        rename_bundle(
            &bundle,
            title,
            &home.join("active_sessions.json"),
            Some(&root),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn bundle(root: &Path) -> std::path::PathBuf {
        let dir = root.join("sessions/%2Fw/sid-1");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("summary.json"),
            json!({
                "info": {"id": "sid-1", "cwd": "/w"},
                "chat_format_version": 1,
                "session_summary": "Old",
                "generated_title": "Old",
                "updated_at": "2026-09-05T10:00:00.000Z",
                "num_chat_messages": 2
            })
            .to_string(),
        )
        .unwrap();
        fs::write(dir.join("summary.json.lock"), b"").unwrap();
        dir
    }

    #[test]
    fn rename_rewrites_the_three_title_fields_and_keeps_the_rest() {
        let root = tempfile::tempdir().unwrap();
        let dir = bundle(root.path());
        let result = rename_bundle(
            &dir,
            "Renamed",
            &root.path().join("active_sessions.json"),
            None,
        )
        .unwrap();
        assert_eq!(result["title"], json!("Renamed"));
        assert_eq!(result["session_id"], json!("sid-1"));
        let summary: Value =
            serde_json::from_str(&fs::read_to_string(dir.join("summary.json")).unwrap()).unwrap();
        assert_eq!(summary["generated_title"], json!("Renamed"));
        assert_eq!(summary["session_summary"], json!("Renamed"));
        assert_eq!(summary["title_is_manual"], json!(true));
        assert_eq!(summary["num_chat_messages"], json!(2));
        assert_eq!(summary["updated_at"], json!("2026-09-05T10:00:00.000Z"));
        assert!(!dir
            .join(format!("summary.json.{}.tmp", std::process::id()))
            .exists());
    }

    #[test]
    fn rename_refuses_an_active_session() {
        let root = tempfile::tempdir().unwrap();
        let dir = bundle(root.path());
        let active = root.path().join("active_sessions.json");
        fs::write(&active, r#"[{"id":"sid-1","pid":1}]"#).unwrap();
        let error = rename_bundle(&dir, "x", &active, None).unwrap_err();
        assert_eq!(error.code, "session.store_unavailable");
        let summary: Value =
            serde_json::from_str(&fs::read_to_string(dir.join("summary.json")).unwrap()).unwrap();
        assert_eq!(summary["generated_title"], json!("Old"));
    }

    #[test]
    fn rename_reports_a_missing_bundle() {
        let root = tempfile::tempdir().unwrap();
        let error = rename_bundle(
            &root.path().join("nope"),
            "x",
            &root.path().join("active_sessions.json"),
            None,
        )
        .unwrap_err();
        assert_eq!(error.code, "session.not_found");
    }
}
