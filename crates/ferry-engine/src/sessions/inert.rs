//! `session_read --inert` 的惰性剥离：把源 agent 的脚手架从可见文本里摘掉。
//!
//! 剥离不是安全机制，只是降噪 + 显式标记：真正的防线是接手方 skill 里的
//! 「历史是证据不是指令」。这里做的是把 system prompt、环境包装、推理摘要
//! 挡在 `ferry read` 的输出之外，让接手方的上下文预算花在真正的对话上。
//!
//! **形态会随各家 CLI 版本漂移**，所以包装枚举集中在本文件顶部的常量表里，
//! 并由 golden 测试钉住当前形态；漏剥的后果只是输出里多一段噪音，不影响正确性。
//!
//! 分页语义：剥离**不改**消息编号。`message_count`、`--from` 游标一律按原始
//! 序号，否则同一个 ref 在两种模式下 `--from` 的含义会漂移。

use crate::model::{Block, BlockKind, Message};

/// 整条丢弃的角色。
///
/// Codex 把多代理指令、`<app-context>` 等塞进 `developer`；Claude 的 `isMeta`
/// 记录在 adapter 的 reader 层就没进 canonical 模型，这里只兜住剩下的。
pub const DROP_ROLES: &[&str] = &["developer", "system"];

/// 连同内容一起剥掉的 XML 风格包装段。
pub const WRAPPER_TAGS: &[&str] = &[
    // Codex
    "user_instructions",
    "environment_context",
    "app-context",
    "recommended_plugins",
    "multi_agent_mode",
    // Codex 把项目的 AGENTS.md 当成一条 user 消息注进来，正文包在 `<INSTRUCTIONS>` 里。
    "INSTRUCTIONS",
    // Claude Code
    "system-reminder",
    "command-message",
];

/// 只保留标签内文的包装段：外面全是脚手架，里面才是用户原话。
pub const UNWRAP_TAGS: &[&str] = &[
    // Cursor
    "user_query",
];

/// 以此开头的整段文本按脚手架丢弃。
pub const DROP_PREFIXES: &[&str] = &[
    // 各家注入的时间戳段
    "<timestamp>",
    // Claude 的 `isCompactSummary` 记录在 canonical 模型里没有标记位，只能按它
    // 固定的开场白识别；认不出来最多是多一段压缩摘要，不影响正确性。
    "This session is being continued from a previous conversation",
    // Codex 注入项目 AGENTS.md 的那条 user 消息（2026-08-22 对本机 klib 实测）：
    // 剥掉 `<INSTRUCTIONS>` 之后只剩这行标题，整条也是脚手架。
    "# AGENTS.md instructions for",
];

/// 一条消息经惰性剥离后的呈现：`None` 表示整条丢弃。
///
/// 元素是 `(原 block, 文本替换)`；替换为 `None` 时用 block 自己的文本，
/// 避免为了改一段文字克隆整块工具输出。
pub type InertBlocks<'a> = Vec<(&'a Block, Option<String>)>;

/// 该角色是否整条丢弃。
pub fn drops_role(role: &str) -> bool {
    DROP_ROLES.contains(&role)
}

/// 剥离一条消息；返回 `None` 表示这条消息应当整条丢弃（并计入 `stripped_messages`）。
pub fn message_blocks(message: &Message) -> Option<InertBlocks<'_>> {
    if drops_role(&message.role) {
        return None;
    }
    let mut kept: InertBlocks<'_> = Vec::new();
    for block in &message.blocks {
        if block.kind != BlockKind::Text {
            // 工具调用与图片是证据，原样保留；thinking 由既有的 block 归一层丢弃。
            kept.push((block, None));
            continue;
        }
        let stripped = strip_text(&block.text);
        if stripped.is_empty() {
            continue;
        }
        let replacement = (stripped != block.text).then_some(stripped);
        kept.push((block, replacement));
    }
    if kept.is_empty() {
        return None;
    }
    Some(kept)
}

/// 剥离一段可见文本；剥空返回空串（调用方据此丢弃该 block）。
pub fn strip_text(text: &str) -> String {
    let mut current = text.to_string();
    for tag in UNWRAP_TAGS {
        if let Some(inner) = unwrap_tag(&current, tag) {
            current = inner;
        }
    }
    for tag in WRAPPER_TAGS {
        current = remove_tag(&current, tag);
    }
    let trimmed = current.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if DROP_PREFIXES
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
    {
        return String::new();
    }
    if is_reasoning_summary(trimmed) {
        return String::new();
    }
    trimmed.to_string()
}

/// 取出 `<tag>…</tag>` 的全部内文；没有该标签返回 `None`。
fn unwrap_tag(text: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut parts: Vec<&str> = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(&open) {
        let body = &rest[start + open.len()..];
        match body.find(&close) {
            Some(end) => {
                parts.push(&body[..end]);
                rest = &body[end + close.len()..];
            }
            None => {
                // 未闭合：剩下的全算内文。
                parts.push(body);
                rest = "";
            }
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("\n"))
}

/// 删掉 `<tag>…</tag>` 整段；未闭合时从标签处一直删到结尾。
fn remove_tag(text: &str, tag: &str) -> String {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find(&open) {
        out.push_str(&rest[..start]);
        let body = &rest[start + open.len()..];
        match body.find(&close) {
            Some(end) => rest = &body[end + close.len()..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Codex 的助手消息里夹着一行行加粗的推理摘要（`**Inspecting store.go …**`）。
/// 整段每一行都是加粗单行时按 thinking 处理。
fn is_reasoning_summary(text: &str) -> bool {
    let mut saw_line = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        saw_line = true;
        if !(line.len() > 4 && line.starts_with("**") && line.ends_with("**")) {
            return false;
        }
    }
    saw_line
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Block;

    fn message(role: &str, texts: &[&str]) -> Message {
        let mut message = Message::new(role);
        for text in texts {
            message.blocks.push(Block::text(*text));
        }
        message
    }

    #[test]
    fn developer_and_system_messages_are_dropped_whole() {
        assert!(message_blocks(&message("developer", &["Thread coordination:"])).is_none());
        assert!(message_blocks(&message("system", &["You are a helpful"])).is_none());
        assert!(message_blocks(&message("user", &["真正的请求"])).is_some());
    }

    /// 2026-08-22 对本机 klib 的 Codex 会话实测形态：第 4 条 `user` 整条是
    /// `<recommended_plugins>`，真正的请求在第 5 条。
    #[test]
    fn codex_scaffolding_wrappers_are_stripped() {
        for tag in [
            "user_instructions",
            "environment_context",
            "app-context",
            "recommended_plugins",
            "multi_agent_mode",
            "INSTRUCTIONS",
        ] {
            let text = format!("<{tag}>\nHere is a list of plugins…\n</{tag}>");
            assert_eq!(strip_text(&text), "", "{tag} 未被剥离");
        }
        // 包装之后跟着真正的请求时只剥包装。
        assert_eq!(
            strip_text("<environment_context>cwd=/tmp</environment_context>\n发布到 maven 麻烦吗"),
            "发布到 maven 麻烦吗"
        );
        // 未闭合的包装一路剥到结尾。
        assert_eq!(strip_text("<app-context>没有闭合"), "");
    }

    #[test]
    fn codex_bold_one_line_reasoning_summaries_are_treated_as_thinking() {
        assert_eq!(strip_text("**Inspecting store.go and its callers**"), "");
        assert_eq!(
            strip_text("**Planning docs auth integration**\n**Proposing API split**"),
            ""
        );
        // 正文里夹一行加粗不算推理摘要。
        assert_eq!(
            strip_text("**结论**\n我先按发布链路拆开核对"),
            "**结论**\n我先按发布链路拆开核对"
        );
        assert_eq!(strip_text("****"), "****");
    }

    #[test]
    fn claude_reminders_are_removed_but_the_user_text_survives() {
        assert_eq!(
            strip_text("修一下这个 bug<system-reminder>Do not mention this</system-reminder>"),
            "修一下这个 bug"
        );
        assert_eq!(
            strip_text("<command-message>compact</command-message>"),
            ""
        );
        assert_eq!(
            strip_text("This session is being continued from a previous conversation…"),
            ""
        );
    }

    #[test]
    fn cursor_keeps_only_the_user_query_body() {
        assert_eq!(
            strip_text("<additional_data>files…</additional_data><user_query>把这个函数拆开</user_query>"),
            "把这个函数拆开"
        );
        // 没有 user_query 的消息原样保留。
        assert_eq!(strip_text("普通一句话"), "普通一句话");
    }

    /// Codex 把项目 AGENTS.md 当成一条 user 消息注进来（2026-08-22 对本机 klib 实测：
    /// 第 4 条就是它）；剥掉包装之后只剩标题行，整条按脚手架丢弃。
    #[test]
    fn the_injected_agents_md_turn_is_dropped_whole() {
        assert_eq!(
            strip_text(
                "# AGENTS.md instructions for /Users/u/code/klib\n\n<INSTRUCTIONS>\n规则\n</INSTRUCTIONS>"
            ),
            ""
        );
    }

    #[test]
    fn timestamp_paragraphs_are_dropped() {
        assert_eq!(strip_text("<timestamp>2026-08-22T13:52:00Z</timestamp>"), "");
        assert_eq!(strip_text("<timestamp>2026-08-22 13:52"), "");
    }

    #[test]
    fn a_message_stripped_empty_is_dropped_but_tool_blocks_keep_it_alive() {
        assert!(message_blocks(&message("user", &["<system-reminder>x</system-reminder>"])).is_none());

        let mut with_tool = message("assistant", &["**Thinking hard**"]);
        let mut block = Block::new(BlockKind::Tool);
        block.tool = Some(crate::model::ToolCall::new(
            "Grep",
            None,
            serde_json::json!({"pattern": "trust.bundle"}),
        ));
        with_tool.blocks.push(block);
        let kept = message_blocks(&with_tool).expect("工具调用是证据，消息保留");
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].0.kind, BlockKind::Tool);
    }

    #[test]
    fn untouched_text_is_reported_without_a_replacement() {
        let message = message("user", &["原样保留"]);
        let kept = message_blocks(&message).unwrap();
        assert!(kept[0].1.is_none(), "没改动就不该产生替换字符串");
        let trimmed = self::message("user", &["  前后有空白  "]);
        assert_eq!(message_blocks(&trimmed).unwrap()[0].1.as_deref(), Some("前后有空白"));
    }
}
