"""OpenCode 作为迁移目标的写入与规划能力。"""
from __future__ import annotations

from ..base.migration import MigrationTargetBase
from .session import OP_FIDELITY, write


class OpenCodeMigrationTarget(MigrationTargetBase):
    tool = "opencode"
    tool_fidelity = OP_FIDELITY

    def write(self, session, cwd: str):
        return write(session, cwd=cwd)
