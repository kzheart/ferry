"""Grok current-format migration target."""
from __future__ import annotations

from ...events import event
from ...sessions.model import tool_result_text
from ...sessions.tool_ops import CANONICAL_OPS, CanonicalOp
from ..shared.migration import MigrationTargetBase
from .writer import write


class GrokMigrationTarget(MigrationTargetBase):
    tool = "grok"
    tool_fidelity = {
        operation: (
            "native" if operation == CanonicalOp.TOOL_INVOKE else "degrade"
        )
        for operation in CANONICAL_OPS
    }
    tool_result_statuses = frozenset({"success", "error", "pending"})
    tool_result_native_blocks = frozenset({"text"})
    tool_result_projected_blocks = frozenset({"json"})

    def preview_tool(self, tool, session, message=None):
        if message is not None and message.role == "user":
            return None
        if tool.op == CanonicalOp.TOOL_INVOKE:
            value = tool.input
            if value.get("namespace") not in {"grok", "mcp"}:
                return None
            name, tool_input = value["name"], value["input"]
        else:
            name, tool_input = tool.name, tool.input
        return {
            "kind": "tool", "name": name, "input": tool_input,
            "output": tool_result_text(tool.result),
            "conversion": (
                "native" if tool.op == CanonicalOp.TOOL_INVOKE
                else "transformed"
            ),
            "_consumed_fields": set(tool.input),
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
