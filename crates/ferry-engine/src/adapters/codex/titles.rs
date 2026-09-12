//! Codex 会话标题的原生来源。
//!
//! rollout JSONL 里**没有**标题字段。Codex 界面显示的名字是 `~/.codex/state_5.sqlite`
//! `threads.name` 列（Desktop 自动生成或 TUI `/rename`），空时退回 `preview`（首条
//! 用户消息）。`threads.title` 是首条用户消息全文的 legacy 列，不是显示标题。
//! `~/.codex/session_index.jsonl` 是 `name → id` 的追加式反查索引（`codex resume
//! <name>` 用），后写覆盖先写，可在注册库缺席时兜底。

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use crate::adapters::shared::scanner::{clip_text_default, iter_lines};
use crate::model::{BlockKind, Session};

use super::native::{table_columns, CodexStore};

/// 只读打开注册库；打不开、没有 `threads.name` 列都视为「没有原生标题」。
fn open_readonly(state_db: &Path) -> Option<Connection> {
    let connection = Connection::open_with_flags(
        state_db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    connection.busy_timeout(Duration::from_secs(5)).ok()?;
    if !table_columns(&connection, "threads")
        .iter()
        .any(|column| column == "name")
    {
        return None;
    }
    Some(connection)
}

fn names_from_database(state_db: &Path) -> Option<HashMap<String, String>> {
    let connection = open_readonly(state_db)?;
    let mut statement = connection
        .prepare("SELECT id, name FROM threads WHERE name IS NOT NULL AND name != ''")
        .ok()?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .ok()?;
    let mut names = HashMap::new();
    for (id, name) in rows.flatten() {
        let name = name.trim();
        if !name.is_empty() {
            names.insert(id, name.to_string());
        }
    }
    Some(names)
}

/// `session_index.jsonl`：`{"id","thread_name","updated_at"}` 每行一条，后写覆盖先写。
fn names_from_index(home: &Path) -> HashMap<String, String> {
    let mut names = HashMap::new();
    let Ok(lines) = iter_lines(&home.join("session_index.jsonl")) else {
        return names;
    };
    for line in lines.map_while(Result::ok) {
        let Ok(record) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let id = record.get("id").and_then(Value::as_str).unwrap_or_default();
        let name = record
            .get("thread_name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        if id.is_empty() {
            continue;
        }
        if name.is_empty() {
            names.remove(id);
        } else {
            names.insert(id.to_string(), name.to_string());
        }
    }
    names
}

/// 全部有名字的线程：注册库优先，缺席时读追加索引。
pub fn thread_names(store: &CodexStore) -> HashMap<String, String> {
    store
        .state_db
        .as_deref()
        .and_then(names_from_database)
        .unwrap_or_else(|| names_from_index(&store.home))
}

/// 与 scanner 同一口径的首句回退：首条非环境上下文（`<`/`[` 开头）的用户文本截 80 字。
pub fn derived_title(session: &Session) -> String {
    for message in &session.messages {
        if message.role != "user" {
            continue;
        }
        let text = message
            .blocks
            .iter()
            .filter(|block| block.kind == BlockKind::Text)
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let trimmed = text.trim();
        if trimmed.is_empty() || trimmed.starts_with(['<', '[']) {
            continue;
        }
        return clip_text_default(&text);
    }
    String::new()
}

/// 给整棵会话树填标题：有原生名字用名字，否则首句回退。
pub fn apply_titles(session: &mut Session, names: &HashMap<String, String>) {
    session.title = names
        .get(&session.source_id)
        .cloned()
        .unwrap_or_else(|| derived_title(session));
    for child in &mut session.children {
        apply_titles(child, names);
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::model::{Block, Message};
    use std::path::PathBuf;

    fn store(root: &Path) -> CodexStore {
        CodexStore {
            home: root.to_path_buf(),
            sessions_dir: root.join("sessions"),
            state_db: Some(root.join("state_5.sqlite")).filter(|path| path.exists()),
        }
    }

    pub(crate) fn seed_database(path: &Path, rows: &[(&str, Option<&str>)]) {
        let connection = Connection::open(path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT NOT NULL,
                 title TEXT NOT NULL, name TEXT, preview TEXT NOT NULL DEFAULT '')",
            )
            .unwrap();
        for (id, name) in rows {
            connection
                .execute(
                    "INSERT INTO threads (id, rollout_path, title, name) VALUES (?, ?, ?, ?)",
                    (id, "/r", "first prompt", name),
                )
                .unwrap();
        }
    }

    #[test]
    fn database_names_win_and_blank_names_are_dropped() {
        let root = tempfile::tempdir().unwrap();
        seed_database(
            &root.path().join("state_5.sqlite"),
            &[("a", Some("  Named A ")), ("b", Some("")), ("c", None)],
        );
        std::fs::write(
            root.path().join("session_index.jsonl"),
            "{\"id\":\"c\",\"thread_name\":\"From Index\"}\n",
        )
        .unwrap();
        let names = thread_names(&store(root.path()));
        assert_eq!(names.get("a").map(String::as_str), Some("Named A"));
        assert!(!names.contains_key("b"));
        // 注册库存在时不看索引文件。
        assert!(!names.contains_key("c"));
    }

    #[test]
    fn the_index_file_is_the_fallback_and_later_lines_override() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("session_index.jsonl"),
            "{\"id\":\"x\",\"thread_name\":\"Old\"}\n{\"id\":\"x\",\"thread_name\":\"New\"}\n{oops}\n",
        )
        .unwrap();
        let names = thread_names(&store(root.path()));
        assert_eq!(names.get("x").map(String::as_str), Some("New"));
    }

    #[test]
    fn titles_fall_back_to_the_first_plain_user_message_per_node() {
        let mut session = Session::new("codex", "root", "/w");
        let mut env = Message::new("user");
        env.blocks.push(Block::text("<environment_context/>"));
        let mut ask = Message::new("user");
        ask.blocks.push(Block::text("  please   help "));
        session.messages = vec![env, ask];
        let mut child = Session::new("codex", "kid", "/w");
        child.messages.push({
            let mut message = Message::new("user");
            message.blocks.push(Block::text("child ask"));
            message
        });
        session.children.push(child);

        let mut names = HashMap::new();
        names.insert("kid".to_string(), "Reviewer".to_string());
        apply_titles(&mut session, &names);
        assert_eq!(session.title, "please help");
        assert_eq!(session.children[0].title, "Reviewer");
        let _ = PathBuf::new();
    }
}
