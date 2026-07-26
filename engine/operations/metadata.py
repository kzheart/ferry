"""Python Engine 独占的会话元数据存储。

Ferry 自有元数据位于 StateDatabase；不读取或迁移历史 JSON 文件。
"""
from __future__ import annotations

from ..context import EngineContext
from ..errors import ConcurrentModificationError
from ..storage.database import now_ms, state_database as _database
from .metadata_store import metadata_key

def list_all(ports: EngineContext) -> dict:
    return _database(ports).metadata.list_all()


def key(tool: str, session_id: str) -> str:
    return metadata_key(tool, session_id)


def set_entry(tool: str, session_id: str, patch: dict,
              ports: EngineContext) -> dict:
    return _database(ports).metadata.set(
        tool, session_id, patch, now_ms(),
    )


def compare_and_set_entry(
        tool: str, session_id: str, expected: dict, patch: dict,
        ports: EngineContext,
) -> dict:
    result = _database(ports).metadata.compare_and_set(
        [(tool, session_id, expected, patch)], now_ms(),
    )
    if result is None:
        raise ConcurrentModificationError("会话元数据在审批后已变化")
    return result[key(tool, session_id)]

