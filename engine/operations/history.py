"""迁移历史：由 Python Engine 独占的 SQLite 状态持久化。"""
from __future__ import annotations

import secrets
from ..context import EngineContext
from ..storage.database import state_database as _database

def append(entry: dict, ports: EngineContext) -> str:
    history_id = "history_" + secrets.token_urlsafe(18)
    _database(ports).migration_history.append(history_id, entry)
    return history_id


def list_entries(ports: EngineContext) -> list[dict]:
    return _database(ports).migration_history.list_all()


def delete(history_id: str, ports: EngineContext) -> dict:
    return _database(ports).migration_history.delete(history_id)
