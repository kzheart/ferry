"""OpenCode SQLite 存储扫描。"""

import hashlib
import json
import sqlite3

from ...sessions.topology import session_roots
from ...sessions.usage import add_tokens, dominant_model, empty_tokens, has_tokens
from ...system.paths import opencode_database_path

OPENCODE_DB = opencode_database_path()
_FINGERPRINT_INDEX: tuple | None = None


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


def _database_stamp() -> tuple:
    paths = (
        OPENCODE_DB,
        OPENCODE_DB.with_name(OPENCODE_DB.name + "-wal"),
        OPENCODE_DB.with_name(OPENCODE_DB.name + "-shm"),
    )
    stamp = []
    for path in paths:
        try:
            stat = path.stat()
        except OSError:
            stamp.append((str(path), None))
            continue
        stamp.append((
            str(path), stat.st_dev, stat.st_ino,
            stat.st_mtime_ns, stat.st_size,
        ))
    return tuple(stamp)


def _hash_row(digest, table: str, row) -> None:
    payload = json.dumps(
        [table, *row], ensure_ascii=False, default=str,
        separators=(",", ":"),
    ).encode()
    digest.update(len(payload).to_bytes(8, "big"))
    digest.update(payload)


def _read_fingerprint_index() -> tuple[dict, dict, dict]:
    uri = f"file:{OPENCODE_DB.resolve()}?mode=ro"
    with sqlite3.connect(uri, uri=True, timeout=5) as database:
        database.execute("BEGIN")
        tables = {row[0] for row in database.execute(
            "SELECT name FROM sqlite_master WHERE type='table'")}
        if "session" not in tables:
            return {}, {}, {}
        session_columns = [
            str(row[1]) for row in database.execute(
                'PRAGMA table_info("session")')
        ]
        session_id_index = session_columns.index("id")
        parent_index = session_columns.index("parent_id")
        sessions = {}
        digests = {}
        for row in database.execute('SELECT * FROM "session" ORDER BY "id"'):
            sid = str(row[session_id_index])
            parent = row[parent_index]
            sessions[sid] = None if parent is None else str(parent)
            digest = hashlib.sha256()
            _hash_row(digest, "session", row)
            digests[sid] = digest
        for table in ("message", "part"):
            if table not in tables:
                continue
            columns = [
                str(row[1]) for row in database.execute(
                    f'PRAGMA table_info("{table}")')
            ]
            session_index = columns.index("session_id")
            for row in database.execute(
                    f'SELECT * FROM "{table}"'
                    f' ORDER BY "session_id", "id"'):
                sid = str(row[session_index])
                if sid in digests:
                    _hash_row(digests[sid], table, row)
    children: dict[str, list[str]] = {}
    for sid, parent in sessions.items():
        if parent is not None:
            children.setdefault(parent, []).append(sid)
    return (
        sessions,
        {sid: digest.digest() for sid, digest in digests.items()},
        children,
    )


def fingerprint(session_id: str) -> str | None:
    """以会话子树的修订元数据校验 Agent 引用。

    粒度必须是会话级:所有 OpenCode 会话同住一个 SQLite 库,若把整库
    stat 混入指纹,任何其他会话的写入都会让本会话的引用与迁移计划失效。
    """
    if not OPENCODE_DB.exists():
        return None
    global _FINGERPRINT_INDEX
    stamp = _database_stamp()
    cached = _FINGERPRINT_INDEX
    if cached is None or cached[0] != stamp:
        current = None
        for _attempt in range(3):
            before = _database_stamp()
            sessions, revisions, children = _read_fingerprint_index()
            after = _database_stamp()
            current = (after, sessions, revisions, children)
            if before == after:
                _FINGERPRINT_INDEX = current
                break
        cached = current
    _stamp, sessions, revisions, children = cached
    if session_id not in sessions:
        return None
    digest = hashlib.sha256()
    pending, seen = [session_id], set()
    while pending:
        sid = pending.pop()
        if sid in seen:
            continue
        seen.add(sid)
        digest.update(f"\0{sid}\0{sessions[sid]}\0".encode())
        digest.update(revisions[sid])
        pending.extend(sorted(children.get(sid, ()), reverse=True))
    return "sha256:" + digest.hexdigest()
