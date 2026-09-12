//! Pi Agent 标题写回。
//!
//! pi 的 `/name`、`--name` 与 `/resume` 里的 Ctrl+R 改名最终都是往会话 JSONL 追加一条
//! `session_info`（`appendSessionInfo`），读取时取整文件最后一条。pi 自己跨文件改名也是
//! 纯 append，所以外部追加一行与它的语义完全一致；`/resume` 列表每次重新扫文件，
//! 没有缓存，下次打开即生效。

use std::collections::HashSet;
use std::path::Path;

use serde_json::{json, Map, Value};

use crate::adapters::contracts::SessionRenamer;
use crate::adapters::shared::editing::append_jsonl_line;
use crate::errors::{DomainError, DomainResult};

use super::adapter::resolve;
use super::reader;
use super::writer::{iso_stamp, uuid4_hex};

pub struct PiRenamer;

pub fn rename_session(path: &Path, title: &str) -> DomainResult<Map<String, Value>> {
    let loaded = reader::load(path)?;
    let ids: HashSet<&str> = loaded
        .entries
        .iter()
        .filter_map(|entry| entry.get("id").and_then(Value::as_str))
        .collect();
    // 追加到末尾，parentId 指向当前叶子（对齐 pi 的 `this.leafId`）。
    let leaf = loaded
        .entries
        .iter()
        .rev()
        .find_map(|entry| entry.get("id").and_then(Value::as_str))
        .map(Value::from)
        .unwrap_or(Value::Null);
    let mut id = uuid4_hex(8);
    while ids.contains(id.as_str()) {
        id = uuid4_hex(8);
    }
    // 对齐 appendSessionInfo：换行折成空格、去首尾空白。
    let name = title.replace(['\r', '\n'], " ").trim().to_string();
    let record = json!({
        "type": "session_info",
        "id": id,
        "parentId": leaf,
        "timestamp": iso_stamp(),
        "name": name,
    });
    append_jsonl_line(path, &record).map_err(|error| {
        DomainError::internal(format!("追加 Pi 标题记录失败: {}: {error}", path.display()))
    })?;

    let mut result = Map::new();
    result.insert("title".into(), Value::from(name));
    result.insert(
        "session_id".into(),
        loaded.header.get("id").cloned().unwrap_or(Value::Null),
    );
    result.insert(
        "saved_as".into(),
        Value::from(path.to_string_lossy().into_owned()),
    );
    result.insert("record_type".into(), Value::from("session_info"));
    result.insert("entry_id".into(), record["id"].clone());
    result.insert("notes".into(), Value::Array(Vec::new()));
    Ok(result)
}

impl SessionRenamer for PiRenamer {
    fn rename(&self, reference: &str, title: &str) -> DomainResult<Map<String, Value>> {
        let path = resolve(reference)?;
        rename_session(&path, title)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::shared::scanner::iter_lines;

    fn write_session(path: &Path, records: &[Value]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let payload: String = records
            .iter()
            .map(|record| record.to_string() + "\n")
            .collect();
        std::fs::write(path, payload).unwrap();
    }

    fn base(path: &Path) {
        write_session(
            path,
            &[
                json!({"type": "session", "version": 3, "id": "sess-1",
                       "timestamp": "2026-01-01T00:00:00.000Z", "cwd": "/w"}),
                json!({"type": "message", "id": "aaaa0001", "parentId": null,
                       "timestamp": "2026-01-01T00:00:01.000Z",
                       "message": {"role": "user", "content": "hello there"}}),
                json!({"type": "session_info", "id": "aaaa0002", "parentId": "aaaa0001",
                       "timestamp": "2026-01-01T00:00:02.000Z", "name": "Old"}),
            ],
        );
    }

    #[test]
    fn rename_appends_session_info_pointing_at_the_current_leaf() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("s.jsonl");
        base(&path);
        let before = std::fs::read_to_string(&path).unwrap();

        let result = rename_session(&path, "Fresh\nname").unwrap();
        assert_eq!(result["title"], json!("Fresh name"));
        assert_eq!(result["session_id"], json!("sess-1"));

        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.starts_with(&before));
        let lines: Vec<String> = iter_lines(&path).unwrap().map(Result::unwrap).collect();
        let appended: Value = serde_json::from_str(lines.last().unwrap()).unwrap();
        assert_eq!(appended["type"], json!("session_info"));
        assert_eq!(appended["name"], json!("Fresh name"));
        assert_eq!(appended["parentId"], json!("aaaa0002"));
        let id = appended["id"].as_str().unwrap();
        assert_eq!(id.len(), 8);
        assert!(id != "aaaa0001" && id != "aaaa0002");

        let session = reader::read(path.to_str().unwrap()).unwrap();
        assert_eq!(session.title, "Fresh name");
    }

    #[test]
    fn an_empty_session_info_clears_the_name_and_falls_back_to_the_first_prompt() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("s.jsonl");
        base(&path);
        let session = reader::read(path.to_str().unwrap()).unwrap();
        assert_eq!(session.title, "Old");

        append_jsonl_line(
            &path,
            &json!({"type": "session_info", "id": "aaaa0003", "parentId": "aaaa0002",
                    "timestamp": "2026-01-01T00:00:03.000Z", "name": "   "}),
        )
        .unwrap();
        let session = reader::read(path.to_str().unwrap()).unwrap();
        assert_eq!(session.title, "hello there");
    }
}
