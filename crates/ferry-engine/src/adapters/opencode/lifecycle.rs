//! OpenCode 会话生命周期：数据库型删除（快照后经 CLI 清理，不可撤销）。
//!
//! OpenCode 不是文件型会话，所以**不能**复用 `delete_file_session`：删除必须走
//! 官方 `opencode session delete`，且要按子树自底向上逐个删。

use std::path::Path;

use serde_json::{Map, Value};

use crate::adapters::contracts::AgentAdapter;
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

    /// 永久删除：没有快照，官方 CLI 删完即不可撤销。
    fn delete(&self, _adapter: &AgentAdapter, reference: &str) -> DomainResult<Map<String, Value>> {
        self.cleanup(reference, Path::new(""))?;
        let mut result = Map::new();
        result.insert("ok".into(), Value::Bool(true));
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::DomainError;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct RecordingCli {
        deleted: Mutex<Vec<String>>,
    }

    impl store::NativeCli for RecordingCli {
        fn run_command(&self, _args: &[&str], _cwd: Option<&Path>) -> DomainResult<String> {
            Ok(String::new())
        }
        fn export_session(&self, _session_id: &str) -> DomainResult<Value> {
            Err(DomainError::internal("不该走 CLI 导出"))
        }
        fn import_payload(&self, _: &Value, _: &str, _: &str) -> DomainResult<()> {
            Ok(())
        }
        fn delete_session(&self, session_id: &str, _cwd: Option<&str>) -> DomainResult<()> {
            self.deleted.lock().unwrap().push(session_id.to_string());
            Ok(())
        }
    }

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

    #[test]
    fn delete_falls_back_to_the_single_id_when_the_tree_is_unreadable() {
        let _guard = store::tests::exclusive();
        let cli = Arc::new(RecordingCli::default());
        store::install_cli(cli.clone());
        // 库不存在 → reader::read 失败 → 只删这一个 id。
        store::set_database_path_override(Some(std::path::PathBuf::from(
            "/nonexistent/ferry-opencode.db",
        )));
        let lifecycle = OpenCodeLifecycle::new("opencode");
        let adapter = super::super::adapter::build().expect("adapter 可装配");
        let result = lifecycle.delete(&adapter, "ses_1").unwrap();
        store::set_database_path_override(None);
        store::reset_cli();
        assert_eq!(result["ok"], Value::Bool(true));
        assert_eq!(*cli.deleted.lock().unwrap(), ["ses_1"]);
    }
}
