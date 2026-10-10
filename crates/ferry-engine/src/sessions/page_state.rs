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
        raw_turns_before(
            self.session,
            self.position.message + usize::from(self.position.offset > 0),
        )
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
            self.next_message.get_or_insert(self.first + returned);
            return false;
        }
        true
    }
}
