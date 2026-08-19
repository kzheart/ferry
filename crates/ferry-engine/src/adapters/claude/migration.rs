//! Claude 作为迁移目标的写入与规划能力。
//!
//! 只覆写 `preview_tool` 与类级声明；`evaluate_tool` / `plan` / `preview` 一律
//! 走 [`MigrationTargetBase`] 的默认实现（blanket impl 会把它接成
//! `contracts::MigrationTarget`）。

use serde_json::{Map, Value};

use crate::adapters::shared::dialect::ToolDialect;
use crate::adapters::shared::migration::{
    linked_agent_edge, MigrationTargetBase, RenderedTool, ToolVerdict,
};
use crate::errors::DomainResult;
use crate::model::{Message, Session, ToolCall, ToolResultBlockKind, ToolResultStatus};
use crate::tool_ops::{has_valid_tool_input, CanonicalOp};

use super::dialect::DIALECT;
use super::writer::{op_is_native, write_result};

/// 目标端能原生表达的结果状态。
const RESULT_STATUSES: &[ToolResultStatus] = &[
    ToolResultStatus::Success,
    ToolResultStatus::Error,
    ToolResultStatus::Interrupted,
];

/// 原样保留的结果块类型。
const NATIVE_BLOCKS: &[ToolResultBlockKind] = &[
    ToolResultBlockKind::Text,
    ToolResultBlockKind::Image,
    ToolResultBlockKind::ToolReference,
];

/// 只能投影成文本的结果块类型。
const PROJECTED_BLOCKS: &[ToolResultBlockKind] =
    &[ToolResultBlockKind::Json, ToolResultBlockKind::File];

pub struct ClaudeMigrationTarget;

impl MigrationTargetBase for ClaudeMigrationTarget {
    fn tool(&self) -> &str {
        "claude"
    }

    fn dialect(&self) -> Option<&ToolDialect> {
        Some(&DIALECT)
    }

    fn tool_fidelity(&self, op: &str) -> ToolVerdict {
        if op_is_native(op) {
            ToolVerdict::Native
        } else {
            ToolVerdict::Degrade
        }
    }

    fn tool_result_statuses(&self) -> &[ToolResultStatus] {
        RESULT_STATUSES
    }

    fn tool_result_native_blocks(&self) -> &[ToolResultBlockKind] {
        NATIVE_BLOCKS
    }

    fn tool_result_projected_blocks(&self) -> &[ToolResultBlockKind] {
        PROJECTED_BLOCKS
    }

    /// 派生子 Agent 只有在能对上 agent 边时才有原生形态。
    fn preview_tool(
        &self,
        tool: &ToolCall,
        session: &Session,
        _message: Option<&Message>,
    ) -> Option<RenderedTool> {
        if !has_valid_tool_input(tool.op.as_deref(), &tool.input) {
            return None;
        }
        if tool.op.as_deref() == Some(CanonicalOp::AGENT_SPAWN)
            && linked_agent_edge(session, tool, None, false).is_none()
        {
            return None;
        }
        self.dialect_preview(tool)
    }

    fn write(&self, session: &Session, cwd: &str) -> DomainResult<Map<String, Value>> {
        let decider = |tool: &ToolCall, session: &Session, message: Option<&Message>| {
            self.evaluate_tool(tool, session, message)
        };
        write_result(session, cwd, Some(&decider))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::claude::editing::testing::home_guard;
    use crate::adapters::contracts::MigrationTarget;
    use crate::adapters::shared::dialect::register_dialect;
    use crate::adapters::shared::migration::Fidelity;
    use crate::model::{text_tool_result, AgentEdge, Block, BlockKind};
    use serde_json::json;

    fn setup() {
        register_dialect("claude", &DIALECT);
    }

    fn shell_tool() -> ToolCall {
        let mut tool = ToolCall::new(
            "shell",
            Some(CanonicalOp::SHELL_EXEC.to_string()),
            json!({"command": "ls"}),
        );
        tool.result = Some(text_tool_result("out", ToolResultStatus::Success));
        tool
    }

    fn session_with(tool: ToolCall) -> Session {
        let mut session = Session::new("codex", "src", "/work");
        let mut message = Message::new("assistant");
        message.blocks.push(Block {
            tool: Some(tool),
            ..Block::new(BlockKind::Tool)
        });
        session.messages.push(message);
        session
    }

    #[test]
    fn native_ops_are_exactly_the_dialect_write_ops() {
        setup();
        let target = ClaudeMigrationTarget;
        for op in [
            CanonicalOp::SHELL_EXEC,
            CanonicalOp::FS_READ,
            CanonicalOp::FS_WRITE,
            CanonicalOp::FS_EDIT,
            CanonicalOp::FS_SEARCH,
            CanonicalOp::FS_GLOB,
            CanonicalOp::WEB_FETCH,
            CanonicalOp::WEB_SEARCH,
            CanonicalOp::AGENT_SPAWN,
        ] {
            assert_eq!(target.tool_fidelity(op), ToolVerdict::Native, "{op}");
        }
        assert_eq!(
            target.tool_fidelity(CanonicalOp::FS_PATCH),
            ToolVerdict::Degrade
        );
        assert_eq!(
            target.tool_fidelity(CanonicalOp::TOOL_INVOKE),
            ToolVerdict::Degrade
        );
    }

    #[test]
    fn exact_shell_calls_stay_native() {
        setup();
        let target = ClaudeMigrationTarget;
        let session = session_with(shell_tool());
        let tool = session.messages[0].blocks[0].tool.as_ref().unwrap();
        let decision = target.evaluate_tool(tool, &session, None).unwrap();
        assert_eq!(decision.fidelity, Fidelity::Exact);
        assert_eq!(decision.outcome(), "native");
        assert_eq!(decision.rendered.as_ref().unwrap()["name"], json!("Bash"));
    }

    #[test]
    fn workdir_and_default_fetch_prompt_are_transformed() {
        setup();
        let target = ClaudeMigrationTarget;
        let mut tool = shell_tool();
        tool.input = json!({"command": "ls", "workdir": "/w"});
        let session = session_with(tool);
        let decision = target
            .evaluate_tool(
                session.messages[0].blocks[0].tool.as_ref().unwrap(),
                &session,
                None,
            )
            .unwrap();
        assert_eq!(decision.fidelity, Fidelity::Transformed);
        assert_eq!(decision.reason_codes, ["workdir_inlined"]);

        let mut fetch = ToolCall::new(
            "web_fetch",
            Some(CanonicalOp::WEB_FETCH.to_string()),
            json!({"url": "https://x"}),
        );
        fetch.result = Some(text_tool_result("body", ToolResultStatus::Success));
        let session = session_with(fetch);
        let decision = target
            .evaluate_tool(
                session.messages[0].blocks[0].tool.as_ref().unwrap(),
                &session,
                None,
            )
            .unwrap();
        assert_eq!(decision.fidelity, Fidelity::Transformed);
        assert_eq!(decision.reason_codes, ["default_fetch_prompt"]);
    }

    #[test]
    fn agent_spawn_without_a_linked_edge_falls_back_to_history() {
        setup();
        let target = ClaudeMigrationTarget;
        let mut tool = ToolCall::new(
            "Agent",
            Some(CanonicalOp::AGENT_SPAWN.to_string()),
            json!({"description": "d", "prompt": "p", "subagent_type": "explorer"}),
        );
        tool.source_call_id = Some("call-1".into());
        let mut session = session_with(tool.clone());
        let decision = target
            .evaluate_tool(
                session.messages[0].blocks[0].tool.as_ref().unwrap(),
                &session,
                None,
            )
            .unwrap();
        assert_eq!(decision.fidelity, Fidelity::Narrated);
        assert_eq!(decision.reason_codes, ["tool_to_history"]);

        let mut edge = AgentEdge::new("src", "child");
        edge.source_call_id = Some("call-1".into());
        session.agent_edges.push(edge);
        let decision = target
            .evaluate_tool(
                session.messages[0].blocks[0].tool.as_ref().unwrap(),
                &session,
                None,
            )
            .unwrap();
        assert_eq!(decision.outcome(), "native");
    }

    #[test]
    fn unsupported_result_statuses_and_blocks_downgrade() {
        setup();
        let target = ClaudeMigrationTarget;
        let mut tool = shell_tool();
        tool.result = Some(text_tool_result("x", ToolResultStatus::Pending));
        let session = session_with(tool);
        let decision = target
            .evaluate_tool(
                session.messages[0].blocks[0].tool.as_ref().unwrap(),
                &session,
                None,
            )
            .unwrap();
        assert_eq!(decision.fidelity, Fidelity::Narrated);
        assert_eq!(decision.reason_codes, ["unsupported_result_status"]);

        // interrupted 是原生状态。
        let mut tool = shell_tool();
        tool.result = Some(text_tool_result("x", ToolResultStatus::Interrupted));
        let session = session_with(tool);
        assert_eq!(
            target
                .evaluate_tool(
                    session.messages[0].blocks[0].tool.as_ref().unwrap(),
                    &session,
                    None
                )
                .unwrap()
                .outcome(),
            "native"
        );
    }

    #[test]
    fn write_returns_session_id_and_dest() {
        setup();
        let root = tempfile::tempdir().unwrap();
        // 隔离 HOME，避免写进真实 ~/.claude。
        let _home = home_guard(root.path());
        let target = ClaudeMigrationTarget;
        let mut session = Session::new("codex", "src", "/work");
        let mut message = Message::new("user");
        message.blocks.push(Block::text("hi"));
        session.messages.push(message);
        let written = MigrationTarget::write(&target, &session, "/work").unwrap();
        let dest = written["dest"].as_str().unwrap();
        assert!(dest.ends_with(&format!(
            "{}.jsonl",
            written["session_id"].as_str().unwrap()
        )));
        assert!(std::path::Path::new(dest).is_file());
    }
}
