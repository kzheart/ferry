//! 按完整响应的编码大小选择 UTF-8 block 分片。
use super::read_cursor::Position;
use super::safety::python_json_len;
use serde_json::{json, Value};

pub(super) struct BlockFragment<'a> {
    block: &'a Value,
    encoded: String,
}

impl<'a> BlockFragment<'a> {
    pub fn new(block: &'a Value) -> Self {
        Self {
            block,
            encoded: serde_json::to_string(block).expect("Value 可编码"),
        }
    }

    pub fn len(&self) -> usize {
        self.encoded.len()
    }

    pub fn valid_offset(&self, offset: usize) -> bool {
        offset < self.len() && self.encoded.is_char_boundary(offset)
    }

    pub fn value(&self, offset: usize, end: usize) -> Value {
        json!({"kind": "fragment", "block": self.block["block"],
            "fragment": {"encoding": "json", "offset_bytes": offset,
                "total_bytes": self.len(), "text": &self.encoded[offset..end], "complete": end == self.len()}})
    }

    pub fn next_position(&self, position: Position, end: usize, next_block: Position) -> Position {
        if end == self.len() {
            next_block
        } else {
            Position {
                offset: end,
                ..position
            }
        }
    }

    pub fn fitting_end(
        &self,
        position: Position,
        budget: usize,
        next_block: Position,
        candidate: impl Fn(Value, Position) -> Value,
    ) -> usize {
        let mut low = position.offset;
        let mut high = self.len();
        while low < high {
            let mut end = low + (high - low).div_ceil(2);
            while end > low && !self.encoded.is_char_boundary(end) {
                end -= 1;
            }
            if end == low {
                end = self.encoded[low..]
                    .char_indices()
                    .nth(1)
                    .map(|(index, _)| low + index)
                    .unwrap_or(self.len());
                if end > high {
                    break;
                }
            }
            let next = self.next_position(position, end, next_block);
            if python_json_len(&candidate(self.value(position.offset, end), next)) <= budget {
                low = end;
            } else {
                high = end - 1;
            }
        }
        low
    }
}
