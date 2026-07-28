"""Pi migration target."""
from __future__ import annotations

from ..shared.migration import MigrationTargetBase
from .dialect import DIALECT
from .writer import OP_FIDELITY, write


class PiMigrationTarget(MigrationTargetBase):
    tool = "pi"
    dialect = DIALECT
    tool_fidelity = OP_FIDELITY
    tool_result_statuses = frozenset({"success", "error", "interrupted"})
    tool_result_native_blocks = frozenset({"text", "image"})

    def write(self, session, cwd: str):
        return write(session, cwd, tool_decider=self.evaluate_tool)
