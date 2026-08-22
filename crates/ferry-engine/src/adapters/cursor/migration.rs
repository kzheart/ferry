//! Cursor 作为迁移目标的规划与写入。
//!
//! **v1 的原生范围只有终端/Shell 类工具**：Cursor 的工具展示形态是内部枚举
//! （`toolFormerTool` / `toolCallCase`），逆向只坐实了 Shell 那一档；把别的工具硬
//! 塞成 Shell 枚举会渲染出语义错误的卡片。因此除 `shell.exec` 外的一切调用都走
//! 共享框架的标准降级——`preview_tool` 返回 `None` → `narrated` → 两层都写成
//! 历史叙述文本。
//!
//! 这里刻意**没有**用规格 §8 的另一种组合（上下文层折叠成文本、展示层仍摆一张
//! 通用工具卡片）：ferry 五个目标端的降级都是「一条历史叙述文本」，preview 的差异
//! 卡、`loss` 统计与 narration 模板全建立在这一条约定上。为 Cursor 单独造一种
//! 「展示层看起来是工具、上下文层其实是文本」的形态，会让 preview 里看到的和写进去
//! 的对不上，也让降级在五个目标端之间不可比。

use serde_json::{Map, Value};

use crate::adapters::shared::dialect::ToolDialect;
use crate::adapters::shared::migration::{MigrationTargetBase, RenderedTool, ToolVerdict};
use crate::errors::DomainResult;
use crate::model::{Message, Session, ToolCall, ToolResultBlockKind, ToolResultStatus};
use crate::tool_ops::{has_valid_tool_input, CanonicalOp};

use super::dialect::DIALECT;
use super::{store, writer};

/// Cursor 迁移目标。
pub struct CursorMigrationTarget;

impl MigrationTargetBase for CursorMigrationTarget {
    fn tool(&self) -> &str {
        "cursor"
    }

    fn dialect(&self) -> Option<&ToolDialect> {
        Some(&DIALECT)
    }

    /// 只有终端调用有原生形态；其余一律降级，不丢弃。
    fn tool_fidelity(&self, op: &str) -> ToolVerdict {
        if op == CanonicalOp::SHELL_EXEC {
            ToolVerdict::Native
        } else {
            ToolVerdict::Degrade
        }
    }

    /// `toolFormerData.status` 只有 completed / error 两个可写档位。
    ///
    /// cancelled（interrupted）与 loading（running）在读端认得，但写端造不出可信的
    /// 中间态——一条永远停在 loading 的工具卡片会让用户以为会话还在跑。
    fn tool_result_statuses(&self) -> &[ToolResultStatus] {
        &[ToolResultStatus::Success, ToolResultStatus::Error]
    }

    fn tool_result_native_blocks(&self) -> &[ToolResultBlockKind] {
        &[ToolResultBlockKind::Text]
    }

    fn tool_result_projected_blocks(&self) -> &[ToolResultBlockKind] {
        &[ToolResultBlockKind::Json]
    }

    /// 终端结果里没有附件的落位。
    fn preserves_tool_result_attachments(&self) -> bool {
        false
    }

    /// 只放行 `shell.exec`。
    ///
    /// 不能直接用基类的 `dialect_preview`：它对 `tool.invoke` 有一条「命名空间是
    /// 本方言或 mcp 就原样原生化」的通路，而 Cursor 写端没有通用工具形态，那条
    /// 通路会产出 writer 落不了地的 exact 判定。
    fn preview_tool(
        &self,
        tool: &ToolCall,
        _session: &Session,
        _message: Option<&Message>,
    ) -> Option<RenderedTool> {
        if tool.op.as_deref() != Some(CanonicalOp::SHELL_EXEC) {
            return None;
        }
        if !has_valid_tool_input(tool.op.as_deref(), &tool.input) {
            return None;
        }
        self.dialect_preview(tool)
    }

    /// Cursor 必须已完全退出才能被写：这条门禁提前到 plan 阶段，用户在「目标」
    /// 步骤就能看到提示，不必走完四步再被写入前的同一条检查拦下。
    fn preflight(&self) -> DomainResult<()> {
        store::ensure_offline()
    }

    fn write(&self, session: &Session, cwd: &str) -> DomainResult<Map<String, Value>> {
        let decide = |tool: &ToolCall, node: &Session, message: &Message| {
            self.evaluate_tool(tool, node, Some(message))
        };
        let outcome = writer::write(session, cwd, &decide)?;
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
    use crate::adapters::shared::migration::Fidelity;
    use crate::model::{text_tool_result, ToolResult};
    use serde_json::json;

    fn register() {
        register_dialect("cursor", &DIALECT);
    }

    fn shell(input: Value) -> ToolCall {
        let mut call = ToolCall::new("Bash", Some(CanonicalOp::SHELL_EXEC.into()), input);
        call.result = Some(text_tool_result("ok", ToolResultStatus::Success));
        call
    }

    #[test]
    fn only_shell_is_native() {
        register();
        let target = CursorMigrationTarget;
        assert_eq!(
            MigrationTarget::classify_tool_call(&target, &shell(json!({"command": "ls"}))),
            "native"
        );
        let read = ToolCall::new(
            "Read",
            Some(CanonicalOp::FS_READ.into()),
            json!({"file_path": "/a"}),
        );
        assert_eq!(
            MigrationTarget::classify_tool_call(&target, &read),
            "degrade"
        );
        // 入参非法的终端调用同样降级。
        assert_eq!(
            MigrationTarget::classify_tool_call(&target, &shell(json!({}))),
            "degrade"
        );
    }

    #[test]
    fn a_shell_call_renders_into_the_native_params() {
        register();
        let target = CursorMigrationTarget;
        let session = Session::new("claude", "root", "/w");
        let call = shell(json!({"command": "ls /tmp", "workdir": "/w", "description": "list"}));
        let rendered = target
            .preview_tool(&call, &session, None)
            .expect("终端调用必须有原生形态");
        assert_eq!(rendered.block["name"], json!("run_terminal_command_v2"));
        assert_eq!(rendered.block["input"]["command"], json!("ls /tmp"));
        assert_eq!(rendered.block["input"]["options"]["timeout"], json!(30000));
        assert_eq!(rendered.block["output"], json!("ok"));

        let decision = target.evaluate_tool(&call, &session, None).unwrap();
        assert_eq!(decision.fidelity, Fidelity::Exact);
        assert!(decision.ignored_fields.is_empty());
    }

    #[test]
    fn unsupported_shell_fields_are_reported_as_lossy() {
        register();
        let target = CursorMigrationTarget;
        let session = Session::new("claude", "root", "/w");
        let call = shell(json!({"command": "ls", "background": true}));
        let decision = target.evaluate_tool(&call, &session, None).unwrap();
        assert_eq!(decision.fidelity, Fidelity::Lossy);
        assert_eq!(decision.reason_codes, ["unsupported_tool_fields"]);
        assert_eq!(
            decision.ignored_fields.iter().cloned().collect::<Vec<_>>(),
            ["background"]
        );
    }

    #[test]
    fn every_other_tool_degrades_to_a_history_narration() {
        register();
        let target = CursorMigrationTarget;
        let session = Session::new("claude", "root", "/w");
        for (name, op, input) in [
            ("Read", CanonicalOp::FS_READ, json!({"file_path": "/a"})),
            ("Grep", CanonicalOp::FS_SEARCH, json!({"query": "todo"})),
            (
                "Task",
                CanonicalOp::AGENT_SPAWN,
                json!({"description": "d", "prompt": "p", "subagent_type": "general"}),
            ),
            (
                "mcp",
                CanonicalOp::TOOL_INVOKE,
                json!({"namespace": "mcp", "name": "x", "input": {}}),
            ),
        ] {
            let mut call = ToolCall::new(name, Some(op.into()), input);
            call.result = Some(ToolResult::new(ToolResultStatus::Success));
            let decision = target.evaluate_tool(&call, &session, None).unwrap();
            assert_eq!(decision.fidelity, Fidelity::Narrated, "{name}");
            assert!(decision.rendered.is_none(), "{name}");
            assert_eq!(decision.reason_codes, ["tool_to_history"], "{name}");
        }
    }

    #[test]
    fn intermediate_result_states_fall_back_to_narration() {
        register();
        let target = CursorMigrationTarget;
        let session = Session::new("claude", "root", "/w");
        for status in [ToolResultStatus::Running, ToolResultStatus::Interrupted] {
            let mut call = shell(json!({"command": "ls"}));
            call.result = Some(ToolResult::new(status));
            let decision = target.evaluate_tool(&call, &session, None).unwrap();
            assert_eq!(decision.fidelity, Fidelity::Narrated);
            assert_eq!(decision.reason_codes, ["unsupported_result_status"]);
        }
    }
}
