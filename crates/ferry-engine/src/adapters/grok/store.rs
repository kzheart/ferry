//! Grok bundle 的安全装载与指纹。
//!
//! bundle 是一个目录：`summary.json`（必需）+ `updates.jsonl`（权威历史）
//! + `chat_history.jsonl`（仅在没有 updates 时作为回退）。

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::adapters::shared::scanner::split_jsonl_lines;
use crate::errors::{DomainError, DomainResult};

/// 一个已校验的 Grok 会话目录。
#[derive(Clone, Debug)]
pub struct GrokBundle {
    pub path: PathBuf,
    pub summary: Value,
    pub updates: Vec<Value>,
    pub chat: Vec<Value>,
    pub diagnostics: Vec<Map<String, Value>>,
}

impl GrokBundle {
    /// 权威成员：summary.json + （updates.jsonl 若在，否则 chat_history.jsonl）。
    pub fn authoritative_members(&self) -> Vec<PathBuf> {
        vec![
            self.path.join("summary.json"),
            authoritative_history(&self.path),
        ]
    }

    /// `summary["info"]`；`load_grok_bundle` 已保证它是带 id/cwd 的对象。
    pub fn info(&self) -> &Map<String, Value> {
        self.summary
            .get("info")
            .and_then(Value::as_object)
            .expect("load_grok_bundle 已校验 info 结构")
    }

    pub fn session_id(&self) -> &str {
        self.info()
            .get("id")
            .and_then(Value::as_str)
            .expect("load_grok_bundle 已校验 info.id 是字符串")
    }

    pub fn cwd(&self) -> &str {
        self.info()
            .get("cwd")
            .and_then(Value::as_str)
            .expect("load_grok_bundle 已校验 info.cwd 是字符串")
    }
}

/// bundle 里承载历史的那个文件；updates 优先。
pub fn authoritative_history(path: &Path) -> PathBuf {
    let updates = path.join("updates.jsonl");
    if updates.is_file() {
        updates
    } else {
        path.join("chat_history.jsonl")
    }
}

/// 等价 `Path.read_text()`：Python 文本模式默认开启 universal newlines，
/// `\r\n` 与 `\r` 都会被翻译成 `\n` 之后才交给 `split_jsonl_lines`。
pub fn read_text(path: &Path) -> std::io::Result<String> {
    let raw = fs::read_to_string(path)?;
    if !raw.contains('\r') {
        return Ok(raw);
    }
    Ok(raw.replace("\r\n", "\n").replace('\r', "\n"))
}

fn io_error(path: &Path, error: &std::io::Error) -> DomainError {
    DomainError::internal(format!(
        "读取 Grok 会话文件失败: {}: {error}",
        path.display()
    ))
}

/// `(记录, 损坏行诊断)`。
type Records = (Vec<Value>, Vec<Map<String, Value>>);

/// 逐行解析 JSONL；容忍**最后一行**截断，其余损坏行记 diagnostics。
fn parse_jsonl(path: &Path) -> DomainResult<Records> {
    if !path.is_file() {
        return Ok((Vec::new(), Vec::new()));
    }
    let text = read_text(path).map_err(|error| io_error(path, &error))?;
    let lines = split_jsonl_lines(&text);
    // 最后一条非空行可能正在被 Grok 追加写，截断不算损坏。
    let final_index = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, _)| index as isize)
        .next_back()
        .unwrap_or(-1);
    let mut records = Vec::new();
    let mut diagnostics = Vec::new();
    let diagnostic = |index: usize, reason: &str| {
        let mut entry = Map::new();
        entry.insert("line".into(), Value::from(index as i64 + 1));
        entry.insert("reason".into(), Value::from(reason));
        entry
    };
    for (index, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(line) {
            Err(_) => {
                if index as isize != final_index {
                    diagnostics.push(diagnostic(index, "invalid_json"));
                }
            }
            Ok(Value::Object(entries)) => records.push(Value::Object(entries)),
            Ok(_) => diagnostics.push(diagnostic(index, "non_object")),
        }
    }
    Ok((records, diagnostics))
}

/// 装载并校验一个 bundle。
///
/// 三道门：目录可解析 → summary.json 是当前结构（chat_format_version==1 且
/// info.id / info.cwd 均为字符串）→ 至少有一份历史。
pub fn load_grok_bundle(path: &Path) -> DomainResult<GrokBundle> {
    let root = fs::canonicalize(path)
        .map_err(|_| DomainError::session_not_found("grok", &path.to_string_lossy()))?;
    let summary_path = root.join("summary.json");
    let summary = read_text(&summary_path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .ok_or_else(|| {
            DomainError::agent_format_changed(
                "grok",
                "summary.json",
                Value::from("current summary object"),
                Value::Null,
            )
        })?;
    let info = summary.get("info").and_then(Value::as_object);
    let valid = info.is_some_and(|info| {
        info.get("id").is_some_and(Value::is_string)
            && info.get("cwd").is_some_and(Value::is_string)
    }) && summary.get("chat_format_version") == Some(&Value::from(1));
    if !valid {
        return Err(DomainError::agent_format_changed(
            "grok",
            "summary.json",
            json!({"chat_format_version": 1, "info": {"id": "str", "cwd": "str"}}),
            summary,
        ));
    }
    let (updates, mut diagnostics) = parse_jsonl(&root.join("updates.jsonl"))?;
    let (chat, chat_diagnostics) = parse_jsonl(&root.join("chat_history.jsonl"))?;
    if updates.is_empty() && chat.is_empty() {
        return Err(DomainError::agent_format_changed(
            "grok",
            "history",
            Value::from("updates.jsonl or chat_history.jsonl"),
            Value::Null,
        ));
    }
    diagnostics.extend(chat_diagnostics);
    Ok(GrokBundle {
        path: root,
        summary,
        updates,
        chat,
        diagnostics,
    })
}

/// bundle 内容指纹：`sha256:` + 逐成员 `name \0 bytes \0` 的摘要。
pub fn fingerprint(path: &Path) -> DomainResult<String> {
    let bundle = load_grok_bundle(path)?;
    let mut digest = Sha256::new();
    for member in bundle.authoritative_members() {
        let name = member
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        digest.update(name.as_bytes());
        digest.update(b"\0");
        digest.update(fs::read(&member).map_err(|error| io_error(&member, &error))?);
        digest.update(b"\0");
    }
    let mut hex = String::from("sha256:");
    for byte in digest.finalize() {
        hex.push_str(&format!("{byte:02x}"));
    }
    Ok(hex)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle(root: &Path, summary: &str, updates: &str, chat: Option<&str>) -> PathBuf {
        let path = root.join("bundle");
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("summary.json"), summary).unwrap();
        if !updates.is_empty() {
            fs::write(path.join("updates.jsonl"), updates).unwrap();
        }
        if let Some(chat) = chat {
            fs::write(path.join("chat_history.jsonl"), chat).unwrap();
        }
        path
    }

    const SUMMARY: &str = r#"{"info":{"id":"s1","cwd":"/w"},"chat_format_version":1}"#;

    #[test]
    fn trailing_truncated_line_is_tolerated_but_earlier_damage_is_reported() {
        let root = tempfile::tempdir().unwrap();
        let path = bundle(
            root.path(),
            SUMMARY,
            "{\"a\": 1}\n{broken\n[1,2]\n{\"b\": 2}\n{\"trunc\"",
            None,
        );
        let loaded = load_grok_bundle(&path).unwrap();
        assert_eq!(loaded.updates.len(), 2);
        // 第 2 行损坏、第 3 行非对象；第 5 行是截断的最后一行，不记诊断。
        let lines: Vec<i64> = loaded
            .diagnostics
            .iter()
            .map(|entry| entry["line"].as_i64().unwrap())
            .collect();
        assert_eq!(lines, [2, 3]);
        assert_eq!(loaded.diagnostics[0]["reason"], Value::from("invalid_json"));
        assert_eq!(loaded.diagnostics[1]["reason"], Value::from("non_object"));
    }

    #[test]
    fn schema_drift_is_reported_as_agent_format_changed() {
        let root = tempfile::tempdir().unwrap();
        // chat_format_version 不是 1。
        let path = bundle(
            root.path(),
            r#"{"info":{"id":"s","cwd":"/w"},"chat_format_version":2}"#,
            "{}\n",
            None,
        );
        let error = load_grok_bundle(&path).unwrap_err();
        assert_eq!(error.code, "agent.format_changed");

        // info.cwd 不是字符串。
        fs::write(
            path.join("summary.json"),
            r#"{"info":{"id":"s","cwd":1},"chat_format_version":1}"#,
        )
        .unwrap();
        assert_eq!(
            load_grok_bundle(&path).unwrap_err().code,
            "agent.format_changed"
        );

        // summary.json 不是合法 JSON。
        fs::write(path.join("summary.json"), "{").unwrap();
        assert_eq!(
            load_grok_bundle(&path).unwrap_err().code,
            "agent.format_changed"
        );
    }

    #[test]
    fn a_bundle_without_any_history_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let path = bundle(root.path(), SUMMARY, "", None);
        let error = load_grok_bundle(&path).unwrap_err();
        assert_eq!(error.code, "agent.format_changed");
        assert_eq!(error.params()["location"], Value::from("history"));
    }

    #[test]
    fn a_missing_bundle_is_a_session_not_found() {
        let root = tempfile::tempdir().unwrap();
        let error = load_grok_bundle(&root.path().join("nope")).unwrap_err();
        assert_eq!(error.code, "session.not_found");
    }

    #[test]
    fn authoritative_members_prefer_updates_over_chat() {
        let root = tempfile::tempdir().unwrap();
        let path = bundle(root.path(), SUMMARY, "{}\n", Some("{}\n"));
        let loaded = load_grok_bundle(&path).unwrap();
        assert!(loaded.authoritative_members()[1].ends_with("updates.jsonl"));
        fs::remove_file(path.join("updates.jsonl")).unwrap();
        let loaded = load_grok_bundle(&path).unwrap();
        assert!(loaded.authoritative_members()[1].ends_with("chat_history.jsonl"));
    }

    #[test]
    fn fingerprint_covers_member_names_and_bytes() {
        let root = tempfile::tempdir().unwrap();
        let path = bundle(root.path(), SUMMARY, "{\"a\": 1}\n", None);
        let before = fingerprint(&path).unwrap();
        assert!(before.starts_with("sha256:"));
        fs::write(path.join("updates.jsonl"), "{\"a\": 2}\n").unwrap();
        assert_ne!(fingerprint(&path).unwrap(), before);
    }

    #[test]
    fn read_text_applies_universal_newlines() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("a.txt");
        fs::write(&path, "a\r\nb\rc\n").unwrap();
        assert_eq!(read_text(&path).unwrap(), "a\nb\nc\n");
    }
}
