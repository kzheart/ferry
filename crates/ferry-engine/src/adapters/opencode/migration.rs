//! OpenCode 作为迁移目标的写入与规划能力。
//!
//! OpenCode 是**唯一**保留工具结果附件的目标端，且原生支持 running / pending
//! 两个中间状态，因此结果侧的降级判定比其它目标宽松。

use serde_json::{Map, Value};

use crate::adapters::shared::dialect::ToolDialect;
use crate::adapters::shared::migration::{
    linked_agent_edge, MigrationTargetBase, RenderedTool, ToolVerdict,
};
use crate::errors::DomainResult;
use crate::model::{Message, Session, ToolCall, ToolResultStatus};
use crate::tool_ops::{has_valid_tool_input, CanonicalOp};

use super::dialect::DIALECT;
use super::tool_calls::op_fidelity;
use super::writer;

/// OpenCode 迁移目标。
pub struct OpenCodeMigrationTarget;

impl MigrationTargetBase for OpenCodeMigrationTarget {
    fn tool(&self) -> &str {
        "opencode"
    }

    fn dialect(&self) -> Option<&ToolDialect> {
        Some(&DIALECT)
    }

    fn tool_fidelity(&self, op: &str) -> ToolVerdict {
        op_fidelity(op)
    }

    /// running / pending 在 opencode 里是一等状态，不必降级成文本。
    fn tool_result_statuses(&self) -> &[ToolResultStatus] {
        &[
            ToolResultStatus::Success,
            ToolResultStatus::Error,
            ToolResultStatus::Running,
            ToolResultStatus::Pending,
        ]
    }

    /// 五个目标端里唯一保留 attachments 的那个。
    fn preserves_tool_result_attachments(&self) -> bool {
        true
    }

    /// 子 Agent 派生必须真的能连上一条边，否则只能降级成历史叙述。
    fn preview_tool(
        &self,
        tool: &ToolCall,
        session: &Session,
        message: Option<&Message>,
    ) -> Option<RenderedTool> {
        if !has_valid_tool_input(tool.op.as_deref(), &tool.input) {
            return None;
        }
        if tool.op.as_deref() == Some(CanonicalOp::AGENT_SPAWN)
            && linked_agent_edge(session, tool, message, true).is_none()
        {
            return None;
        }
        self.dialect_preview(tool)
    }

    fn write(&self, session: &Session, cwd: &str) -> DomainResult<Map<String, Value>> {
        let decider = |tool: &ToolCall, session: &Session, message: &Message| -> DomainResult<_> {
            self.evaluate_tool(tool, session, Some(message))
        };
        let outcome = writer::write(session, Some(cwd), Some(&decider), None)?;
        let mut result = Map::new();
        result.insert("session_id".into(), Value::from(outcome.session_id));
        result.insert(
            "dest".into(),
            Value::from(outcome.dest.to_string_lossy().into_owned()),
        );
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::contracts::MigrationTarget;
    use crate::adapters::shared::dialect::register_dialect;
    use crate::adapters::shared::migration::{Fidelity, RenderDecision};
    use crate::model::{text_tool_result, AgentEdge, Block, BlockKind, Message, ToolResult};
    use serde_json::json;

    fn register() {
        register_dialect("opencode", &DIALECT);
    }

    fn spawn_tool(call_id: &str) -> ToolCall {
        let mut tool = ToolCall::new(
            "Task",
            Some(CanonicalOp::AGENT_SPAWN.into()),
            json!({"description": "d", "prompt": "p", "subagent_type": "general"}),
        );
        tool.source_call_id = Some(call_id.into());
        tool.result = Some(ToolResult::new(ToolResultStatus::Success));
        tool
    }

    #[test]
    fn every_writer_op_is_native_and_unknown_ops_degrade() {
        let target = OpenCodeMigrationTarget;
        let tool = ToolCall::new(
            "bash",
            Some(CanonicalOp::SHELL_EXEC.into()),
            json!({"command": "ls"}),
        );
        assert_eq!(
            MigrationTarget::classify_tool_call(&target, &tool),
            "native"
        );
        // 入参非法 → degrade（无论 tool_fidelity 说什么）。
        let broken = ToolCall::new("bash", Some(CanonicalOp::SHELL_EXEC.into()), json!({}));
        assert_eq!(
            MigrationTarget::classify_tool_call(&target, &broken),
            "degrade"
        );
    }

    fn decide(tool: &ToolCall) -> RenderDecision {
        let mut block = Block::new(BlockKind::Tool);
        block.tool = Some(tool.clone());
        let mut message = Message::new("assistant");
        message.blocks = vec![block];
        let mut session = Session::new("claude", "root", "/src");
        session.messages = vec![message];
        OpenCodeMigrationTarget
            .evaluate_tool(tool, &session, session.messages.first())
            .unwrap()
    }

    fn shell_call() -> ToolCall {
        ToolCall::new(
            "bash",
            Some(CanonicalOp::SHELL_EXEC.into()),
            json!({"command": "ls"}),
        )
    }

    #[test]
    fn running_and_pending_results_stay_native() {
        register();
        for status in [ToolResultStatus::Running, ToolResultStatus::Pending] {
            let mut call = shell_call();
            call.result = Some(text_tool_result("", status));
            let decision = decide(&call);
            assert_eq!(decision.fidelity, Fidelity::Exact, "{status:?}");
            assert_eq!(decision.outcome(), "native");
        }
        // 五个目标端里只有 opencode 保留 attachments：不该记丢弃、不该降级。
        let mut with_attachment = shell_call();
        with_attachment.result = Some(ToolResult {
            status: ToolResultStatus::Success,
            attachments: vec![json!({"type": "file", "path": "/a.png"})],
            ..ToolResult::default()
        });
        let decision = decide(&with_attachment);
        assert_eq!(decision.fidelity, Fidelity::Exact);
        assert!(!decision
            .reason_codes
            .contains(&"tool_result_attachments_dropped".to_string()));
    }

    #[test]
    fn agent_spawn_without_a_linked_edge_has_no_preview() {
        register();
        let target = OpenCodeMigrationTarget;
        let mut session = Session::new("claude", "root", "/src");
        let mut message = Message::new("assistant");
        message.source_id = Some("spawn".into());
        let tool = spawn_tool("call-1");
        let mut block = Block::new(BlockKind::Tool);
        block.tool = Some(tool.clone());
        message.blocks = vec![block];
        session.messages = vec![message.clone()];

        assert!(target
            .preview_tool(&tool, &session, Some(&message))
            .is_none());

        let mut edge = AgentEdge::new("root", "child");
        edge.source_call_id = Some("call-1".into());
        session.agent_edges = vec![edge];
        let rendered = target
            .preview_tool(&tool, &session, Some(&message))
            .expect("有边即可原生渲染");
        assert_eq!(rendered.block["name"], json!("task"));
    }

    #[test]
    fn invalid_tool_input_never_previews() {
        register();
        let target = OpenCodeMigrationTarget;
        let session = Session::new("claude", "root", "/src");
        let tool = ToolCall::new("read", Some(CanonicalOp::FS_READ.into()), json!({}));
        assert!(target.preview_tool(&tool, &session, None).is_none());
    }
}
