"""OpenCode SQLite 存储扫描。"""

import hashlib
import json
import sqlite3
from pathlib import Path

from ...domain.topology import session_roots
from ...domain.usage import add_tokens, dominant_model, empty_tokens, has_tokens

OPENCODE_DB = Path.home() / ".local/share/opencode/opencode.db"


def _msg_tokens(data: dict) -> dict:
    tokens = data.get("tokens") or {}
    cache = tokens.get("cache") or {}
    return {"input": tokens.get("input") or 0,
            "output": (tokens.get("output") or 0) + (tokens.get("reasoning") or 0),
            "cache_read": cache.get("read") or 0,
            "cache_write": cache.get("write") or 0}


def _aggregate_usage(database) -> dict:
    """从 message 表按会话累加 token(session 表的 rollup 列覆盖不全)。"""
    by_session: dict[str, dict] = {}
    for sid, blob in database.execute("SELECT session_id, data FROM message"):
        try:
            data = json.loads(blob)
        except (json.JSONDecodeError, TypeError):
            continue
        if data.get("role") != "assistant":
            continue
        model = data.get("modelID") or data.get("model") or ""
        if not model and not data.get("tokens"):
            continue
        by_model = by_session.setdefault(sid, {})
        add_tokens(by_model.setdefault(model, empty_tokens()), _msg_tokens(data))
    return by_session


def scan(_cache):
    if not OPENCODE_DB.exists():
        return []
    uri = f"file:{OPENCODE_DB}?mode=ro"
    with sqlite3.connect(uri, uri=True, timeout=5) as database:
        counts = dict(database.execute("SELECT session_id, COUNT(*) FROM message GROUP BY session_id"))
        usage = _aggregate_usage(database)
        records = database.execute(
            "SELECT id, title, directory, time_updated, time_created, parent_id FROM session").fetchall()

    rows = []
    for sid, title, directory, updated, created, parent in records:
        by_model = usage.get(sid, {})
        tokens = empty_tokens()
        for model_tokens in by_model.values():
            add_tokens(tokens, model_tokens)
        rows.append({"tool": "opencode", "id": sid, "title": title or "",
            "dir": directory or "", "updated": updated or 0, "created": created or None,
            "count": counts.get(sid, 0), "size": 0, "path": "", "parent_id": parent,
            "tokens": tokens if has_tokens(tokens) else None,
            "model": dominant_model(by_model)})
    return [root for root in session_roots(rows) if root["count"]]


def fingerprint(session_id: str) -> str | None:
    """保守地指纹化整个存储，覆盖递归子会话和 task 关联。"""
    if not OPENCODE_DB.exists():
        return None
    digest = hashlib.sha256()
    uri = f"file:{OPENCODE_DB.resolve()}?mode=ro"
    with sqlite3.connect(uri, uri=True, timeout=5) as database:
        tables = {row[0] for row in database.execute(
            "SELECT name FROM sqlite_master WHERE type='table'")}
        if "session" not in tables:
            return None
        found = bool(database.execute(
            'SELECT 1 FROM "session" WHERE "id" = ? LIMIT 1',
            (session_id,)).fetchone())
        if not found:
            return None
        for table in ("session", "message", "part"):
            if table not in tables:
                continue
            rows = database.execute(
                f'SELECT * FROM "{table}" ORDER BY 1').fetchall()
            digest.update(table.encode())
            digest.update(b"\0")
            for row in rows:
                digest.update(repr(tuple(row)).encode())
                digest.update(b"\0")
    return "sha256:" + digest.hexdigest()
