"""Claude 作为迁移目标的写入与规划能力。"""
from __future__ import annotations

from ..shared.migration import MigrationTargetBase, linked_agent_edge
from ...sessions.tool_ops import CanonicalOp, has_valid_tool_input
from .dialect import DIALECT
from .writer import OP_FIDELITY, write


class ClaudeMigrationTarget(MigrationTargetBase):
    tool = "claude"
    dialect = DIALECT
    tool_fidelity = OP_FIDELITY
    tool_result_statuses = frozenset({"success", "error", "interrupted"})
    tool_result_native_blocks = frozenset({
        "text", "image", "tool_reference",
    })
    tool_result_projected_blocks = frozenset({"json", "file"})

    def preview_tool(self, tool, session, message=None):
        if not has_valid_tool_input(tool.op, tool.input):
            return None
        if tool.op == CanonicalOp.AGENT_SPAWN and \
                not linked_agent_edge(session, tool):
            return None
        return self._dialect_preview(tool)

    def write(self, session, cwd: str):
        return write(session, cwd=cwd, tool_decider=self.evaluate_tool)
