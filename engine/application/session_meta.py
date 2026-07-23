"""Python Engine 独占的会话元数据存储。

Ferry 自有元数据位于 StateDatabase；不读取或迁移历史 JSON 文件。
"""
from __future__ import annotations

import time
from pathlib import Path

from ..domain.errors import ConcurrentModificationError
from ..infrastructure.state_db import StateDatabase
from .ports import current


def _database() -> StateDatabase:
    path = Path(current().snapshot_dir()) / "ferry-state.sqlite3"
    # 元数据调用不能把正在执行的 Operation 标为中断；该恢复动作仅由
    # OperationService 重启时执行。
    return StateDatabase(path, recover_interrupted=False)


def _now_ms() -> int:
    return int(time.time() * 1000)


def list_all() -> dict:
    return _database().list_session_metadata()


def set_entry(sid: str, patch: dict) -> dict:
    return _database().set_session_metadata(sid, patch, _now_ms())


def compare_and_set_entry(sid: str, expected: dict, patch: dict) -> dict:
    result = _database().compare_and_set_session_metadata(
        [(sid, expected, patch)], _now_ms(),
    )
    if result is None:
        raise ConcurrentModificationError("会话元数据在审批后已变化")
    return result[sid]


def compare_and_set_entries(changes: list[dict]) -> dict:
    encoded = [
        (change["id"], change.get("expected", {}), change.get("patch", {}))
        for change in changes
    ]
    result = _database().compare_and_set_session_metadata(encoded, _now_ms())
    if result is None:
        raise ConcurrentModificationError("会话元数据在整理提案审批后已变化")
    return result
