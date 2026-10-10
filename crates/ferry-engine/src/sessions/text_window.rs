//! 搜索摘要与字符索引，保留原始文本用于展示。
use crate::model::{tool_result_text, BlockKind, Message};

pub(super) fn searchable_text(message: &Message, outputs: bool, inert: bool) -> String {
    let mut parts = Vec::new();
    for block in &message.blocks {
        if block.kind == BlockKind::Text && !block.text.is_empty() {
            let text = if inert {
                super::inert::strip_text(&block.text)
            } else {
                block.text.clone()
            };
            if !text.is_empty() {
                parts.push(text);
            }
        } else if outputs && block.kind == BlockKind::Tool {
            let Some(call) = block.tool.as_ref() else {
                continue;
            };
            parts.push(format!("[tool {}]", call.name));
            let output = tool_result_text(call.result.as_ref());
            if !output.is_empty() {
                parts.push(output);
            }
        }
    }
    parts.join("\n")
}

pub fn char_slice(text: &str, start: usize, end: usize) -> String {
    text.chars()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect()
}

pub fn char_find(haystack: &str, needle: &str) -> Option<usize> {
    haystack
        .find(needle)
        .map(|byte| haystack[..byte].encode_utf16().count())
}

pub(super) struct TextWindow {
    pub snippet: String,
    pub complete: bool,
}

impl TextWindow {
    pub fn around(text: &str, first_hit: usize) -> Self {
        let count = text.chars().count();
        let start = first_hit.saturating_sub(240);
        let end = count.min(first_hit + 560);
        Self {
            snippet: format!(
                "{}{}{}",
                if start > 0 { "…" } else { "" },
                char_slice(text, start, end),
                if end < count { "…" } else { "" }
            ),
            complete: start == 0 && end == count,
        }
    }
}
