//! 上下文与搜索共用的会话元数据。
use super::index::IndexedSession;
use super::safety::{record_session_id, truncate_text};
use super::session_view::raw_turns_before;
use crate::model::Session;
use serde_json::{json, Value};

pub(super) struct ReadMetadata<'a> {
    tool: &'a str,
    reference: &'a str,
    record: &'a IndexedSession,
    session: &'a Session,
}

impl<'a> ReadMetadata<'a> {
    pub fn new(
        tool: &'a str,
        reference: &'a str,
        record: &'a IndexedSession,
        session: &'a Session,
    ) -> Self {
        Self {
            tool,
            reference,
            record,
            session,
        }
    }

    pub fn context(&self, inert: bool) -> Value {
        let title_source = if inert {
            super::inert::strip_text(&self.session.title)
        } else {
            self.session.title.clone()
        };
        let (title, title_truncated) = truncate_text(&title_source, 200);
        let (project, project_truncated) = truncate_text(&self.session.cwd, 1024);
        json!({"tool": self.tool, "ref": self.reference,
            "session_id": record_session_id(&self.record.row, Some(&self.session.source_id)),
            "revision": self.record.revision, "title": title, "project": project,
            "title_source": self.record.row.get("title_source").and_then(Value::as_str).unwrap_or(""),
            "title_truncated": title_truncated, "project_truncated": project_truncated,
            "message_count": self.session.messages.len(),
            "turn_count": raw_turns_before(self.session, self.session.messages.len()), "mode": "context"})
    }

    pub fn search(&self) -> serde_json::Map<String, Value> {
        let mut result = serde_json::Map::new();
        result.insert("tool".into(), json!(self.tool));
        result.insert("ref".into(), json!(self.reference));
        result.insert(
            "session_id".into(),
            json!(record_session_id(
                &self.record.row,
                Some(&self.session.source_id)
            )),
        );
        result.insert("revision".into(), json!(self.record.revision));
        result.insert("message_count".into(), json!(self.session.messages.len()));
        result.insert(
            "turn_count".into(),
            json!(raw_turns_before(self.session, self.session.messages.len())),
        );
        result
    }
}
