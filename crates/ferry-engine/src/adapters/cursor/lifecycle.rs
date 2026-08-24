//! Cursor 的接续。
//!
//! Cursor 是 IDE 而不是 CLI Agent：它没有「按会话 id 接续」的命令行入口，会话只能
//! 在打开对应工作区之后从聊天历史里选。因此 resume 描述符落成 `cursor <cwd>`——
//! 把用户送到那个工作区。session_id 不进命令，但仍原样出现在描述符里供 UI 展示。
//!
//! Cursor 不再是迁移目标，因此没有写后验收与失败回滚路径。

use std::path::Path;

use crate::adapters::shared::lifecycle::BaseLifecycle;
use crate::errors::DomainResult;

pub struct CursorLifecycle {
    executable: String,
}

impl CursorLifecycle {
    pub fn new(executable: impl Into<String>) -> Self {
        Self {
            executable: executable.into(),
        }
    }
}

impl BaseLifecycle for CursorLifecycle {
    fn tool(&self) -> &str {
        "cursor"
    }

    fn executable(&self) -> &str {
        &self.executable
    }

    /// `cursor .`：在当前工作目录打开 Cursor。
    ///
    /// 会话 id 无法进命令行（Cursor 没有这个入口），所以这里不拼它——拼一个 Cursor
    /// 会当成路径的参数，只会让它打开一个不存在的文件夹。
    fn resume_args(&self, _session_id: &str) -> DomainResult<Vec<String>> {
        Ok(vec![".".to_string()])
    }

    /// Cursor 会话不落文件：验收引用就是原生 composerId。
    fn validation_ref(&self, session_id: &str, _dest: &Path) -> DomainResult<String> {
        Ok(session_id.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn resume_opens_the_workspace_instead_of_a_session_id() {
        let lifecycle = CursorLifecycle::new("/usr/local/bin/cursor");
        let descriptor = lifecycle.resume_descriptor("c-1", "/work").unwrap();
        assert_eq!(descriptor["tool"], json!("cursor"));
        assert_eq!(descriptor["session_id"], json!("c-1"));
        assert_eq!(descriptor["args"], json!(["."]));
        assert_eq!(
            descriptor["display_command"],
            json!("cd /work && /usr/local/bin/cursor .")
        );
    }

    #[test]
    fn validation_uses_the_composer_id_not_the_database_path() {
        let lifecycle = CursorLifecycle::new("cursor");
        assert_eq!(
            lifecycle
                .validation_ref("c-1", Path::new("/tmp/state.vscdb"))
                .unwrap(),
            "c-1"
        );
    }
}
