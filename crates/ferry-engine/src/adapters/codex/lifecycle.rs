//! Codex 会话生命周期：resume 与迁移清理策略。

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::adapters::shared::lifecycle::BaseLifecycle;
use crate::adapters::shared::scanner::iter_lines;
use crate::errors::DomainResult;

use super::native::{discover_closure, CodexStore};
use super::registry::unregister_tree;

/// Codex 生命周期策略。
pub struct CodexLifecycle {
    executable: String,
}

impl CodexLifecycle {
    pub fn new(executable: impl Into<String>) -> Self {
        Self {
            executable: executable.into(),
        }
    }
}

impl BaseLifecycle for CodexLifecycle {
    fn tool(&self) -> &str {
        "codex"
    }

    fn executable(&self) -> &str {
        &self.executable
    }

    fn resume_args(&self, session_id: &str) -> DomainResult<Vec<String>> {
        Ok(vec!["resume".to_string(), session_id.to_string()])
    }

    /// 迁移失败时清理已写入的 rollout 树与注册行。
    fn cleanup(&self, session_id: &str, dest: &Path) -> DomainResult<()> {
        let store = CodexStore::for_rollout(dest);
        let mut owned_ids: BTreeSet<String> = BTreeSet::new();
        let mut owned_paths: Vec<PathBuf> = Vec::new();
        if let Ok(closure) = discover_closure(dest, Some(store.clone())) {
            owned_ids.extend(closure.nodes.keys().cloned());
            owned_paths.extend(closure.nodes.values().map(|node| node.path.clone()));
        }
        let pattern = store
            .sessions_dir
            .join("*/*/*/rollout-*.jsonl")
            .to_string_lossy()
            .into_owned();
        let mut hits: Vec<PathBuf> = glob::glob(&pattern)
            .map(|paths| paths.filter_map(Result::ok).collect())
            .unwrap_or_default();
        hits.sort();
        for hit in hits {
            if owned_paths.contains(&hit) {
                continue;
            }
            let Some(meta) = first_session_meta(&hit) else {
                continue;
            };
            if meta.get("id").and_then(Value::as_str) == Some(session_id) {
                owned_ids.insert(session_id.to_string());
                owned_paths.push(hit);
            }
        }
        unregister_tree(store.state_db.as_deref(), &owned_ids);
        for path in owned_paths {
            let _ = fs::remove_file(path);
        }
        Ok(())
    }
}

/// 读第一条 `session_meta` 的 payload；解析失败一律当没有。
fn first_session_meta(path: &Path) -> Option<Map<String, Value>> {
    let lines = iter_lines(path).ok()?;
    for line in lines {
        let line = line.ok()?;
        if line.trim().is_empty() {
            continue;
        }
        let record = serde_json::from_str::<Value>(&line).ok()?;
        if record.get("type").and_then(Value::as_str) == Some("session_meta") {
            return Some(
                record
                    .get("payload")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default(),
            );
        }
    }
    Some(Map::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sessions_dir(root: &Path) -> PathBuf {
        let dir = root.join(".codex").join("sessions").join("2026/07/25");
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_rollout(dir: &Path, name: &str, payload: Value) -> PathBuf {
        let path = dir.join(name);
        fs::write(
            &path,
            serde_json::to_string(&json!({"type": "session_meta", "payload": payload})).unwrap()
                + "\n",
        )
        .unwrap();
        path
    }

    #[test]
    fn resume_descriptor_uses_the_manifest_executable() {
        let lifecycle = CodexLifecycle::new("codex");
        let descriptor = lifecycle.resume_descriptor("abc", "/work").unwrap();
        assert_eq!(descriptor["args"], json!(["resume", "abc"]));
        assert_eq!(
            descriptor["display_command"],
            json!("cd /work && codex resume abc")
        );
    }

    #[test]
    fn cleanup_removes_the_whole_closure() {
        let temp = tempfile::tempdir().unwrap();
        let dir = sessions_dir(temp.path());
        let root = write_rollout(
            &dir,
            "rollout-root.jsonl",
            json!({"id": "root", "cwd": "/w"}),
        );
        let child = write_rollout(
            &dir,
            "rollout-child.jsonl",
            json!({"id": "child", "parent_thread_id": "root", "cwd": "/w"}),
        );
        let unrelated = write_rollout(
            &dir,
            "rollout-other.jsonl",
            json!({"id": "other", "cwd": "/w"}),
        );

        CodexLifecycle::new("codex").cleanup("root", &root).unwrap();
        assert!(!root.exists());
        assert!(!child.exists());
        assert!(unrelated.exists());
    }

    /// 闭包发现失败（重复 thread id）时退化成「按 id 逐个匹配」。
    #[test]
    fn cleanup_falls_back_to_id_matching_when_the_closure_is_broken() {
        let temp = tempfile::tempdir().unwrap();
        let dir = sessions_dir(temp.path());
        let root = write_rollout(
            &dir,
            "rollout-root.jsonl",
            json!({"id": "dup", "cwd": "/w"}),
        );
        let stray = write_rollout(
            &dir,
            "rollout-stray.jsonl",
            json!({"id": "dup", "cwd": "/w"}),
        );
        let unrelated = write_rollout(
            &dir,
            "rollout-other.jsonl",
            json!({"id": "other", "cwd": "/w"}),
        );

        CodexLifecycle::new("codex").cleanup("dup", &root).unwrap();
        assert!(!root.exists());
        assert!(!stray.exists());
        assert!(unrelated.exists());
    }
}
