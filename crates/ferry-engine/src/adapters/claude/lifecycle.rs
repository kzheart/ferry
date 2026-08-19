//! Claude 会话生命周期：resume、迁移清理、永久删除与 sidecar 归档策略。

use std::path::Path;

use serde_json::{Map, Value};

use crate::adapters::contracts::AgentAdapter;
use crate::adapters::shared::lifecycle::{
    delete_file_session, BaseLifecycle, FileSessionLifecycle,
};
use crate::errors::DomainResult;
use crate::system::paths::home_dir;

pub struct ClaudeLifecycle {
    /// 装配时由 adapter 从 manifest executables 注入。
    executable: String,
}

impl ClaudeLifecycle {
    pub fn new(executable: impl Into<String>) -> Self {
        Self {
            executable: executable.into(),
        }
    }
}

impl BaseLifecycle for ClaudeLifecycle {
    fn tool(&self) -> &str {
        "claude"
    }

    fn executable(&self) -> &str {
        &self.executable
    }

    fn resume_args(&self, session_id: &str) -> DomainResult<Vec<String>> {
        Ok(vec!["--resume".to_string(), session_id.to_string()])
    }

    /// 迁移回滚：删掉刚写出的会话文件与它的 sidecar 目录。
    fn cleanup(&self, session_id: &str, _dest: &Path) -> DomainResult<()> {
        let pattern = home_dir().join(format!(".claude/projects/*/{session_id}.jsonl"));
        let Ok(hits) = glob::glob(&pattern.to_string_lossy()) else {
            return Ok(());
        };
        for hit in hits.filter_map(Result::ok) {
            let _ = std::fs::remove_file(&hit);
            let _ = std::fs::remove_dir_all(hit.with_extension(""));
        }
        Ok(())
    }

    fn delete(&self, adapter: &AgentAdapter, reference: &str) -> DomainResult<Map<String, Value>> {
        delete_file_session(self, adapter, reference)
    }
}

impl FileSessionLifecycle for ClaudeLifecycle {
    /// subagents 与 journals 都挂在同名的无后缀目录下。
    fn delete_sidecar(&self, path: &Path) -> DomainResult<()> {
        let sidecar = path.with_extension("");
        if sidecar.is_dir() {
            let _ = std::fs::remove_dir_all(sidecar);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::claude::editing::testing::home_guard;

    #[test]
    fn resume_uses_the_resume_flag() {
        let lifecycle = ClaudeLifecycle::new("/usr/local/bin/claude");
        assert_eq!(lifecycle.resume_args("abc").unwrap(), ["--resume", "abc"]);
        let descriptor = lifecycle.resume_descriptor("abc", "/work").unwrap();
        assert_eq!(
            descriptor.keys().map(String::as_str).collect::<Vec<_>>(),
            [
                "tool",
                "session_id",
                "cwd",
                "executable",
                "args",
                "display_command"
            ]
        );
        assert_eq!(descriptor["tool"], Value::from("claude"));
        assert_eq!(
            descriptor["display_command"],
            Value::from("cd /work && /usr/local/bin/claude --resume abc")
        );
    }

    #[test]
    fn cleanup_removes_the_session_and_its_sidecar() {
        let home = tempfile::tempdir().unwrap();
        let projects = home.path().join(".claude/projects/slug");
        std::fs::create_dir_all(&projects).unwrap();
        let session = projects.join("sid.jsonl");
        std::fs::write(&session, "{}\n").unwrap();
        let sidecar = projects.join("sid/subagents");
        std::fs::create_dir_all(&sidecar).unwrap();
        std::fs::write(sidecar.join("agent-a.jsonl"), "{}\n").unwrap();

        {
            let _home = home_guard(home.path());
            ClaudeLifecycle::new("claude")
                .cleanup("sid", Path::new("/unused"))
                .unwrap();
        }
        assert!(!session.exists());
        assert!(!projects.join("sid").exists());
    }

    #[test]
    fn delete_sidecar_is_a_noop_without_a_directory() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("s.jsonl");
        std::fs::write(&path, "{}\n").unwrap();
        ClaudeLifecycle::new("claude")
            .delete_sidecar(&path)
            .unwrap();
        assert!(path.exists());
    }
}
