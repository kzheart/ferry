//! Grok 作为迁移目标。
//!
//! 与其他目标的两处差异：
//! - 方言没覆盖的操作**不降级成叙述**，而是按源端名称与参数原样落地为外来工具
//!   记录（`foreign_tool_record`）——形态没变，身份变了，理由码要说清楚；
//! - `plan` 在共享统计之上追加 context compaction 的丢弃计数（Grok bundle 里
//!   没有承载它的字段）。

use serde_json::{Map, Value};

use crate::adapters::shared::dialect::ToolDialect;
use crate::adapters::shared::migration::{
    default_plan, MigrationTargetBase, RenderedTool, ToolVerdict,
};
use crate::errors::DomainResult;
use crate::events::event;
use crate::model::{
    tool_result_text, Message, Session, ToolCall, ToolResultBlockKind, ToolResultStatus,
};
use crate::tool_ops::{has_valid_tool_input, CanonicalOp};

use super::dialect::DIALECT;
use super::writer::write_bundle;

pub struct GrokMigrationTarget;

impl MigrationTargetBase for GrokMigrationTarget {
    fn tool(&self) -> &str {
        "grok"
    }

    fn dialect(&self) -> Option<&ToolDialect> {
        Some(&DIALECT)
    }

    /// 方言能写出来的操作 + `tool.invoke` 是原生的，其余降级。
    fn tool_fidelity(&self, op: &str) -> ToolVerdict {
        if op == CanonicalOp::TOOL_INVOKE || DIALECT.write_ops().contains(op) {
            ToolVerdict::Native
        } else {
            ToolVerdict::Degrade
        }
    }

    /// Grok 的 tool_call_update 认 pending 状态。
    fn tool_result_statuses(&self) -> &[ToolResultStatus] {
        &[
            ToolResultStatus::Success,
            ToolResultStatus::Error,
            ToolResultStatus::Pending,
        ]
    }

    fn tool_result_native_blocks(&self) -> &[ToolResultBlockKind] {
        &[ToolResultBlockKind::Text]
    }

    fn tool_result_projected_blocks(&self) -> &[ToolResultBlockKind] {
        &[ToolResultBlockKind::Json]
    }

    fn preview_tool(
        &self,
        tool: &ToolCall,
        _session: &Session,
        message: Option<&Message>,
    ) -> Option<RenderedTool> {
        // 用户消息里的工具块只能变成叙述文本：Grok 的 user 行没有工具槽位。
        if message.is_some_and(|message| message.role == "user") {
            return None;
        }
        if !has_valid_tool_input(tool.op.as_deref(), &tool.input) {
            return None;
        }
        let rendered = self.dialect_preview(tool);
        if rendered.is_some() || tool.op.as_deref() == Some(CanonicalOp::TOOL_INVOKE) {
            return rendered;
        }
        // 方言尚无映射的操作：按源端名称与参数原样落地为外来工具记录。
        let output = tool_result_text(tool.result.as_ref());
        let consumed: Vec<String> = tool
            .input
            .as_object()
            .map(|entries| entries.keys().cloned().collect())
            .unwrap_or_default();
        Some(
            RenderedTool::tool(&tool.name, tool.input.clone(), &output)
                .conversion("transformed")
                .consumed_fields(consumed)
                .reason_codes(&["foreign_tool_record"]),
        )
    }

    /// 共享统计 + compaction 丢弃计数。
    fn plan(&self, session: &Session) -> DomainResult<Map<String, Value>> {
        let mut result = default_plan(self, session)?;
        let compactions: i64 = session
            .walk()
            .iter()
            .map(|node| node.context_compactions.len() as i64)
            .sum();
        if compactions == 0 {
            return Ok(result);
        }
        for key in ["drop", "dropped"] {
            let current = result.get(key).and_then(Value::as_i64).unwrap_or(0);
            result.insert(key.into(), Value::from(current + compactions));
        }
        let mut params = Map::new();
        params.insert("kind".into(), Value::from("compaction"));
        params.insert("count".into(), Value::from(compactions));
        if let Some(Value::Array(details)) = result.get_mut("drop_details") {
            details.push(
                serde_json::to_value(event("migration.content_dropped", params))
                    .unwrap_or(Value::Null),
            );
        }
        Ok(result)
    }

    fn write(&self, session: &Session, cwd: &str) -> DomainResult<Map<String, Value>> {
        let decider = |tool: &ToolCall, node: &Session, message: Option<&Message>| {
            self.evaluate_tool(tool, node, message)
        };
        let (session_id, destination) = write_bundle(session, cwd, None, Some(&decider))?;
        let mut result = Map::new();
        result.insert("session_id".into(), Value::from(session_id));
        result.insert(
            "dest".into(),
            Value::from(destination.to_string_lossy().as_ref()),
        );
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::shared::migration::Fidelity;
    use crate::model::{text_tool_result, Block, BlockKind, ContextCompaction};
    use serde_json::json;

    fn session_with(tool: ToolCall, role: &str) -> Session {
        let mut session = Session::new("fixture", "s", "/w");
        let mut message = Message::new(role);
        let mut block = Block::new(BlockKind::Tool);
        block.tool = Some(tool);
        message.blocks.push(block);
        session.messages.push(message);
        session
    }

    #[test]
    fn dialect_backed_operations_stay_native() {
        let mut tool = ToolCall::new(
            "Bash",
            Some(CanonicalOp::SHELL_EXEC.to_string()),
            json!({"command": "ls"}),
        );
        tool.result = Some(text_tool_result("out", ToolResultStatus::Success));
        let session = session_with(tool.clone(), "assistant");
        let decision = GrokMigrationTarget
            .evaluate_tool(&tool, &session, Some(&session.messages[0]))
            .unwrap();
        assert_eq!(decision.fidelity, Fidelity::Exact);
        let rendered = decision.rendered.unwrap();
        assert_eq!(rendered["name"], json!("run_terminal_command"));
        assert_eq!(rendered["input"], json!({"command": "ls"}));
    }

    #[test]
    fn operations_without_a_mapping_become_foreign_tool_records() {
        // fs.patch 在 grok 方言里没有绑定。
        let tool = ToolCall::new(
            "apply_patch",
            Some(CanonicalOp::FS_PATCH.to_string()),
            json!({"operations": [{"kind": "update"}]}),
        );
        let session = session_with(tool.clone(), "assistant");
        let decision = GrokMigrationTarget
            .evaluate_tool(&tool, &session, Some(&session.messages[0]))
            .unwrap();
        assert_eq!(decision.fidelity, Fidelity::Transformed);
        assert_eq!(decision.reason_codes, ["foreign_tool_record"]);
        let rendered = decision.rendered.unwrap();
        assert_eq!(rendered["name"], json!("apply_patch"));
        assert_eq!(
            rendered["input"],
            json!({"operations": [{"kind": "update"}]})
        );
    }

    #[test]
    fn tool_blocks_in_user_messages_degrade_to_narration() {
        let tool = ToolCall::new(
            "Bash",
            Some(CanonicalOp::SHELL_EXEC.to_string()),
            json!({"command": "ls"}),
        );
        let session = session_with(tool.clone(), "user");
        let decision = GrokMigrationTarget
            .evaluate_tool(&tool, &session, Some(&session.messages[0]))
            .unwrap();
        assert_eq!(decision.fidelity, Fidelity::Narrated);
        assert!(decision.rendered.is_none());
    }

    #[test]
    fn pending_results_are_natively_representable() {
        let mut tool = ToolCall::new(
            "Bash",
            Some(CanonicalOp::SHELL_EXEC.to_string()),
            json!({"command": "ls"}),
        );
        tool.result = Some(text_tool_result("", ToolResultStatus::Pending));
        let session = session_with(tool.clone(), "assistant");
        let decision = GrokMigrationTarget
            .evaluate_tool(&tool, &session, Some(&session.messages[0]))
            .unwrap();
        assert_eq!(decision.fidelity, Fidelity::Exact);
        // 但 interrupted 不行。
        let mut tool = tool;
        tool.result = Some(text_tool_result("", ToolResultStatus::Interrupted));
        let decision = GrokMigrationTarget
            .evaluate_tool(&tool, &session, Some(&session.messages[0]))
            .unwrap();
        assert_eq!(decision.fidelity, Fidelity::Narrated);
        assert_eq!(decision.reason_codes, ["unsupported_result_status"]);
    }

    #[test]
    fn plan_counts_context_compactions_as_drops() {
        let mut session = Session::new("fixture", "s", "/w");
        let mut message = Message::new("assistant");
        message.blocks.push(Block::text("hi"));
        session.messages.push(message);
        let baseline = default_plan(&GrokMigrationTarget, &session).unwrap();
        session
            .context_compactions
            .push(ContextCompaction::new("c1", "claude"));
        let plan = GrokMigrationTarget.plan(&session).unwrap();
        assert_eq!(
            plan["drop"].as_i64().unwrap(),
            baseline["drop"].as_i64().unwrap() + 1
        );
        assert_eq!(
            plan["dropped"].as_i64().unwrap(),
            baseline["dropped"].as_i64().unwrap() + 1
        );
        let details = plan["drop_details"].as_array().unwrap();
        let last = details.last().unwrap();
        assert_eq!(last["code"], json!("migration.content_dropped"));
        assert_eq!(last["params"]["kind"], json!("compaction"));
        assert_eq!(last["params"]["count"], json!(1));
    }
}
