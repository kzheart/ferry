//! OpenCode 会话生命周期：resume 与迁移清理。

use std::path::Path;

#[cfg(test)]
use serde_json::Value;

use crate::adapters::shared::lifecycle::BaseLifecycle;
use crate::errors::DomainResult;

use super::{reader, store};

/// OpenCode 的生命周期策略。
pub struct OpenCodeLifecycle {
    executable: String,
}

impl OpenCodeLifecycle {
    /// `executable` 由 `build()` 从 manifest 的可执行白名单注入。
    pub fn new(executable: impl Into<String>) -> Self {
        Self {
            executable: executable.into(),
        }
    }

    /// 会话子树的删除顺序：先叶子后根（`reversed(walk())`）。
    ///
    /// 读不到子树时退化成「只删这一个 id」——删除本身是尽力而为，不因读失败中断。
    fn delete_ids(session_id: &str) -> Vec<String> {
        match reader::read(session_id) {
            Ok(tree) => {
                let mut ids: Vec<String> = tree
                    .walk()
                    .iter()
                    .map(|node| node.source_id.clone())
                    .collect();
                ids.reverse();
                ids
            }
            Err(_) => vec![session_id.to_string()],
        }
    }
}

impl BaseLifecycle for OpenCodeLifecycle {
    fn tool(&self) -> &str {
        "opencode"
    }

    fn executable(&self) -> &str {
        &self.executable
    }

    fn resume_args(&self, session_id: &str) -> DomainResult<Vec<String>> {
        Ok(vec!["-s".into(), session_id.to_string()])
    }

    /// 迁移失败清理：逐个删掉刚写进去的会话；单条失败不中断其余删除。
    fn cleanup(&self, session_id: &str, _dest: &Path) -> DomainResult<()> {
        for id in Self::delete_ids(session_id) {
            let _ = store::delete_session(&id, None);
        }
        Ok(())
    }

    /// OpenCode 的会话引用就是原生 id，不落文件。
    fn validation_ref(&self, session_id: &str, _dest: &Path) -> DomainResult<String> {
        Ok(session_id.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_descriptor_uses_the_session_flag() {
        let lifecycle = OpenCodeLifecycle::new("/usr/local/bin/opencode");
        let descriptor = lifecycle.resume_descriptor("ses_1", "/work").unwrap();
        assert_eq!(
            descriptor["display_command"],
            Value::from("cd /work && /usr/local/bin/opencode -s ses_1")
        );
        assert_eq!(descriptor["args"], serde_json::json!(["-s", "ses_1"]));
        assert_eq!(descriptor["tool"], Value::from("opencode"));
    }

    #[test]
    fn validation_ref_is_the_native_id_not_a_path() {
        let lifecycle = OpenCodeLifecycle::new("opencode");
        assert_eq!(
            lifecycle
                .validation_ref("ses_1", Path::new("/tmp/whatever.db"))
                .unwrap(),
            "ses_1"
        );
    }
}
