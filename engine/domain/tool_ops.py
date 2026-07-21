"""Canonical tool-operation contract shared by every agent adapter.

Adapters normalize native calls into these operations. Writers declare which
operations they can preserve natively or only render as a degradation, so the
migration preview and the actual write path share one vocabulary.
"""
from dataclasses import dataclass
from typing import Final


class CanonicalOp:
    SHELL_EXEC: Final = "shell.exec"
    FS_READ: Final = "fs.read"
    FS_WRITE: Final = "fs.write"
    FS_EDIT: Final = "fs.edit"
    AGENT_SPAWN: Final = "agent.spawn"


@dataclass(frozen=True)
class ToolOpSpec:
    required_inputs: tuple[str, ...]
    optional_inputs: tuple[str, ...] = ()


TOOL_OP_SPECS: Final = {
    CanonicalOp.SHELL_EXEC: ToolOpSpec(("command",), ("workdir",)),
    CanonicalOp.FS_READ: ToolOpSpec(("file_path",)),
    CanonicalOp.FS_WRITE: ToolOpSpec(("file_path", "content")),
    CanonicalOp.FS_EDIT: ToolOpSpec(("file_path", "old", "new")),
    CanonicalOp.AGENT_SPAWN: ToolOpSpec(
        ("description", "prompt", "subagent_type")),
}

CANONICAL_OPS: Final = frozenset(TOOL_OP_SPECS)
