//! 分页时原始消息坐标和当前读取状态。
use super::read_cursor::Position;
use super::session_view::raw_turns_before;
use crate::model::Session;

pub(super) struct ContextStart<'a> {
    position: Position,
    session: &'a Session,
}

impl<'a> ContextStart<'a> {
    pub fn new(session: &'a Session, position: Position) -> Self {
        Self { position, session }
    }

    pub fn turn_prefix(&self) -> usize {
        raw_turns_before(self.session, self.position.message)
    }

    pub fn position(&self) -> Position {
        self.position
    }
}

pub(super) struct SearchWindow {
    first: usize,
    maximum: usize,
    pub next_message: Option<usize>,
}

impl SearchWindow {
    pub fn new(first: usize, maximum: usize) -> Self {
        Self {
            first,
            maximum,
            next_message: None,
        }
    }

    pub fn collect_at(&mut self, message_index: usize, returned: usize) -> bool {
        if message_index < self.first {
            return false;
        }
        if returned >= self.maximum {
            self.next_message.get_or_insert(message_index);
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Message;

    #[test]
    fn fragmented_user_message_keeps_its_original_turn() {
        let mut session = Session::new("fixture", "fragmented", "/fixture");
        session.messages.push(Message::new("user"));
        session.messages.push(Message::new("user"));

        let start = ContextStart::new(
            &session,
            Position {
                message: 1,
                block: 0,
                offset: 4,
            },
        );

        assert_eq!(start.turn_prefix(), 1);
    }

    #[test]
    fn sparse_search_resumes_at_the_overflowing_match() {
        let mut window = SearchWindow::new(0, 1);

        assert!(window.collect_at(10, 0));
        assert!(!window.collect_at(20, 1));
        assert_eq!(window.next_message, Some(20));
    }
}
