"""Grok current-format migration target."""
from __future__ import annotations

from ...events import event
from ...sessions.model import tool_result_text
from ...sessions.tool_ops import CANONICAL_OPS, CanonicalOp, has_valid_tool_input
from ..shared.migration import MigrationTargetBase
from .dialect import DIALECT
from .writer import write


class GrokMigrationTarget(MigrationTargetBase):
    tool = "grok"
    dialect = DIALECT
    tool_fidelity = {
        operation: (
            "native" if operation == CanonicalOp.TOOL_INVOKE
            or operation in DIALECT.write_ops() else "degrade"
        )
        for operation in CANONICAL_OPS
    }
    tool_result_statuses = frozenset({"success", "error", "pending"})
    tool_result_native_blocks = frozenset({"text"})
    tool_result_projected_blocks = frozenset({"json"})

    def preview_tool(self, tool, session, message=None):
        if message is not None and message.role == "user":
            return None
        if not has_valid_tool_input(tool.op, tool.input):
            return None
        rendered = self._dialect_preview(tool)
        if rendered is not None or tool.op == CanonicalOp.TOOL_INVOKE:
            return rendered
        # 方言尚无映射的操作:按源端名称与参数原样落地为外来工具记录。
        # 形态没变,身份变了,理由码要说清楚,否则差异卡上看不出改了什么。
        return {
            "kind": "tool", "name": tool.name, "input": tool.input,
            "output": tool_result_text(tool.result),
            "conversion": "transformed",
            "_consumed_fields": set(tool.input),
            "_reason_codes": ("foreign_tool_record",),
        }

    def plan(self, session):
        result = super().plan(session)
        compactions = sum(
            len(node.context_compactions) for node in session.walk()
        )
        if compactions:
            result["drop"] += compactions
            result["dropped"] += compactions
            result["drop_details"].append(event(
                "migration.content_dropped",
                kind="compaction", count=compactions,
            ))
        return result

    def write(self, session, cwd: str):
        return write(
            session, cwd, tool_decider=self.evaluate_tool,
        )
