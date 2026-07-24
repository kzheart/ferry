"""会话删除与恢复用例。"""
from __future__ import annotations

import json
from pathlib import Path

from ..domain.errors import SnapshotInvalidSourceError
from .ports import ApplicationPorts


class SessionDeletionService:
    def __init__(self, ports: ApplicationPorts):
        self._ports = ports

    def delete(self, tool: str, reference: str) -> dict:
        adapter = self._ports.adapter(tool)
        return adapter.lifecycle.delete(adapter, reference)

    def restore(self, snapshot: str) -> dict:
        path = Path(snapshot)
        if path.parent != Path(self._ports.snapshot_dir()):
            raise SnapshotInvalidSourceError(
                "只允许从快照目录恢复", {"snapshot": snapshot})
        try:
            metadata = json.loads(path.with_suffix(".meta.json").read_text())
        except (OSError, json.JSONDecodeError) as error:
            raise SnapshotInvalidSourceError(
                "快照缺少元数据,无法撤销", {"snapshot": snapshot}) from error
        tool = metadata.get("tool")
        if not isinstance(tool, str) or not tool:
            raise SnapshotInvalidSourceError(
                "快照缺少来源 Agent", {"snapshot": snapshot})
        return self._ports.adapter(tool).lifecycle.restore_delete(path, metadata)
