"""会话删除用例:永久删除,不留恢复快照。"""
from __future__ import annotations

from ..context import EngineContext
from ..errors import require_agent_capability


class SessionDeletionService:
    def __init__(self, ports: EngineContext):
        self._ports = ports

    def delete(self, tool: str, reference: str) -> dict:
        adapter = self._ports.adapter(tool)
        lifecycle = require_agent_capability(adapter, "delete", "lifecycle")
        return lifecycle.delete(adapter, reference)
