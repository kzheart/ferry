//! Pi 文件型会话生命周期。
//!
//! 语义事实源：`engine/adapters/pi/lifecycle.py`。
//!
//! 实现 `BaseLifecycle` 即自动满足 `contracts::SessionLifecycle`；
//! 删除走 `FileSessionLifecycle` 的通用算法（`delete_file_session`），
//! pi 没有子会话文件也没有 sidecar，两个钩子都用默认值。

use std::fs;
use std::path::Path;

use serde_json::{Map, Value};

use crate::adapters::contracts::AgentAdapter;
use crate::adapters::shared::lifecycle::{
    delete_file_session, BaseLifecycle, FileSessionLifecycle,
};
use crate::errors::DomainResult;

pub struct PiLifecycle {
    executable: String,
}

impl PiLifecycle {
    /// `executable` 由 `build()` 从 manifest 白名单注入。
    pub fn new(executable: impl Into<String>) -> Self {
        Self {
            executable: executable.into(),
        }
    }
}

impl BaseLifecycle for PiLifecycle {
    fn tool(&self) -> &str {
        "pi"
    }

    fn executable(&self) -> &str {
        &self.executable
    }

    /// pi 用**绝对文件路径**恢复会话；解析不出来时原样透传，让 CLI 自己报错。
    fn resume_args(&self, session_id: &str) -> DomainResult<Vec<String>> {
        let target = super::adapter::resolve(session_id)
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|_| session_id.to_string());
        Ok(vec!["--session".to_string(), target])
    }

    /// 迁移失败时清理已写入的产物；不存在不算错。
    fn cleanup(&self, _session_id: &str, dest: &Path) -> DomainResult<()> {
        let _ = fs::remove_file(dest);
        Ok(())
    }

    fn delete(&self, adapter: &AgentAdapter, reference: &str) -> DomainResult<Map<String, Value>> {
        delete_file_session(self, adapter, reference)
    }
}

impl FileSessionLifecycle for PiLifecycle {}

#[cfg(test)]
mod tests {
    // 只 use BaseLifecycle：`SessionLifecycle` 的 blanket impl 会让同名方法歧义，
    // 需要走 trait 对象的地方直接用 adapter 上的引用。
    use super::*;
    use std::path::PathBuf;

    const FIXTURE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/agent_formats/pi/case-01-plain/session.jsonl"
    );

    fn staged(root: &Path) -> PathBuf {
        let path = root.join("session.jsonl");
        fs::copy(FIXTURE, &path).unwrap();
        path
    }

    /// 描述符形状：pi 用 `--session <文件>` 恢复。
    ///
    /// 解析成绝对路径的分支需要真实扫描根，放在 `adapter` 的环境变量测试里
    /// （那边持有进程级 `PI_CODING_AGENT_SESSION_DIR` 锁）。
    #[test]
    fn resume_descriptor_uses_the_session_flag() {
        let root = tempfile::tempdir().unwrap();
        let path = staged(root.path());
        let lifecycle = PiLifecycle::new("/usr/local/bin/pi");
        let descriptor = BaseLifecycle::resume_descriptor(
            &lifecycle,
            &path.to_string_lossy(),
            &root.path().to_string_lossy(),
        )
        .unwrap();
        assert_eq!(descriptor["tool"], Value::from("pi"));
        assert_eq!(descriptor["executable"], Value::from("/usr/local/bin/pi"));
        let args = descriptor["args"].as_array().unwrap();
        assert_eq!(args[0], Value::from("--session"));
        assert_eq!(args.len(), 2);
        assert!(descriptor["display_command"]
            .as_str()
            .unwrap()
            .contains("/usr/local/bin/pi --session "));
    }

    /// 会话不在扫描根内（解析不出来）时原样透传，让 CLI 自己报错。
    #[test]
    fn resume_falls_back_to_the_raw_reference() {
        let lifecycle = PiLifecycle::new("pi");
        let args = lifecycle.resume_args("not-a-real-session").unwrap();
        assert_eq!(args, ["--session", "not-a-real-session"]);
    }

    #[test]
    fn cleanup_is_idempotent() {
        let root = tempfile::tempdir().unwrap();
        let path = staged(root.path());
        let lifecycle = PiLifecycle::new("pi");
        BaseLifecycle::cleanup(&lifecycle, "sid", &path).unwrap();
        assert!(!path.exists());
        // 第二次不报错。
        BaseLifecycle::cleanup(&lifecycle, "sid", &path).unwrap();
    }

    #[test]
    fn delete_is_permanent_and_leaves_no_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let path = staged(root.path());
        let backups = root.path().join("backups");
        // 与 shared::lifecycle 的算法对照：editor.load → 删子会话 → 删 sidecar → unlink。
        let adapter = super::super::adapter::build().unwrap();
        let result = adapter
            .require_lifecycle("delete")
            .unwrap()
            .delete(&adapter, &path.to_string_lossy())
            .unwrap();
        assert_eq!(result["ok"], Value::Bool(true));
        assert_eq!(result["children"], Value::from(0));
        assert!(!path.exists());
        assert!(!backups.exists());
    }
}
