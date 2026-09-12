//! Claude Code 标题写回。
//!
//! Claude Code 读标题时扫文件尾部的 `custom-title` 记录（用户 `/rename` 写的同一种
//! 记录），优先级高于 `ai-title` 与首句。所以只需往会话 JSONL **追加**一条
//! `custom-title`，桌面端与 `claude --resume` 列表下次打开就显示新标题；正在运行的
//! 会话不受影响，因为 Claude Code 自己也是纯追加写，从不重写整个文件。
//!
//! 不能复用编辑事务的全量重写落盘：rename 覆盖会让正在写的 Claude 进程继续往孤儿
//! inode 写，后续消息全部丢失。

use std::path::Path;

use serde_json::{json, Map, Value};

use crate::adapters::contracts::SessionRenamer;
use crate::adapters::shared::editing::append_jsonl_line;
use crate::errors::{DomainError, DomainResult};

use super::editing as claude_edit;

pub struct ClaudeRenamer;

/// `~/.claude/projects/<slug>/<sid>.jsonl` → `<sid>`；Claude 的 sessionId 就是文件名。
fn session_id_of(path: &Path) -> DomainResult<String> {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .map(str::to_string)
        .ok_or_else(|| DomainError::internal("Claude 会话文件名不含 session id"))
}

/// `<dir>/<sid>/custom-title.json`：新版 Claude Code 的标题 sidecar。尾部记录优先于它，
/// 不写也正确；但它已存在时同步改掉，避免将来清标题时旧值从 sidecar 复活。
fn refresh_sidecar(path: &Path, title: &str) -> Option<String> {
    let sidecar = path.with_extension("").join("custom-title.json");
    if !sidecar.exists() {
        return None;
    }
    let payload = json!({"customTitle": title}).to_string();
    std::fs::write(&sidecar, payload).ok()?;
    Some(sidecar.to_string_lossy().into_owned())
}

pub fn rename_session(path: &Path, title: &str) -> DomainResult<Map<String, Value>> {
    if !path.is_file() {
        return Err(DomainError::session_not_found(
            "claude",
            &path.to_string_lossy(),
        ));
    }
    let session_id = session_id_of(path)?;
    let record = json!({
        "type": "custom-title",
        "customTitle": title,
        "sessionId": session_id,
        "uuid": claude_edit::uuid4(),
        "timestamp": claude_edit::utc_iso_now_micros(),
    });
    append_jsonl_line(path, &record).map_err(|error| {
        DomainError::internal(format!(
            "追加 Claude 标题记录失败: {}: {error}",
            path.display()
        ))
    })?;
    let sidecar = refresh_sidecar(path, title);

    let mut result = Map::new();
    result.insert("title".into(), Value::from(title));
    result.insert("session_id".into(), Value::from(session_id));
    result.insert(
        "saved_as".into(),
        Value::from(path.to_string_lossy().into_owned()),
    );
    result.insert("record_type".into(), Value::from("custom-title"));
    if let Some(sidecar) = sidecar {
        result.insert("sidecar".into(), Value::from(sidecar));
    }
    result.insert("notes".into(), Value::Array(Vec::new()));
    Ok(result)
}

impl SessionRenamer for ClaudeRenamer {
    fn rename(&self, reference: &str, title: &str) -> DomainResult<Map<String, Value>> {
        let path = claude_edit::resolve(reference)?;
        rename_session(&path, title)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::claude::reader;
    use crate::adapters::shared::scanner::iter_lines;

    fn write_session(path: &Path, records: &[Value]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let payload: String = records
            .iter()
            .map(|record| record.to_string() + "\n")
            .collect();
        std::fs::write(path, payload).unwrap();
    }

    #[test]
    fn rename_appends_a_custom_title_record_and_keeps_existing_lines() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("slug/abc-123.jsonl");
        write_session(
            &path,
            &[
                json!({"type": "custom-title", "customTitle": "Old", "sessionId": "abc-123"}),
                json!({"uuid": "u1", "type": "user", "sessionId": "abc-123",
                       "message": {"role": "user", "content": "hi"}}),
            ],
        );
        let before = std::fs::read_to_string(&path).unwrap();

        let result = rename_session(&path, "New Title").unwrap();
        assert_eq!(result["title"], json!("New Title"));
        assert_eq!(result["session_id"], json!("abc-123"));

        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.starts_with(&before), "原有行必须原样保留");
        let lines: Vec<String> = iter_lines(&path).unwrap().map(Result::unwrap).collect();
        assert_eq!(lines.len(), 3);
        let appended: Value = serde_json::from_str(&lines[2]).unwrap();
        assert_eq!(appended["type"], json!("custom-title"));
        assert_eq!(appended["customTitle"], json!("New Title"));
        assert_eq!(appended["sessionId"], json!("abc-123"));
        assert!(appended["uuid"]
            .as_str()
            .is_some_and(|value| value.len() == 36));
        assert!(appended["timestamp"]
            .as_str()
            .is_some_and(|value| value.ends_with('Z')));

        // reader 读回的就是新标题（尾部 custom-title 最后一条生效）。
        let session = reader::read(path.to_str().unwrap()).unwrap();
        assert_eq!(session.title, "New Title");
    }

    #[test]
    fn rename_refreshes_an_existing_sidecar_only() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("slug/sid.jsonl");
        write_session(
            &path,
            &[json!({"uuid": "u1", "type": "user", "sessionId": "sid",
                     "message": {"role": "user", "content": "hi"}})],
        );
        let result = rename_session(&path, "T1").unwrap();
        assert!(result.get("sidecar").is_none());
        assert!(!root.path().join("slug/sid/custom-title.json").exists());

        let sidecar = root.path().join("slug/sid/custom-title.json");
        std::fs::create_dir_all(sidecar.parent().unwrap()).unwrap();
        std::fs::write(&sidecar, r#"{"customTitle":"stale"}"#).unwrap();
        let result = rename_session(&path, "T2").unwrap();
        assert!(result.get("sidecar").is_some());
        let payload: Value =
            serde_json::from_str(&std::fs::read_to_string(&sidecar).unwrap()).unwrap();
        assert_eq!(payload["customTitle"], json!("T2"));
    }

    #[test]
    fn rename_rejects_a_missing_file() {
        let root = tempfile::tempdir().unwrap();
        let error = rename_session(&root.path().join("nope.jsonl"), "x").unwrap_err();
        assert_eq!(error.code, "session.not_found");
    }
}
