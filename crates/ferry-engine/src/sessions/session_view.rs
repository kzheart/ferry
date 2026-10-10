//! 原始消息、可见轮次与页面序号之间的映射。
use super::read_cursor::digest;
use crate::model::{BlockKind, Message, Session};
use serde_json::{json, Value};
use std::collections::HashSet;

pub(super) struct SessionView<'a> {
    session: &'a Session,
    summaries: HashSet<&'a str>,
}

impl<'a> SessionView<'a> {
    pub fn new(session: &'a Session) -> Self {
        let summaries = session
            .context_compactions
            .iter()
            .filter_map(|entry| entry.summary_message_id.as_deref())
            .collect();
        Self { session, summaries }
    }

    pub fn visible(&self, message: &Message) -> bool {
        message
            .source_id
            .as_deref()
            .is_none_or(|id| !self.summaries.contains(id))
    }

    pub fn complete_end(&self, start: usize) -> Option<usize> {
        let first = self.session.messages.get(start)?;
        if first.role != "user" || !self.visible(first) {
            return None;
        }
        let next = self
            .session
            .messages
            .iter()
            .enumerate()
            .skip(start + 1)
            .find(|(_, message)| message.role == "user" && self.visible(message))
            .map_or(self.session.messages.len(), |(index, _)| index);
        let (index, last) = self.session.messages[..next]
            .iter()
            .enumerate()
            .skip(start + 1)
            .rev()
            .find(|(_, message)| self.visible(message))?;
        (last.role == "assistant"
            && last.turn_complete == Some(true)
            && last
                .blocks
                .iter()
                .any(|block| block.kind == BlockKind::Text && !block.text.trim().is_empty()))
        .then_some(index + 1)
    }

    pub fn completed_on_page(&self, from: usize, to: usize) -> Vec<usize> {
        let mut position = 0;
        let positions: Vec<_> = self
            .session
            .messages
            .iter()
            .map(|message| {
                position += usize::from(self.visible(message));
                position
            })
            .collect();
        self.session
            .messages
            .iter()
            .enumerate()
            .filter_map(|(index, _)| {
                let end = self.complete_end(index)?;
                (positions[end - 1] >= from && positions[end - 1] <= to).then_some(index)
            })
            .collect()
    }

    pub fn latest_user_before(&self, end: usize) -> Option<usize> {
        self.session.messages[..end]
            .iter()
            .rposition(|message| message.role == "user" && self.visible(message))
    }

    pub fn native_anchor(&self, end: usize) -> &'a Message {
        self.session
            .messages
            .iter()
            .filter(|message| self.visible(message))
            .nth(end - 1)
            .unwrap_or(&self.session.messages[end - 1])
    }

    pub fn visible_turns(&self, end: usize) -> usize {
        self.session.messages[..end]
            .iter()
            .filter(|message| message.role == "user" && self.visible(message))
            .count()
    }

    pub fn prefix_digest(&self, end: usize) -> String {
        digest(&json!([
            self.session.source_tool,
            self.session.source_id,
            &self.session.messages[..end]
        ]))
    }
}

pub(super) fn content_snapshot(revision: &str, session: &Session) -> String {
    digest(&json!([revision, session]))
}

pub(super) fn raw_turns_before(session: &Session, end: usize) -> usize {
    session
        .messages
        .iter()
        .take(end)
        .filter(|message| message.role == "user")
        .count()
}

pub(super) fn clear_derived_metadata(session: &mut Session) {
    session.children.clear();
    session.agent_edges.clear();
    session.context_compactions.clear();
    session.loss.clear();
    session.title = "Bounded conversation".into();
}

pub(super) fn message_identity(message: &Message) -> Value {
    json!({"origin": super::inert::message_origin(message),
        "duplicate_key": super::inert::duplicate_key(message)})
}
