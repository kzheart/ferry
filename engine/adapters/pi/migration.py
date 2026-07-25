"""Pi migration target."""
from __future__ import annotations

from ..shared.migration import MigrationTargetBase
from ...sessions.model import tool_result_text
from ...sessions.tool_ops import CanonicalOp, has_valid_tool_input
from .writer import OP_FIDELITY, OP_NAMES, _native_input, write


class PiMigrationTarget(MigrationTargetBase):
    tool = "pi"
    tool_fidelity = OP_FIDELITY
    tool_result_statuses = frozenset({"success", "error", "interrupted"})
    tool_result_native_blocks = frozenset({"text", "image"})

    def preview_tool(self, tool, session, message=None):
        if not has_valid_tool_input(tool.op, tool.input):
            return None
        if tool.op == CanonicalOp.TOOL_INVOKE:
            if tool.input["namespace"] not in {"pi", "mcp"}:
                return None
            name, value = tool.input["name"], tool.input["input"]
        elif tool.op in OP_NAMES:
            name, value = _native_input(tool)
        else:
            return None
        return {"kind": "tool", "name": name, "input": value,
                "output": tool_result_text(tool.result), "conversion": "native",
                "_consumed_fields": set(tool.input)}

    def write(self, session, cwd: str):
        return write(session, cwd, tool_decider=self.evaluate_tool)
