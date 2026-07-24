"""迁移历史：由 Python Engine 独占的 SQLite 状态边界持久化。"""
from __future__ import annotations

import secrets
from pathlib import Path

from ..infrastructure.state_db import StateDatabase
from .ports import ApplicationPorts


def _database(ports: ApplicationPorts) -> StateDatabase:
    return StateDatabase(
        Path(ports.snapshot_dir()) / "ferry-state.sqlite3",
        recover_interrupted=False,
    )


def append(entry: dict, ports: ApplicationPorts) -> str:
    history_id = "history_" + secrets.token_urlsafe(18)
    _database(ports).append_migration_history(history_id, entry)
    return history_id


def list_entries(ports: ApplicationPorts) -> list[dict]:
    return _database(ports).list_migration_history()


def delete(history_id: str, ports: ApplicationPorts) -> dict:
    return _database(ports).delete_migration_history(history_id)
