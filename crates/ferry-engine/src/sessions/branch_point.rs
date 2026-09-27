//! Durable, content-bound end-of-turn checkpoints shared by native forks and resume reads.
//! Tokens contain no transcript, path, or engine-local ref. They are locators, not authority.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::read_cursor::digest;
use crate::errors::{DomainError, DomainResult};
use crate::model::{native_locator, BlockKind, Session};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BranchPoint {
    v: u8,
    tool: String,
    session: String,
    pub end: usize,
    hash: String,
}

fn invalid(reason: &str) -> DomainError {
    let mut error = DomainError::agent_request_invalid(reason);
    error.params_mut().insert("field".into(), json!("through"));
    error.params_mut().insert("message".into(), json!(reason));
    error
}

fn visible(session: &Session, message: &crate::model::Message) -> bool {
    !session
        .context_compactions
        .iter()
        .any(|c| c.summary_message_id.is_some() && c.summary_message_id == message.source_id)
}

/// End is exclusive. Internal compaction messages do not create UI turns.
pub fn complete_end(session: &Session, start: usize) -> Option<usize> {
    let first = session.messages.get(start)?;
    if first.role != "user" || !visible(session, first) {
        return None;
    }
    let next = session
        .messages
        .iter()
        .enumerate()
        .skip(start + 1)
        .find(|(_, m)| m.role == "user" && visible(session, m))
        .map_or(session.messages.len(), |(i, _)| i);
    let (last_index, last) = session.messages[..next]
        .iter()
        .enumerate()
        .skip(start + 1)
        .rev()
        .find(|(_, m)| visible(session, m))?;
    (last.role == "assistant"
        && last.turn_complete == Some(true)
        && last
            .blocks
            .iter()
            .any(|b| b.kind == BlockKind::Text && !b.text.trim().is_empty()))
    .then_some(last_index + 1)
}

/// Match show()'s visible-message pagination without issuing locators for unloaded turns.
pub fn completed_on_page(session: &Session, from: usize, to: usize) -> Vec<usize> {
    let mut count = 0;
    let positions: Vec<usize> = session
        .messages
        .iter()
        .map(|m| {
            if visible(session, m) {
                count += 1;
            }
            count
        })
        .collect();
    session
        .messages
        .iter()
        .enumerate()
        .filter_map(|(i, _)| {
            let end = complete_end(session, i)?;
            (positions[end - 1] >= from && positions[end - 1] <= to).then_some(i)
        })
        .collect()
}

impl BranchPoint {
    pub fn issue(session: &Session, user_locator: &str) -> DomainResult<String> {
        let start = session
            .messages
            .iter()
            .enumerate()
            .position(|(i, m)| native_locator(m, i) == user_locator)
            .ok_or_else(|| invalid("Selected turn no longer exists; reload the session."))?;
        let end = complete_end(session, start).ok_or_else(|| {
            invalid("Only a natively completed answer can be used as a branch point.")
        })?;
        let point = Self {
            v: 1,
            tool: session.source_tool.clone(),
            session: session.source_id.clone(),
            end,
            hash: prefix_hash(session, end),
        };
        Ok(format!(
            "fbp_{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&point).unwrap())
        ))
    }

    pub fn resolve(token: &str, session: &Session) -> DomainResult<Self> {
        let fail =
            || invalid("Branch point is invalid or its history changed; select the turn again.");
        let encoded = token
            .strip_prefix("fbp_")
            .filter(|s| s.len() <= 4096)
            .ok_or_else(fail)?;
        let point: Self =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(encoded).map_err(|_| fail())?)
                .map_err(|_| fail())?;
        if point.v != 1
            || point.tool != session.source_tool
            || point.session != session.source_id
            || point.end == 0
            || point.end > session.messages.len()
            || point.hash != prefix_hash(session, point.end)
        {
            return Err(fail());
        }
        let start = session.messages[..point.end]
            .iter()
            .rposition(|m| m.role == "user" && visible(session, m))
            .ok_or_else(fail)?;
        if complete_end(session, start) != Some(point.end) {
            return Err(fail());
        }
        Ok(point)
    }

    pub fn native_request(&self, session: &Session, reference: &str) -> Value {
        let last = &session.messages[self.end - 1];
        json!({"tool": self.tool, "sessionId": self.session, "cwd": session.cwd,
            "sourceRef": reference, "lastMessageId": last.source_id,
            "turn": session.messages[..self.end].iter().filter(|m| m.role == "user" && visible(session, m)).count()})
    }
}

fn prefix_hash(session: &Session, end: usize) -> String {
    digest(&json!([
        session.source_tool,
        session.source_id,
        &session.messages[..end]
    ]))
}

/// Drop every derived whole-session object as well: child transcripts and later summaries
/// must never become a back door around the boundary.
pub fn restrict(session: &mut Session, token: Option<&str>) -> DomainResult<()> {
    if let Some(token) = token {
        let point = BranchPoint::resolve(token, session)?;
        session.messages.truncate(point.end);
        session.children.clear();
        session.agent_edges.clear();
        session.context_compactions.clear();
        session.loss.clear();
        session.title = "Bounded conversation".into();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Block, Message};
    fn sample() -> Session {
        let mut s = Session::new("pi", "session", "/work");
        for (i, role) in ["user", "assistant", "user", "assistant"]
            .into_iter()
            .enumerate()
        {
            let mut m = Message::new(role);
            m.source_id = Some(format!("m{i}"));
            m.blocks.push(Block::text(format!("text{i}")));
            m.turn_complete = (role == "assistant").then_some(true);
            s.messages.push(m);
        }
        s
    }
    #[test]
    fn internal_summaries_do_not_move_visible_turn_or_page_boundaries() {
        let mut s = sample();
        let mut summary = Message::new("user");
        summary.source_id = Some("internal-summary".into());
        summary.blocks.push(Block::text("internal"));
        s.messages.insert(2, summary);
        let mut compaction = crate::model::ContextCompaction::new("compact", "pi");
        compaction.summary_message_id = Some("internal-summary".into());
        s.context_compactions.push(compaction);
        assert_eq!(complete_end(&s, 0), Some(2));
        assert_eq!(complete_end(&s, 2), None);
        assert!(completed_on_page(&s, 1, 1).is_empty());
        assert_eq!(completed_on_page(&s, 1, 2), vec![0]);
        assert_eq!(completed_on_page(&s, 3, 4), vec![3]);
        let token = BranchPoint::issue(&s, "m2").unwrap();
        let point = BranchPoint::resolve(&token, &s).unwrap();
        assert_eq!(point.native_request(&s, "file")["turn"], 2);
    }

    #[test]
    fn point_survives_append_but_not_rewrite_or_other_session() {
        let mut s = sample();
        let token = BranchPoint::issue(&s, "m0").unwrap();
        s.messages.push(Message::new("user"));
        assert_eq!(BranchPoint::resolve(&token, &s).unwrap().end, 2);
        s.messages[0].blocks[0].text = "changed".into();
        assert!(BranchPoint::resolve(&token, &s).is_err());
        let mut other = sample();
        other.source_id = "other".into();
        assert!(BranchPoint::resolve(&token, &other).is_err());
    }
    #[test]
    fn incomplete_answers_and_malformed_points_fail_closed() {
        let mut s = sample();
        s.messages[1].turn_complete = Some(false);
        assert!(BranchPoint::issue(&s, "m0").is_err());
        assert!(BranchPoint::resolve("fbp_invalid", &s).is_err());
    }
    #[test]
    fn bounded_view_excludes_future_content_and_derived_metadata() {
        let mut s = sample();
        let token = BranchPoint::issue(&s, "m0").unwrap();
        s.children.push(sample());
        s.title = "future secret".into();
        restrict(&mut s, Some(&token)).unwrap();
        assert_eq!(s.messages.len(), 2);
        assert!(s.children.is_empty());
        assert!(!s.title.contains("secret"));
    }
}
