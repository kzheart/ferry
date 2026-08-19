//! Grok 的 resume 与永久删除。
//!
//! grok 是目录型存储，删除不是 unlink 一个文件：
//! - `cleanup`（迁移回滚）先把整棵子树 `rename` 到隔离名，删索引行成功后才真删；
//!   任一步失败就把隔离目录换回去，保证迁移失败不会吃掉别人的会话。
//! - `delete`（用户永久删除）交给 `grok sessions delete` CLI，我们只负责补删
//!   索引行——bundle 之外还有 Grok 自己的状态，绕过 CLI 会留下悬挂引用。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Map, Value};

use crate::adapters::contracts::AgentAdapter;
use crate::adapters::shared::lifecycle::BaseLifecycle;
use crate::errors::{DomainError, DomainResult};
use crate::system::executables;
use crate::system::paths::home_dir;
use crate::system::probes;

use super::store::read_text;
use super::writer::delete_index_rows;

pub struct GrokLifecycle {
    executable: String,
}

impl GrokLifecycle {
    pub fn new(executable: impl Into<String>) -> Self {
        Self {
            executable: executable.into(),
        }
    }

    /// 归属该会话的全部 bundle：自己，加上把它当 root 的整棵子树。
    fn owned_bundles(
        session_id: &str,
        dest: &Path,
    ) -> DomainResult<(PathBuf, Vec<(String, PathBuf)>)> {
        let destination = fs::canonicalize(dest).unwrap_or_else(|_| dest.to_path_buf());
        // `<sessions_root>/<urlencode(cwd)>/<session-uuid>` → 上溯两级。
        let sessions_root = destination
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| DomainError::internal("无法定位 Grok 会话根目录"))?
            .to_path_buf();
        let mut owned = Vec::new();
        for entry in walkdir::WalkDir::new(&sessions_root)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
        {
            if entry.file_name() != "summary.json" || !entry.file_type().is_file() {
                continue;
            }
            let Some(summary) = read_text(entry.path())
                .ok()
                .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            else {
                continue;
            };
            let info = summary.get("info").cloned().unwrap_or(Value::Null);
            let id = info.get("id").and_then(Value::as_str);
            let root = summary.get("root_session_id").and_then(Value::as_str);
            if id == Some(session_id) || root == Some(session_id) {
                owned.push((
                    id.unwrap_or_default().to_string(),
                    entry
                        .path()
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_default(),
                ));
            }
        }
        owned.sort();
        Ok((sessions_root, owned))
    }
}

impl BaseLifecycle for GrokLifecycle {
    fn tool(&self) -> &str {
        "grok"
    }

    fn executable(&self) -> &str {
        &self.executable
    }

    fn resume_args(&self, session_id: &str) -> DomainResult<Vec<String>> {
        Ok(vec!["--resume".to_string(), session_id.to_string()])
    }

    fn cleanup(&self, session_id: &str, dest: &Path) -> DomainResult<()> {
        let (sessions_root, owned) = Self::owned_bundles(session_id, dest)?;
        let ids: Vec<String> = owned
            .iter()
            .map(|(id, _)| id.clone())
            .filter(|id| !id.is_empty())
            .collect();
        let mut quarantined: Vec<(PathBuf, PathBuf)> = Vec::new();
        let outcome = (|| -> DomainResult<()> {
            for (_, path) in &owned {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let temporary = path.with_file_name(format!(
                    ".{name}.ferry-cleanup.{}.tmp",
                    super::writer::cleanup_token()
                ));
                fs::rename(path, &temporary).map_err(|error| {
                    DomainError::internal(format!(
                        "隔离 Grok 会话失败: {}: {error}",
                        path.display()
                    ))
                })?;
                quarantined.push((path.clone(), temporary));
            }
            if !ids.is_empty() {
                delete_index_rows(&ids, &sessions_root)?;
            }
            Ok(())
        })();
        if let Err(error) = outcome {
            // 回滚：逆序把隔离目录换回原位（原位已被别人占用时不覆盖）。
            for (original, temporary) in quarantined.iter().rev() {
                if temporary.exists() && !original.exists() {
                    let _ = fs::rename(temporary, original);
                }
            }
            return Err(error);
        }
        for (_, temporary) in &quarantined {
            fs::remove_dir_all(temporary).map_err(|error| {
                DomainError::internal(format!(
                    "删除隔离目录失败: {}: {error}",
                    temporary.display()
                ))
            })?;
        }
        Ok(())
    }

    fn delete(&self, adapter: &AgentAdapter, reference: &str) -> DomainResult<Map<String, Value>> {
        let path = PathBuf::from(adapter.require_browser()?.resolve_ref(reference)?);
        let summary: Value = read_text(&path.join("summary.json"))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .ok_or_else(|| DomainError::session_not_found("grok", reference))?;
        let info = summary.get("info").cloned().unwrap_or(Value::Null);
        let session_id = info
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| DomainError::session_not_found("grok", reference))?
            .to_string();
        let cwd = info
            .get("cwd")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let command_cwd = if Path::new(&cwd).is_dir() {
            PathBuf::from(&cwd)
        } else {
            home_dir()
        };
        let command = executables::argv("grok", &["sessions", "delete", &session_id]);
        let result = probes::run(&command, Some(&command_cwd), Duration::from_secs(30), None)
            .map_err(|error| DomainError::internal(error.message))?;
        if result.returncode != Some(0) {
            let detail = if result.stderr.is_empty() {
                result.stdout.clone()
            } else {
                result.stderr.clone()
            };
            return Err(DomainError::internal(format!(
                "grok sessions delete 失败: {}",
                detail.trim()
            )));
        }
        let sessions_root = path
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| DomainError::internal("无法定位 Grok 会话根目录"))?;
        delete_index_rows(std::slice::from_ref(&session_id), sessions_root)?;
        let mut payload = Map::new();
        payload.insert("ok".into(), Value::Bool(true));
        payload.insert("session_id".into(), Value::from(session_id));
        Ok(payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn bundle(root: &Path, project: &str, id: &str, summary: Value) -> PathBuf {
        let path = root.join(project).join(id);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("summary.json"), summary.to_string()).unwrap();
        fs::write(path.join("updates.jsonl"), "{}\n").unwrap();
        path
    }

    #[test]
    fn resume_descriptor_uses_the_manifest_executable() {
        let lifecycle = GrokLifecycle::new("/usr/local/bin/grok");
        let descriptor = lifecycle.resume_descriptor("abc", "/work").unwrap();
        assert_eq!(descriptor["tool"], json!("grok"));
        assert_eq!(
            descriptor["display_command"],
            json!("cd /work && /usr/local/bin/grok --resume abc")
        );
    }

    #[test]
    fn cleanup_removes_the_whole_owned_subtree() {
        let root = tempfile::tempdir().unwrap();
        let sessions = root.path().join("sessions");
        let parent = bundle(
            &sessions,
            "p",
            "root-id",
            json!({"info": {"id": "root-id", "cwd": "/w"}, "chat_format_version": 1,
                   "root_session_id": "root-id"}),
        );
        let child = bundle(
            &sessions,
            "p",
            "child-id",
            json!({"info": {"id": "child-id", "cwd": "/w"}, "chat_format_version": 1,
                   "root_session_id": "root-id", "parent_session_id": "root-id"}),
        );
        // 与本次迁移无关的邻居必须留下。
        let neighbour = bundle(
            &sessions,
            "p",
            "other",
            json!({"info": {"id": "other", "cwd": "/w"}, "chat_format_version": 1}),
        );

        GrokLifecycle::new("grok")
            .cleanup("root-id", &parent)
            .unwrap();
        assert!(!parent.exists());
        assert!(!child.exists());
        assert!(neighbour.exists());
        // 隔离目录不得残留。
        let leftovers: Vec<_> = walkdir::WalkDir::new(&sessions)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains("ferry-cleanup")
            })
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn cleanup_rolls_back_when_the_index_refuses() {
        let root = tempfile::tempdir().unwrap();
        let sessions = root.path().join("sessions");
        let parent = bundle(
            &sessions,
            "p",
            "root-id",
            json!({"info": {"id": "root-id", "cwd": "/w"}, "chat_format_version": 1}),
        );
        // 一份 schema 不受支持的索引：删索引行必然失败。
        let database = rusqlite::Connection::open(sessions.join("session_search.sqlite")).unwrap();
        database
            .execute_batch("CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT)")
            .unwrap();
        drop(database);

        let error = GrokLifecycle::new("grok")
            .cleanup("root-id", &parent)
            .unwrap_err();
        assert_eq!(
            error.message(),
            "Grok session_search.sqlite 结构或版本不受支持"
        );
        // 回滚后 bundle 必须还在原位。
        assert!(parent.join("summary.json").is_file());
    }
}
