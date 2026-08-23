//! Cursor 的接续与迁移回滚。
//!
//! Cursor 是 IDE 而不是 CLI Agent：它没有「按会话 id 接续」的命令行入口，会话只能
//! 在打开对应工作区之后从聊天历史里选。因此 resume 描述符落成 `cursor <cwd>`——
//! 把用户送到那个工作区，迁入的会话就排在聊天列表最前（写入时 `recency` 取的是
//! 写入时刻）。session_id 不进命令，但仍原样出现在描述符里供 UI 展示。
//!
//! 迁移链路要求目标 adapter 具备 lifecycle（`operations::migrate` 用它做写后验收的
//! 引用解析与失败回滚），这也是 cursor 声明 `resume` 能力的原因。

use std::path::Path;

use crate::adapters::shared::lifecycle::BaseLifecycle;
use crate::errors::DomainResult;

use super::writer;

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

    /// 迁移失败的回滚：删掉刚写进去的那条会话及其子代理子树。
    fn cleanup(&self, session_id: &str, _dest: &Path) -> DomainResult<()> {
        writer::delete_composer_tree(session_id)
    }

    /// Cursor 会话不落文件：验收引用就是原生 composerId。
    fn validation_ref(&self, session_id: &str, _dest: &Path) -> DomainResult<String> {
        Ok(session_id.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::cursor::store;
    use crate::adapters::cursor::store::tests::{exclusive, materialize};
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

    #[test]
    fn cleanup_removes_the_composer_and_its_subagents_only() {
        let _guard = exclusive();
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("state.vscdb");
        materialize(
            &database,
            &json!({"sessions": [
                {"id": "root-1", "header": {"createdAt": 1},
                 "composerData": {"_v": 17, "fullConversationHeadersOnly": [
                     {"bubbleId": "b1"}, {"bubbleId": "b2"}]},
                 "bubbles": {"b1": {"_v": 3, "type": 1, "text": "q"},
                             "b2": {"_v": 3, "type": 2, "text": "a"}}},
                {"id": "sub-1", "subagent": true,
                 "header": {"createdAt": 2, "subagentInfo": {"parentComposerId": "root-1"}},
                 "composerData": {"_v": 17,
                     "fullConversationHeadersOnly": [{"bubbleId": "s1"}]},
                 "bubbles": {"s1": {"_v": 3, "type": 2, "text": "sub"}}},
                {"id": "other", "header": {"createdAt": 3},
                 "composerData": {"_v": 17,
                     "fullConversationHeadersOnly": [{"bubbleId": "o1"}]},
                 "bubbles": {"o1": {"_v": 3, "type": 2, "text": "keep"}}},
            ]}),
        );
        store::set_database_path_override(Some(database.clone()));
        CursorLifecycle::new("cursor")
            .cleanup("root-1", Path::new(""))
            .unwrap();

        let connection = store::open_readonly(&database).unwrap();
        let remaining: i64 = connection
            .query_row("SELECT COUNT(*) FROM composerHeaders", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, 1);
        for key in [
            "composerData:root-1",
            "composerData:sub-1",
            "bubbleId:root-1:b1",
            "bubbleId:root-1:b2",
            "bubbleId:sub-1:s1",
        ] {
            assert!(store::disk_kv(&connection, key).unwrap().is_none(), "{key}");
        }
        // 邻居会话一条都不能少。
        assert!(store::disk_kv(&connection, "composerData:other")
            .unwrap()
            .is_some());
        assert!(store::disk_kv(&connection, "bubbleId:other:o1")
            .unwrap()
            .is_some());
        drop(connection);
        store::set_database_path_override(None);
    }
}
