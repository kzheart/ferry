"""Ferry Runtime 会话事件的 Engine SQLite 端口。

这里仅持久化 Runtime 已脱敏的 JSON 记录，不解释 Provider、Role 或 AgentMessage。
"""
from __future__ import annotations

from pathlib import Path

from ..domain.errors import AgentRequestError
from ..infrastructure.state_db import StateDatabase
from .ports import ApplicationPorts


def _database(ports: ApplicationPorts) -> StateDatabase:
    return StateDatabase(
        Path(ports.snapshot_dir()) / "ferry-state.sqlite3",
        recover_interrupted=False,
    )


def load_all(ports: ApplicationPorts) -> list[dict]:
    return _database(ports).load_runtime_sessions()


def commit(update: dict, ports: ApplicationPorts) -> dict:
    if not isinstance(update, dict):
        raise AgentRequestError("runtime commit 必须是 object")
    metadata = update.get("metadata")
    if not isinstance(metadata, dict) or not isinstance(metadata.get("session_id"), str):
        raise AgentRequestError("runtime commit 缺少 metadata.session_id")
    if not isinstance(update.get("timestamp"), str):
        raise AgentRequestError("runtime commit 缺少 timestamp")
    for key in ("messages", "events"):
        if key in update and not isinstance(update[key], list):
            raise AgentRequestError(f"runtime commit 的 {key} 必须是数组")
    _database(ports).commit_runtime_session(update)
    return {"session_id": metadata["session_id"], "committed": True}


def delete(session_id: str, ports: ApplicationPorts) -> dict:
    return {"session_id": session_id,
            "deleted": _database(ports).delete_runtime_session(session_id)}
