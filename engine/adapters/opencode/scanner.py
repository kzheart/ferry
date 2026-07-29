"""OpenCode SQLite 存储扫描。"""

import hashlib
import json
import logging
import os
import sqlite3
import threading
import time
from pathlib import Path

from ...sessions.topology import session_roots
from ...sessions.usage import add_tokens, dominant_model, empty_tokens, has_tokens
from ...system.paths import opencode_database_path
from ...system.snapshots import data_dir

OPENCODE_DB = opencode_database_path()
_FINGERPRINT_INDEX: tuple | None = None
# 全量扫描的规范化线程会并发进来;重建要全量读库,不加锁会同时重建 N 遍。
_FINGERPRINT_LOCK = threading.Lock()
_REBUILD_THREAD: threading.Thread | None = None
_REBUILD_SCHEDULE_LOCK = threading.Lock()
_FINGERPRINT_STORE_VERSION = 1


def _fingerprint_store_path() -> Path:
    """指纹索引的跨进程缓存位置:重建要整库读+逐行哈希(上千会话约 5s),
    进程内缓存冷启动永远是空的,不落盘的话每次开机都要白付一遍。"""
    return data_dir() / "opencode-fingerprints.json"

log = logging.getLogger(__name__)


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
    # 不含 -shm:它只是 WAL 的共享内存索引,连只读连接都会更新其 mtime。
    # 把它算进戳记会让指纹缓存被自己的读取动作反复失效,每次扫描都全量
    # 重读整个库;数据变更必然体现在 .db 或 -wal 上,排除它不损失正确性。
    paths = (
        OPENCODE_DB,
        OPENCODE_DB.with_name(OPENCODE_DB.name + "-wal"),
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


def _stamp_key(stamp: tuple) -> list:
    """stamp 是嵌套 tuple,JSON 落盘后变嵌套 list;统一成 list 再比较。"""
    return [list(entry) for entry in stamp]


def _load_fingerprint_store() -> tuple | None:
    """读回落盘快照,stamp 原样带回;新鲜度由调用方裁决。"""
    try:
        data = json.loads(_fingerprint_store_path().read_text())
    except (OSError, json.JSONDecodeError) as error:
        log.info("opencode 指纹快照不可读: %s", error)
        return None
    if (
        not isinstance(data, dict)
        or data.get("version") != _FINGERPRINT_STORE_VERSION
        or data.get("db") != str(OPENCODE_DB)
    ):
        log.info(
            "opencode 指纹快照不匹配: version=%s db=%s (期望 %s)",
            data.get("version") if isinstance(data, dict) else type(data),
            data.get("db") if isinstance(data, dict) else None,
            OPENCODE_DB,
        )
        return None
    stamp_raw = data.get("stamp")
    sessions_raw = data.get("sessions")
    revisions_raw = data.get("revisions")
    if (
        not isinstance(stamp_raw, list)
        or not isinstance(sessions_raw, dict)
        or not isinstance(revisions_raw, dict)
        or set(sessions_raw) != set(revisions_raw)
    ):
        log.info("opencode 指纹快照结构无效")
        return None
    try:
        stamp = tuple(tuple(entry) for entry in stamp_raw)
        revisions = {
            sid: bytes.fromhex(value)
            for sid, value in revisions_raw.items()
        }
    except (TypeError, ValueError) as error:
        log.info("opencode 指纹快照解析失败: %s", error)
        return None
    sessions = {
        sid: None if parent is None else str(parent)
        for sid, parent in sessions_raw.items()
    }
    children: dict[str, list[str]] = {}
    for sid, parent in sessions.items():
        if parent is not None:
            children.setdefault(parent, []).append(sid)
    return (stamp, sessions, revisions, children)


def _save_fingerprint_store(index: tuple) -> None:
    stamp, sessions, revisions, _children = index
    payload = json.dumps({
        "version": _FINGERPRINT_STORE_VERSION,
        "db": str(OPENCODE_DB),
        "stamp": _stamp_key(stamp),
        "sessions": sessions,
        "revisions": {sid: digest.hex() for sid, digest in revisions.items()},
    })
    try:
        store = _fingerprint_store_path()
        store.parent.mkdir(parents=True, exist_ok=True)
        temp = store.with_name(
            f"{store.name}.{os.getpid()}.{threading.get_ident()}.tmp",
        )
        temp.write_text(payload)
        os.replace(temp, store)
    except OSError:
        log.warning("opencode 指纹索引落盘失败", exc_info=True)


def _rebuild_index_locked(trigger: str = "unknown") -> tuple | None:
    """全量重建指纹索引;必须在持有 _FINGERPRINT_LOCK 时调用。"""
    global _FINGERPRINT_INDEX
    current = None
    # 重建应只出现在:background(扫描收尾)/strict(Agent 钉内容)/
    # cold-no-store(冷启动且落盘不可用)。其它触发方即是泄漏,记名字。
    log.info("opencode 指纹索引重建触发: %s", trigger)
    rebuild_started = time.monotonic()
    stable = False
    for _attempt in range(3):
        before = _database_stamp()
        sessions, revisions, children = _read_fingerprint_index()
        after = _database_stamp()
        current = (after, sessions, revisions, children)
        stable = before == after
        if stable:
            _FINGERPRINT_INDEX = current
            _save_fingerprint_store(current)
            break
    # 重建要全量读库;stamp 不稳定意味着缓存没能写入,下一次调用还会
    # 整个重来——这正是需要被看见的信号。
    log.info(
        "opencode 指纹索引重建: %d 会话 耗时=%.1fs 缓存%s",
        len(current[1]) if current else 0,
        time.monotonic() - rebuild_started,
        "已写入" if stable else "未写入(stamp 不稳定)",
    )
    return current


def _background_rebuild() -> None:
    try:
        with _FINGERPRINT_LOCK:
            current = _FINGERPRINT_INDEX
            if current is not None and current[0] == _database_stamp():
                return
            _rebuild_index_locked("background")
    except Exception:  # noqa: BLE001 - 后台重建失败留给下一轮扫描重试
        log.exception("opencode 指纹索引后台重建失败")


def _schedule_background_rebuild() -> None:
    global _REBUILD_THREAD
    with _REBUILD_SCHEDULE_LOCK:
        if _REBUILD_THREAD is not None and _REBUILD_THREAD.is_alive():
            return
        thread = threading.Thread(
            target=_background_rebuild,
            name="opencode-fingerprint-rebuild",
            daemon=True,
        )
        _REBUILD_THREAD = thread
        thread.start()


def _current_index(allow_stale: bool) -> tuple | None:
    global _FINGERPRINT_INDEX
    stamp = _database_stamp()
    cached = _FINGERPRINT_INDEX
    if cached is not None and cached[0] == stamp:
        return cached
    if cached is None:
        with _FINGERPRINT_LOCK:
            cached = _FINGERPRINT_INDEX
            if cached is None:
                # 进程冷启动:先吃上一次进程落盘的成果。库没变它就是新鲜的;
                # 库变了它仍可作为扫描路径的旧快照,严格路径会按 stamp 重建。
                loaded = _load_fingerprint_store()
                if loaded is not None:
                    _FINGERPRINT_INDEX = cached = loaded
            if cached is None:
                # 连旧快照都没有,stale 与否都只能同步建一次。
                return _rebuild_index_locked(
                    f"cold-no-store(allow_stale={allow_stale})",
                )
        if cached is not None and cached[0] == _database_stamp():
            return cached
    # 有旧快照但库已变化:扫描路径吃旧快照,重建由扫描收尾的
    # ensure_fingerprint_index_fresh 统一调度;严格路径同步重建。
    if allow_stale:
        return cached
    with _FINGERPRINT_LOCK:
        current = _FINGERPRINT_INDEX
        if current is not None and current[0] == _database_stamp():
            return current
        return _rebuild_index_locked("strict")


def scan_fingerprint(session_id: str) -> str | None:
    """扫描路径的指纹:容忍落后一轮的快照。

    全量扫描对每个会话都要指纹,库一有写入就同步整库重建会把每次
    刷新拖慢数秒。扫描期间吃旧快照:UI 列表新鲜度由 session 表的
    updated 保证,Agent 读写路径仍走严格的 fingerprint()。重建不在
    这里调度——它持 GIL 读整库,与扫描并行会把扫描拖慢数倍,由
    扫描收尾的 ensure_fingerprint_index_fresh 触发。
    """
    return _subtree_fingerprint(session_id, allow_stale=True)


def ensure_fingerprint_index_fresh() -> None:
    """扫描收尾钩子:快照落后于库时在后台补一次重建。"""
    if not OPENCODE_DB.exists():
        return
    cached = _FINGERPRINT_INDEX
    if cached is None or cached[0] != _database_stamp():
        _schedule_background_rebuild()


def fingerprint(session_id: str) -> str | None:
    """以会话子树的修订元数据校验 Agent 引用。

    粒度必须是会话级:所有 OpenCode 会话同住一个 SQLite 库,若把整库
    stat 混入指纹,任何其他会话的写入都会让本会话的引用与迁移计划失效。
    """
    return _subtree_fingerprint(session_id, allow_stale=False)


def _subtree_fingerprint(session_id: str, *, allow_stale: bool) -> str | None:
    if not OPENCODE_DB.exists():
        return None
    cached = _current_index(allow_stale)
    if cached is None:
        return None
    _stamp, sessions, revisions, children = cached
    if session_id not in sessions:
        if allow_stale and cached[0] != _database_stamp():
            # 快照落后于库:比快照更新的会话还没进快照。给占位指纹保住
            # 索引条目(否则新会话会从扫描结果里消失),后台重建收敛后
            # 下一轮扫描会替换为真实指纹;钉内容路径对占位值必然失配,
            # 语义安全。
            return "sha256:pending-" + session_id
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
