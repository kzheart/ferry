//! Pi 迁移目标。
//!
//! 实现 `MigrationTargetBase` 即自动获得 `contracts::MigrationTarget`
//! （shared 的 blanket impl），因此这里只声明四个类级差异点 + `write`。

use serde_json::{Map, Value};

use crate::adapters::shared::dialect::ToolDialect;
use crate::adapters::shared::migration::{MigrationTargetBase, ToolVerdict};
use crate::errors::DomainResult;
use crate::model::{Session, ToolResultBlockKind, ToolResultStatus};

use super::dialect::DIALECT;
use super::writer::{op_fidelity, write, ToolDecider};

pub struct PiMigrationTarget;

impl MigrationTargetBase for PiMigrationTarget {
    fn tool(&self) -> &str {
        "pi"
    }

    fn dialect(&self) -> Option<&ToolDialect> {
        Some(&DIALECT)
    }

    fn tool_fidelity(&self, op: &str) -> ToolVerdict {
        op_fidelity(op)
    }

    /// pi 的 `bashExecution` 能表达「被中断」，比默认的两档多一档。
    fn tool_result_statuses(&self) -> &[ToolResultStatus] {
        &[
            ToolResultStatus::Success,
            ToolResultStatus::Error,
            ToolResultStatus::Interrupted,
        ]
    }

    /// toolResult 的 content 原生支持 text 与 image 两种 part。
    fn tool_result_native_blocks(&self) -> &[ToolResultBlockKind] {
        &[ToolResultBlockKind::Text, ToolResultBlockKind::Image]
    }

    fn write(&self, session: &Session, cwd: &str) -> DomainResult<Map<String, Value>> {
        // `evaluate_tool` 是 plan / preview / writer 三路唯一的判定入口，
        // writer 必须复用它而不是自己再判一遍。
        let decider: ToolDecider = &|tool, node, message| self.evaluate_tool(tool, node, message);
        write(session, cwd, None, Some(decider))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::shared::migration::{Fidelity, RenderDecision};
    use crate::model::{
        text_tool_result, Block, BlockKind, Message, ToolCall, ToolResult, ToolResultBlock,
        ToolResultStatus,
    };
    use crate::tool_ops::CanonicalOp;
    use serde_json::json;

    fn session_with(tool: ToolCall) -> Session {
        let mut block = Block::new(BlockKind::Tool);
        block.tool = Some(tool);
        let mut message = Message::new("assistant");
        message.blocks = vec![block];
        let mut session = Session::new("fixture", "root", "/tmp");
        session.messages = vec![message];
        session
    }

    fn decide(tool: &ToolCall) -> RenderDecision {
        let session = session_with(tool.clone());
        PiMigrationTarget
            .evaluate_tool(tool, &session, session.messages.first())
            .unwrap()
    }

    #[test]
    fn native_dialect_calls_stay_exact() {
        let mut call = ToolCall::new(
            "read",
            Some(CanonicalOp::FS_READ.to_string()),
            json!({"file_path": "/a.txt"}),
        );
        call.result = Some(text_tool_result("ok", ToolResultStatus::Success));
        let decision = decide(&call);
        assert_eq!(decision.fidelity, Fidelity::Exact);
        assert_eq!(decision.outcome(), "native");
        assert_eq!(
            decision.rendered.unwrap()["input"],
            json!({"path": "/a.txt"})
        );
    }

    #[test]
    fn pi_namespace_tool_invoke_is_native() {
        let mut call = ToolCall::new(
            "custom",
            Some(CanonicalOp::TOOL_INVOKE.to_string()),
            json!({"namespace": "pi", "name": "custom", "input": {"x": 1}}),
        );
        call.result = Some(text_tool_result("ok", ToolResultStatus::Success));
        let decision = decide(&call);
        assert_eq!(decision.fidelity, Fidelity::Exact);
        assert_eq!(decision.rendered.unwrap()["name"], json!("custom"));
    }

    #[test]
    fn foreign_namespace_tool_invoke_narrates() {
        let call = ToolCall::new(
            "native_lookup",
            Some(CanonicalOp::TOOL_INVOKE.to_string()),
            json!({"namespace": "codex", "name": "native_lookup", "input": {}}),
        );
        let decision = decide(&call);
        assert!(decision.rendered.is_none());
        assert_eq!(decision.fidelity, Fidelity::Narrated);
        assert_eq!(decision.reason_code(), Some("tool_to_history"));
    }

    #[test]
    fn patch_ops_degrade_to_narration() {
        let call = ToolCall::new(
            "apply_patch",
            Some(CanonicalOp::FS_PATCH.to_string()),
            json!({"operations": [{"kind": "update"}]}),
        );
        let decision = decide(&call);
        assert!(decision.rendered.is_none());
        assert_eq!(decision.fidelity, Fidelity::Narrated);
    }

    #[test]
    fn interrupted_results_survive_but_files_do_not() {
        let mut call = ToolCall::new(
            "bash",
            Some(CanonicalOp::SHELL_EXEC.to_string()),
            json!({"command": "sleep 100"}),
        );
        call.result = Some(text_tool_result("", ToolResultStatus::Interrupted));
        assert_eq!(decide(&call).outcome(), "native");

        let mut with_file = call.clone();
        with_file.result = Some(ToolResult {
            status: ToolResultStatus::Success,
            blocks: vec![ToolResultBlock::new(ToolResultBlockKind::File)],
            ..ToolResult::default()
        });
        let decision = decide(&with_file);
        assert_eq!(decision.fidelity, Fidelity::Lossy);
        assert!(decision
            .reason_codes
            .contains(&"tool_result_block_dropped".to_string()));
    }

    #[test]
    fn image_result_blocks_are_native_for_pi() {
        let mut call = ToolCall::new(
            "read",
            Some(CanonicalOp::FS_READ.to_string()),
            json!({"file_path": "/a.png"}),
        );
        call.result = Some(ToolResult {
            status: ToolResultStatus::Success,
            blocks: vec![ToolResultBlock::new(ToolResultBlockKind::Image)],
            ..ToolResult::default()
        });
        assert_eq!(decide(&call).fidelity, Fidelity::Exact);
    }

    #[test]
    fn preview_reports_schema_version_three() {
        let call = ToolCall::new(
            "read",
            Some(CanonicalOp::FS_READ.to_string()),
            json!({"file_path": "/a.txt"}),
        );
        let session = session_with(call);
        let preview = PiMigrationTarget.preview(&session, None).unwrap();
        assert_eq!(preview["schema_version"], json!(3));
        assert_eq!(preview["target_tool"], json!("pi"));
        assert_eq!(preview["read_only"], json!(true));
    }
}
