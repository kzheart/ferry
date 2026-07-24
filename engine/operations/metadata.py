"""Python Engine 独占的会话元数据存储。

Ferry 自有元数据位于 StateDatabase；不读取或迁移历史 JSON 文件。
"""
from __future__ import annotations

import time
from pathlib import Path

from ..application.ports import ApplicationPorts
from ..domain.errors import ConcurrentModificationError
from ..infrastructure.state_db import StateDatabase


def _database(ports: ApplicationPorts) -> StateDatabase:
    path = Path(ports.snapshot_dir()) / "ferry-state.sqlite3"
    # 元数据调用不能把正在执行的 Operation 标为中断；该恢复动作仅由
    # OperationService 重启时执行。
    return StateDatabase(path, recover_interrupted=False)


def _now_ms() -> int:
    return int(time.time() * 1000)


def list_all(ports: ApplicationPorts) -> dict:
    return _database(ports).list_session_metadata()


def key(tool: str, session_id: str) -> str:
    return StateDatabase.metadata_key(tool, session_id)


def set_entry(tool: str, session_id: str, patch: dict,
              ports: ApplicationPorts) -> dict:
    return _database(ports).set_session_metadata(tool, session_id, patch, _now_ms())


def compare_and_set_entry(
        tool: str, session_id: str, expected: dict, patch: dict,
        ports: ApplicationPorts,
) -> dict:
    result = _database(ports).compare_and_set_session_metadata(
        [(tool, session_id, expected, patch)], _now_ms(),
    )
    if result is None:
        raise ConcurrentModificationError("会话元数据在审批后已变化")
    return result[key(tool, session_id)]


def compare_and_set_entries(changes: list[dict], ports: ApplicationPorts) -> dict:
    encoded = [
        (
            change["tool"], change["id"],
            change.get("expected", {}), change.get("patch", {}),
        )
        for change in changes
    ]
    result = _database(ports).compare_and_set_session_metadata(encoded, _now_ms())
    if result is None:
        raise ConcurrentModificationError("会话元数据在整理提案审批后已变化")
    return result
