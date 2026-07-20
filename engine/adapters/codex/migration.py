"""Codex 作为迁移目标的写入与规划能力。"""
from __future__ import annotations

from ..base.migration import MigrationTargetBase
from .writer import write


class CodexMigrationTarget(MigrationTargetBase):
    tool = "codex"

    def write(self, session, cwd: str):
        return write(session, cwd=cwd)
