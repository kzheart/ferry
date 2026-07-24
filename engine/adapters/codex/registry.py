"""Codex state_5.sqlite 会话注册。"""
from __future__ import annotations

import json
import sqlite3
import time
from pathlib import Path

from ...sessions.model import Session


def _columns(db: sqlite3.Connection, table: str) -> list[tuple]:
    return db.execute(f"PRAGMA table_info({table})").fetchall()


def _first_user_message(session: Session) -> str:
    for message in session.messages:
        if message.role != "user":
            continue
        text = "\n".join(block.text for block in message.blocks
                         if block.kind == "text" and block.text)
        if text:
            return text
    return ""


def _insert(db: sqlite3.Connection, table: str, values: dict) -> None:
    schema = _columns(db, table)
    if not schema:
        raise RuntimeError(f"Codex 注册库缺少 {table} 表")
    available = {column[1] for column in schema}
    required = {column[1] for column in schema
                if column[3] and column[4] is None and not column[5]}
    missing = required - values.keys()
    if missing:
        raise RuntimeError(
            f"Codex 注册库包含不支持的必填字段: {', '.join(sorted(missing))}")
    row = {key: value for key, value in values.items() if key in available}
    names = list(row)
    placeholders = ",".join("?" for _ in names)
    db.execute(
        f"INSERT OR REPLACE INTO {table} ({','.join(names)}) VALUES ({placeholders})",
        tuple(row[name] for name in names),
    )


def register_tree(
        state_db: Path,
        nodes: list[tuple[Session, str, Path, str | None, str, str, str | None]],
        cli_version: str = "") -> None:
    """注册 writer 已发布的 (session, id, path, parent_id, cwd) 节点。"""
    if not state_db.exists():
        raise RuntimeError(f"Codex 注册库不存在: {state_db}")
    now = int(time.time())
    now_ms = int(time.time() * 1000)
    with sqlite3.connect(state_db, timeout=5) as db:
        db.execute("PRAGMA foreign_keys=ON")
        db.execute("BEGIN IMMEDIATE")
        for session, session_id, path, parent_id, cwd, agent_path, _status in nodes:
            first_user = _first_user_message(session)
            title = session.title or first_user[:80]
            source = "cli" if parent_id is None else json.dumps({
                "subagent": {"thread_spawn": {
                    "parent_thread_id": parent_id,
                    "agent_path": agent_path,
                    "agent_nickname": session.agent_nickname,
                    "agent_role": session.agent_role,
                }}
            }, ensure_ascii=False, separators=(",", ":"))
            _insert(db, "threads", {
                "id": session_id,
                "rollout_path": str(path.resolve()),
                "created_at": now,
                "updated_at": now,
                "created_at_ms": now_ms,
                "updated_at_ms": now_ms,
                "recency_at": now,
                "recency_at_ms": now_ms,
                "source": source,
                "model_provider": session.model_provider or "openai",
                "cwd": cwd,
                "title": title,
                "sandbox_policy": json.dumps({"type": "read-only"}),
                "approval_mode": "on-request",
                "tokens_used": 0,
                "has_user_event": int(bool(first_user)),
                "archived": 0,
                "cli_version": cli_version,
                "first_user_message": first_user,
                "agent_nickname": session.agent_nickname,
                "agent_role": session.agent_role,
                "agent_path": agent_path,
                "thread_source": "user" if parent_id is None else "subagent",
                "preview": first_user,
                "history_mode": "legacy",
            })
        if _columns(db, "thread_spawn_edges"):
            for _session, session_id, _path, parent_id, _cwd, _agent_path, status in nodes:
                if parent_id:
                    _insert(db, "thread_spawn_edges", {
                        "parent_thread_id": parent_id,
                        "child_thread_id": session_id,
                        "status": status or "closed",
                    })


def unregister_tree(state_db: Path | None, session_ids: set[str]) -> None:
    if not state_db or not state_db.exists() or not session_ids:
        return
    with sqlite3.connect(state_db, timeout=5) as db:
        placeholders = ",".join("?" for _ in session_ids)
        values = tuple(session_ids)
        if _columns(db, "thread_spawn_edges"):
            db.execute(
                f"DELETE FROM thread_spawn_edges WHERE parent_thread_id IN ({placeholders}) "
                f"OR child_thread_id IN ({placeholders})", values + values)
        if _columns(db, "threads"):
            db.execute(f"DELETE FROM threads WHERE id IN ({placeholders})", values)
