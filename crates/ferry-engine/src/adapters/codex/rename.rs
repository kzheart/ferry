//! Codex 标题写回。
//!
//! 主路径走 app-server 的 `thread/name/set`：由 Codex 自己写 `threads.name`、追加
//! `session_index.jsonl` 并向所有已连接客户端广播 `thread/name/updated`，Desktop / TUI
//! 立即刷新。daemon 不在跑（或协议不兼容）时降级为直写注册库 + 追加索引：数据落地
//! 一致，但运行中的 Codex 进程要重启才看得到，用 `notes` 告知。

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use rusqlite::Connection;
use serde_json::{json, Map, Value};

use crate::adapters::claude::editing::utc_iso_now_micros;
use crate::adapters::contracts::SessionRenamer;
use crate::adapters::shared::editing::append_jsonl_line;
use crate::errors::{DomainError, DomainResult};
use crate::system::executables;

use super::editor::resolve;
use super::native::{table_columns, CodexStore};

pub struct CodexRenamer;

const RPC_TIMEOUT: Duration = Duration::from_secs(6);
pub const RESTART_NOTE: &str = "Codex 正在运行时需重启后才会显示新标题";

/// `rollout-<时间戳>-<uuid>.jsonl` → `<uuid>`。
pub(super) fn thread_id_of(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let candidate = stem.get(stem.len().checked_sub(36)?..)?;
    let shaped = candidate.chars().enumerate().all(|(index, character)| {
        if matches!(index, 8 | 13 | 18 | 23) {
            character == '-'
        } else {
            character.is_ascii_hexdigit()
        }
    });
    shaped.then(|| candidate.to_string())
}

fn line_reader(stdout: std::process::ChildStdout) -> mpsc::Receiver<Value> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(value) = serde_json::from_str::<Value>(&line) {
                if sender.send(value).is_err() {
                    break;
                }
            }
        }
    });
    receiver
}

fn wait_response(
    receiver: &mpsc::Receiver<Value>,
    id: i64,
    deadline: Instant,
) -> Result<Value, String> {
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| "app-server 响应超时".to_string())?;
        let message = receiver
            .recv_timeout(remaining)
            .map_err(|_| "app-server 响应超时或连接关闭".to_string())?;
        if message.get("id").and_then(Value::as_i64) != Some(id) {
            continue;
        }
        if let Some(error) = message.get("error") {
            return Err(format!("app-server 拒绝: {error}"));
        }
        return Ok(message.get("result").cloned().unwrap_or(Value::Null));
    }
}

/// 通过 `codex app-server proxy` 调 `thread/name/set`。
fn rename_via_app_server(thread_id: &str, title: &str) -> Result<(), String> {
    let executable = executables::resolve("codex").ok_or("找不到 codex 可执行文件")?;
    let mut child = Command::new(executable)
        .args(["app-server", "proxy"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("无法启动 codex app-server proxy: {error}"))?;
    let outcome = (|| -> Result<(), String> {
        let mut stdin = child.stdin.take().ok_or("proxy stdin 不可用")?;
        let stdout = child.stdout.take().ok_or("proxy stdout 不可用")?;
        let receiver = line_reader(stdout);
        let deadline = Instant::now() + RPC_TIMEOUT;
        let mut send = |message: Value| -> Result<(), String> {
            stdin
                .write_all(format!("{message}\n").as_bytes())
                .and_then(|()| stdin.flush())
                .map_err(|error| format!("写入 proxy 失败: {error}"))
        };
        send(json!({
            "id": 1,
            "method": "initialize",
            "params": {"clientInfo": {
                "name": "ferry",
                "title": "Ferry",
                "version": env!("CARGO_PKG_VERSION"),
            }},
        }))?;
        wait_response(&receiver, 1, deadline)?;
        send(json!({"method": "initialized", "params": {}}))?;
        send(json!({
            "id": 2,
            "method": "thread/name/set",
            "params": {"threadId": thread_id, "name": title},
        }))?;
        wait_response(&receiver, 2, deadline)?;
        Ok(())
    })();
    let _ = child.kill();
    let _ = child.wait();
    outcome
}

/// 直写注册库 `threads.name`，并往 `session_index.jsonl` 追加一行反查索引。
pub(super) fn rename_in_store(
    store: &CodexStore,
    thread_id: &str,
    title: &str,
) -> DomainResult<()> {
    let state_db = store.state_db.as_deref().ok_or_else(|| {
        DomainError::session_store_unavailable("codex", "找不到 state_5.sqlite 注册库")
    })?;
    let connection = Connection::open(state_db).map_err(|error| {
        DomainError::session_store_unavailable("codex", &format!("注册库打开失败: {error}"))
    })?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|error| DomainError::internal(format!("注册库设置超时失败: {error}")))?;
    if !table_columns(&connection, "threads")
        .iter()
        .any(|column| column == "name")
    {
        return Err(DomainError::agent_format_changed(
            "codex",
            "state_5.sqlite threads.name",
            Value::from("name column"),
            Value::Null,
        ));
    }
    let changed = connection
        .execute(
            "UPDATE threads SET name = ?1 WHERE id = ?2",
            (title, thread_id),
        )
        .map_err(|error| DomainError::internal(format!("写入 Codex 标题失败: {error}")))?;
    if changed == 0 {
        return Err(DomainError::session_not_found("codex", thread_id));
    }
    let index = store.home.join("session_index.jsonl");
    if !index.exists() {
        std::fs::write(&index, b"").map_err(|error| {
            DomainError::internal(format!("创建 session_index.jsonl 失败: {error}"))
        })?;
    }
    append_jsonl_line(
        &index,
        &json!({"id": thread_id, "thread_name": title, "updated_at": utc_iso_now_micros()}),
    )
    .map_err(|error| DomainError::internal(format!("追加 session_index.jsonl 失败: {error}")))?;
    Ok(())
}

pub fn rename_rollout(path: &Path, title: &str) -> DomainResult<Map<String, Value>> {
    let store = CodexStore::for_rollout(path);
    let thread_id = thread_id_of(path)
        .ok_or_else(|| DomainError::internal("Codex rollout 文件名不含线程 id"))?;
    let mut notes: Vec<Value> = Vec::new();
    let via = match rename_via_app_server(&thread_id, title) {
        Ok(()) => "app-server",
        Err(reason) => {
            rename_in_store(&store, &thread_id, title)?;
            notes.push(Value::from(format!(
                "{RESTART_NOTE}（app-server 不可用: {reason}）"
            )));
            "state-db"
        }
    };
    let mut result = Map::new();
    result.insert("title".into(), Value::from(title));
    result.insert("session_id".into(), Value::from(thread_id));
    result.insert("via".into(), Value::from(via));
    result.insert(
        "saved_as".into(),
        Value::from(
            store
                .state_db
                .as_deref()
                .map(|db| db.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ),
    );
    result.insert("notes".into(), Value::Array(notes));
    Ok(result)
}

impl SessionRenamer for CodexRenamer {
    fn rename(&self, reference: &str, title: &str) -> DomainResult<Map<String, Value>> {
        let path = resolve(reference)?;
        rename_rollout(&path, title)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::codex::titles::{tests::seed_database, thread_names};
    use std::path::PathBuf;

    #[test]
    fn the_thread_id_is_the_uuid_suffix_of_the_rollout_name() {
        let path = PathBuf::from(
            "/x/sessions/2026/09/12/rollout-2026-09-12T01-28-06-01a09183-24a3-7a23-be23-ebd2d05a416f.jsonl",
        );
        assert_eq!(
            thread_id_of(&path).as_deref(),
            Some("01a09183-24a3-7a23-be23-ebd2d05a416f")
        );
        assert_eq!(thread_id_of(Path::new("/x/rollout-short.jsonl")), None);
    }

    #[test]
    fn store_rename_updates_the_name_column_and_appends_the_index() {
        let root = tempfile::tempdir().unwrap();
        let db = root.path().join("state_5.sqlite");
        seed_database(&db, &[("t1", Some("Old"))]);
        let store = CodexStore {
            home: root.path().to_path_buf(),
            sessions_dir: root.path().join("sessions"),
            state_db: Some(db),
        };
        rename_in_store(&store, "t1", "Brand New").unwrap();
        assert_eq!(
            thread_names(&store).get("t1").map(String::as_str),
            Some("Brand New")
        );
        let index = std::fs::read_to_string(root.path().join("session_index.jsonl")).unwrap();
        let line: Value = serde_json::from_str(index.trim()).unwrap();
        assert_eq!(line["id"], json!("t1"));
        assert_eq!(line["thread_name"], json!("Brand New"));
        assert!(line["updated_at"].as_str().unwrap().ends_with('Z'));

        let missing = rename_in_store(&store, "nope", "x").unwrap_err();
        assert_eq!(missing.code, "session.not_found");
    }

    #[test]
    fn store_rename_refuses_a_registry_without_a_name_column() {
        let root = tempfile::tempdir().unwrap();
        let db = root.path().join("state_5.sqlite");
        Connection::open(&db)
            .unwrap()
            .execute_batch("CREATE TABLE threads (id TEXT PRIMARY KEY, title TEXT NOT NULL)")
            .unwrap();
        let store = CodexStore {
            home: root.path().to_path_buf(),
            sessions_dir: root.path().join("sessions"),
            state_db: Some(db),
        };
        let error = rename_in_store(&store, "t1", "x").unwrap_err();
        assert_eq!(error.code, "agent.format_changed");
    }
}
