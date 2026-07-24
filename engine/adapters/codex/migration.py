"""Codex 作为迁移目标的写入与规划能力。"""
from __future__ import annotations

import shlex

from ..base.migration import MigrationTargetBase, linked_agent_edge
from ...sessions.model import tool_result_text
from ...sessions.tool_ops import CanonicalOp, has_valid_tool_input
from .writer import OP_FIDELITY, write


class CodexMigrationTarget(MigrationTargetBase):
    tool = "codex"
    tool_fidelity = OP_FIDELITY

    def preview_tool(self, tool, session, message=None):
        if not has_valid_tool_input(tool.op, tool.input):
            return None
        inputs = tool.input
        if tool.op == CanonicalOp.SHELL_EXEC:
            supported = {"command", "workdir"}
            ignored = set(inputs) - supported
            return {"kind": "tool", "name": "exec", "input": {
                "cmd": inputs["command"], "workdir": inputs.get("workdir", session.cwd)},
                "output": tool_result_text(tool.result), "conversion": "native",
                "_consumed_fields": set(inputs) - ignored,
                "_ignored_fields": ignored,
                "_reason_codes": ("unsupported_tool_fields",) if ignored else ()}
        if tool.op == CanonicalOp.FS_READ:
            ignored = set(inputs) - {"file_path"}
            return {"kind": "tool", "name": "exec", "input": {
                "cmd": f"cat {shlex.quote(str(inputs['file_path']))}",
                "workdir": session.cwd},
                "output": tool_result_text(tool.result), "conversion": "transformed",
                "_consumed_fields": {"file_path"},
                "_ignored_fields": ignored,
                "_fidelity": "lossy" if ignored else "transformed",
                "_reason_codes": (("tool_transformed", "unsupported_tool_fields")
                                  if ignored else ("tool_transformed",))}
        if tool.op in {CanonicalOp.FS_WRITE, CanonicalOp.FS_EDIT}:
            supported = ({"file_path", "content"} if
                         tool.op == CanonicalOp.FS_WRITE else
                         {"file_path", "old", "new"})
            ignored = set(inputs) - supported
            return {"kind": "tool", "name": "apply_patch", "input": inputs,
                    "output": tool_result_text(tool.result), "conversion": "native",
                    "_consumed_fields": set(inputs) - ignored,
                    "_ignored_fields": ignored,
                    "_reason_codes": ("unsupported_tool_fields",) if ignored else ()}
        if tool.op == CanonicalOp.FS_PATCH:
            if not inputs.get("raw_patch"):
                return None
            ignored = set(inputs) - {"operations", "raw_patch"}
            return {
                "kind": "tool", "name": "apply_patch",
                "input": {"patch": inputs["raw_patch"]},
                "output": tool_result_text(tool.result), "conversion": "native",
                "_consumed_fields": set(inputs) - ignored,
                "_ignored_fields": ignored,
                "_reason_codes": ("unsupported_tool_fields",) if ignored else (),
            }
        if tool.op == CanonicalOp.FS_SEARCH:
            command = ["rg", "--line-number", "--color", "never"]
            if inputs.get("glob"):
                command.extend(["-g", str(inputs["glob"])])
            command.extend([
                "--", str(inputs["query"]), str(inputs.get("path") or ".")])
            ignored = set(inputs) - {"query", "path", "glob"}
            return {
                "kind": "tool", "name": "exec",
                "input": {
                    "cmd": " ".join(shlex.quote(part) for part in command),
                    "workdir": session.cwd,
                },
                "output": tool_result_text(tool.result), "conversion": "transformed",
                "_consumed_fields": set(inputs) - ignored,
                "_ignored_fields": ignored,
                "_fidelity": "lossy" if ignored else "transformed",
                "_reason_codes": (("tool_transformed", "unsupported_tool_fields")
                                  if ignored else ("tool_transformed",)),
            }
        if tool.op == CanonicalOp.FS_GLOB:
            command = ["rg", "--files", "-g", str(inputs["pattern"]), "--",
                       str(inputs.get("path") or ".")]
            ignored = set(inputs) - {"pattern", "path"}
            return {
                "kind": "tool", "name": "exec",
                "input": {
                    "cmd": " ".join(shlex.quote(part) for part in command),
                    "workdir": session.cwd,
                },
                "output": tool_result_text(tool.result), "conversion": "transformed",
                "_consumed_fields": set(inputs) - ignored,
                "_ignored_fields": ignored,
                "_fidelity": "lossy" if ignored else "transformed",
                "_reason_codes": (("tool_transformed", "unsupported_tool_fields")
                                  if ignored else ("tool_transformed",)),
            }
        if tool.op == CanonicalOp.AGENT_SPAWN:
            if not linked_agent_edge(session, tool, message, allow_message=True):
                return None
            supported = {"description", "prompt", "subagent_type"}
            ignored = set(inputs) - supported
            return {"kind": "tool", "name": "spawn_agent", "input": inputs,
                    "output": tool_result_text(tool.result), "conversion": "native",
                    "_consumed_fields": set(inputs) - ignored,
                    "_ignored_fields": ignored,
                    "_reason_codes": ("unsupported_tool_fields",) if ignored else ()}
        return None

    def write(self, session, cwd: str):
        return write(session, cwd=cwd, tool_decider=self.evaluate_tool)
