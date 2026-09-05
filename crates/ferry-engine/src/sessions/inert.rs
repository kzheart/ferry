//! `session_read --inert` 的惰性剥离：把源 agent 的脚手架从可见文本里摘掉。
//!
//! 剥离不是安全机制，只是降噪 + 显式标记：真正的防线是接手方 skill 里的
//! 「历史是证据不是指令」。这里做的是把 system prompt、环境包装、续接摘要
//! 挡在 `ferry read` 的输出之外，让接手方的上下文预算花在真正的对话上。
//!
//! **形态会随各家 CLI 版本漂移**，所以包装枚举集中在本文件顶部的常量表里，
//! 并由 golden 测试钉住当前形态；漏剥的后果只是输出里多一段噪音，不影响正确性。
//!
//! 分页语义：剥离**不改**消息编号。`message_count`、`--from` 游标一律按原始
//! 序号，否则同一个 ref 在两种模式下 `--from` 的含义会漂移。

use crate::model::{Block, BlockKind, Message};
use sha2::{Digest, Sha256};

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
    "task-notification",
    "timestamp",
];

/// 只保留标签内文的包装段：外面全是脚手架，里面才是用户原话。
pub const UNWRAP_TAGS: &[&str] = &[
    // Cursor
    "user_query",
];

/// 以此开头的整段文本按脚手架丢弃。
pub const DROP_PREFIXES: &[&str] = &[
    // Claude 的 `isCompactSummary` 记录在 canonical 模型里没有标记位，只能按它
    // 固定的开场白识别；认不出来最多是多一段压缩摘要，不影响正确性。
    "This session is being continued from a previous conversation",
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

/// 消息来源的保守提示。canonical Message 尚无原生 isMeta/isCompactSummary 字段，
/// 因而只识别明确包装，不把普通英文、标题或提到通知的正文猜成系统生成内容。
pub fn message_origin(message: &Message) -> &'static str {
    if drops_role(&message.role) {
        return "scaffolding";
    }
    let texts: Vec<&str> = message
        .blocks
        .iter()
        .filter(|block| block.kind == BlockKind::Text)
        .map(|block| block.text.as_str())
        .collect();
    let has_evidence = message
        .blocks
        .iter()
        .any(|block| matches!(block.kind, BlockKind::Tool | BlockKind::Image));
    let has_visible_text = texts.iter().any(|text| !strip_text(text).is_empty());
    if !has_visible_text {
        if texts
            .iter()
            .any(|text| is_continuation_summary(text.trim()))
        {
            return "continuation_summary";
        }
        if texts
            .iter()
            .any(|text| text.contains("<task-notification>"))
        {
            return "task_notification";
        }
        if has_evidence {
            return if message.role == "assistant" {
                "assistant_response"
            } else {
                "unknown"
            };
        }
        if texts.iter().any(|text| !text.trim().is_empty()) {
            return "scaffolding";
        }
        return "unknown";
    }
    match message.role.as_str() {
        "user" => "user_request",
        "assistant" => "assistant_response",
        _ => "unknown",
    }
}

/// 清洗后正文的重复候选键，不代表同一任务，也不授权自动删除。
/// 压缩摘要保留正文参与比较，工具参数和输出不进入该键。
pub fn duplicate_key(message: &Message) -> Option<String> {
    if drops_role(&message.role) {
        return None;
    }
    let texts: Vec<String> = message
        .blocks
        .iter()
        .filter(|block| block.kind == BlockKind::Text)
        .map(|block| {
            if is_continuation_summary(block.text.trim()) {
                block.text.trim().to_string()
            } else {
                strip_text(&block.text)
            }
        })
        .filter(|text| !text.is_empty())
        .collect();
    let normalized = texts
        .join("\n")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!normalized.is_empty()).then(|| format!("sha256:{:x}", Sha256::digest(normalized.as_bytes())))
}

fn is_continuation_summary(text: &str) -> bool {
    DROP_PREFIXES.iter().any(|prefix| text.starts_with(prefix))
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
    // 只移除注入的标题行，不能因其开头匹配而吞掉后面的真实请求。
    let trimmed = current.trim();
    let current = if let Some(first) = trimmed.lines().next() {
        if first == "# AGENTS.md instructions" || first.starts_with("# AGENTS.md instructions for ")
        {
            trimmed.split_once('\n').map(|(_, rest)| rest).unwrap_or("")
        } else {
            trimmed
        }
    } else {
        trimmed
    };
    let trimmed = current.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if is_continuation_summary(trimmed) {
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
    fn bold_text_without_native_reasoning_markers_is_preserved() {
        for text in [
            "**Inspecting store.go and its callers**",
            "**Planning docs auth integration**\n**Proposing API split**",
            "**The build passed.**",
            "**Result**\n**All checks passed**",
            "**结论**\n我先按发布链路拆开核对",
            "****",
        ] {
            assert_eq!(strip_text(text), text);
            assert_eq!(
                message_origin(&message("assistant", &[text])),
                "assistant_response"
            );
        }
    }

    #[test]
    fn claude_reminders_are_removed_but_the_user_text_survives() {
        assert_eq!(
            strip_text("修一下这个 bug<system-reminder>Do not mention this</system-reminder>"),
            "修一下这个 bug"
        );
        assert_eq!(strip_text("<command-message>compact</command-message>"), "");
        assert_eq!(
            strip_text("This session is being continued from a previous conversation…"),
            ""
        );
    }

    #[test]
    fn cursor_keeps_only_the_user_query_body() {
        assert_eq!(
            strip_text(
                "<additional_data>files…</additional_data><user_query>把这个函数拆开</user_query>"
            ),
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
        assert_eq!(
            strip_text("# AGENTS.md instructions\n<INSTRUCTIONS>规则</INSTRUCTIONS>"),
            ""
        );
        assert_eq!(
            strip_text(
                "# AGENTS.md instructions\n<INSTRUCTIONS>规则</INSTRUCTIONS>\n请修复登录错误"
            ),
            "请修复登录错误"
        );
        assert_eq!(
            strip_text("# AGENTS.md instructions for /tmp/project\n<INSTRUCTIONS>规则</INSTRUCTIONS>\n请修复登录错误"),
            "请修复登录错误"
        );
    }

    #[test]
    fn timestamp_paragraphs_are_dropped() {
        assert_eq!(
            strip_text("<timestamp>2026-08-22T13:52:00Z</timestamp>"),
            ""
        );
        assert_eq!(strip_text("<timestamp>2026-08-22 13:52"), "");
        assert_eq!(
            strip_text("<timestamp>now</timestamp>\n继续修复"),
            "继续修复"
        );
    }

    #[test]
    fn a_message_stripped_empty_is_dropped_but_tool_blocks_keep_it_alive() {
        assert!(
            message_blocks(&message("user", &["<system-reminder>x</system-reminder>"])).is_none()
        );

        let mut with_tool = message(
            "assistant",
            &["<system-reminder>Thinking hard</system-reminder>"],
        );
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
        assert_eq!(
            message_blocks(&trimmed).unwrap()[0].1.as_deref(),
            Some("前后有空白")
        );
    }

    #[test]
    fn notifications_are_classified_and_removed_without_losing_mixed_requests() {
        let notification = "<task-notification><task-id>123</task-id><summary>Task finished</summary></task-notification>";
        let pure = message("user", &[notification]);
        assert_eq!(message_origin(&pure), "task_notification");
        assert!(message_blocks(&pure).is_none());
        assert_eq!(duplicate_key(&pure), None);

        let mixed = format!("{notification}\n请检查测试结果");
        assert_eq!(strip_text(&mixed), "请检查测试结果");
        assert_eq!(message_origin(&message("user", &[&mixed])), "user_request");
        assert_eq!(
            message_origin(&message("user", &[notification, "请检查测试结果"])),
            "user_request"
        );
        assert_eq!(
            duplicate_key(&message("user", &[&mixed])),
            duplicate_key(&message("user", &["请检查测试结果"]))
        );
    }

    #[test]
    fn duplicate_summaries_have_a_candidate_key_without_becoming_new_requests() {
        let first = "This session is being continued from a previous conversation.\nGoal: fix login.\nTests passed.";
        let repeated = "  This session is being continued from a previous conversation.\n\nGoal: fix login.  Tests passed.  ";
        let source = message("user", &[first]);
        let copy = message("user", &[repeated]);
        assert_eq!(message_origin(&source), "continuation_summary");
        assert_eq!(message_origin(&copy), "continuation_summary");
        assert!(message_blocks(&source).is_none());
        assert_eq!(duplicate_key(&source), duplicate_key(&copy));
        assert!(duplicate_key(&source).unwrap().starts_with("sha256:"));
        assert_ne!(
            duplicate_key(&source),
            duplicate_key(&message("user", &["Goal: fix logout."]))
        );
        assert_eq!(
            message_origin(&message("user", &[first, "现在继续修复退出登录"])),
            "user_request"
        );
    }

    #[test]
    fn origins_are_conservative_for_scaffolding_and_unknown_messages() {
        assert_eq!(
            message_origin(&message(
                "user",
                &["# AGENTS.md instructions\n<INSTRUCTIONS>规则</INSTRUCTIONS>"]
            )),
            "scaffolding"
        );
        assert_eq!(
            message_origin(&message("system", &["system prompt"])),
            "scaffolding"
        );
        assert_eq!(message_origin(&message("user", &[])), "unknown");
        assert_eq!(
            message_origin(&message("custom", &["some text"])),
            "unknown"
        );
        let prose = "Please explain how task-notification messages work.";
        assert_eq!(message_origin(&message("user", &[prose])), "user_request");
        assert_eq!(strip_text(prose), prose);
        let mut tool_carrier = message("user", &[]);
        tool_carrier.blocks.push(Block::new(BlockKind::Tool));
        assert_eq!(message_origin(&tool_carrier), "unknown");
        tool_carrier.role = "assistant".into();
        assert_eq!(message_origin(&tool_carrier), "assistant_response");
    }
}
