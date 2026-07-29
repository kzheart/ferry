"""Codex 原生 rollout 树克隆。

只重映射线程身份字段，未知记录和模型历史保持原样。迁移 writer 不参与此流程。
"""
from __future__ import annotations

import hashlib
import json
import shutil
import sqlite3
from dataclasses import dataclass, field
from pathlib import Path

from ..shared.scanner import split_jsonl_lines


class CodexCloneError(RuntimeError):
    pass


@dataclass(frozen=True)
class CodexStore:
    home: Path
    sessions_dir: Path
    state_db: Path | None

    @classmethod
    def for_rollout(cls, path: Path) -> "CodexStore":
        path = path.resolve()
        sessions = next((p for p in path.parents if p.name == "sessions"), None)
        if sessions is None:
            return cls(path.parent, path.parent, None)
        home = sessions.parent
        db = home / "state_5.sqlite"
        return cls(home, sessions, db if db.exists() else None)


@dataclass
class NativeRollout:
    thread_id: str
    root_id: str
    parent_id: str | None
    path: Path
    records: list[dict]
    digest: str


@dataclass
class CodexClosure:
    anchor_id: str
    root_id: str
    nodes: dict[str, NativeRollout]
    parents: dict[str, str]
    store: CodexStore
    revision: str
    registry_revision: str | None
    pruned_ids: set[str] = field(default_factory=set)


THREAD_KEYS = {
    "threadId", "thread_id", "agent_thread_id", "sender_thread_id",
    "new_thread_id", "parent_thread_id", "child_thread_id", "session_id",
    "forked_from_id",
}
JSON_STRING_KEYS = {"arguments", "output", "input", "metadata", "state"}


def _digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _read_jsonl(path: Path) -> tuple[list[dict], str]:
    raw = path.read_bytes()
    try:
        records = [json.loads(line) for line in split_jsonl_lines(raw.decode())
                   if line.strip()]
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise CodexCloneError(f"无法解析 Codex rollout: {path}: {error}") from error
    return records, _digest(raw)


def _canonical_meta(records: list[dict]) -> dict:
    for record in records:
        if record.get("type") == "session_meta" and isinstance(record.get("payload"), dict):
            return record["payload"]
    raise CodexCloneError("rollout 缺少 canonical session_meta")


def _identity(meta: dict, fallback: str) -> tuple[str, str, str | None]:
    del fallback
    source = meta.get("source") or {}
    source = source if isinstance(source, dict) else {}
    subagent = source.get("subagent") or {}
    subagent = subagent if isinstance(subagent, dict) else {}
    spawn = subagent.get("thread_spawn") or {}
    spawn = spawn if isinstance(spawn, dict) else {}
    current = meta["id"]
    root = meta.get("session_id") or spawn.get("session_id") or current
    parent = (meta.get("parent_thread_id") or spawn.get("parent_thread_id") or
              subagent.get("parent_thread_id"))
    return str(current), str(root), str(parent) if parent else None


def _rollout(path: Path) -> NativeRollout:
    records, digest = _read_jsonl(path)
    thread_id, root_id, parent_id = _identity(_canonical_meta(records), path.stem)
    return NativeRollout(thread_id, root_id, parent_id, path.resolve(), records, digest)


def _rollout_identity(path: Path) -> NativeRollout:
    """索引阶段只读到第一条 session_meta，避免加载全库所有历史正文。"""
    try:
        with path.open() as stream:
            for line in stream:
                if not line.strip():
                    continue
                record = json.loads(line)
                if record.get("type") == "session_meta" and isinstance(record.get("payload"), dict):
                    thread_id, root_id, parent_id = _identity(record["payload"], path.stem)
                    return NativeRollout(thread_id, root_id, parent_id,
                                         path.resolve(), [], "")
    except (OSError, json.JSONDecodeError) as error:
        raise CodexCloneError(f"无法索引 Codex rollout: {path}: {error}") from error
    raise CodexCloneError(f"rollout 缺少 canonical session_meta: {path}")


def _db_edges(db_path: Path | None) -> set[tuple[str, str]]:
    if db_path is None or not db_path.exists():
        return set()
    uri = f"file:{db_path.resolve()}?mode=ro"
    try:
        with sqlite3.connect(uri, uri=True) as db:
            exists = db.execute(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='thread_spawn_edges'"
            ).fetchone()
            if not exists:
                return set()
            return {(str(parent), str(child)) for parent, child in db.execute(
                "SELECT parent_thread_id, child_thread_id FROM thread_spawn_edges")}
    except sqlite3.Error as error:
        raise CodexCloneError(f"读取 Codex 注册库失败: {error}") from error


def discover_closure(anchor_path: Path, store: CodexStore | None = None) -> CodexClosure:
    anchor_path = anchor_path.resolve()
    store = store or CodexStore.for_rollout(anchor_path)
    candidates = (list(store.sessions_dir.rglob("rollout*.jsonl"))
                  if store.sessions_dir.name == "sessions" else [anchor_path])
    if anchor_path not in [p.resolve() for p in candidates]:
        candidates.append(anchor_path)

    index: dict[str, NativeRollout] = {}
    by_path = {}
    for path in candidates:
        try:
            node = _rollout_identity(path)
        except CodexCloneError:
            if path.resolve() == anchor_path:
                raise
            continue
        if node.thread_id in index and index[node.thread_id].path != node.path:
            raise CodexCloneError(
                f"重复 Codex thread id {node.thread_id}: {index[node.thread_id].path} / {node.path}")
        index[node.thread_id] = node
        by_path[node.path] = node
    anchor = by_path.get(anchor_path)
    if anchor is None:
        raise CodexCloneError(f"找不到目标 rollout: {anchor_path}")

    parents = {node.thread_id: node.parent_id for node in index.values() if node.parent_id}
    for parent, child in _db_edges(store.state_db):
        if child not in index or parent not in index:
            continue
        declared = parents.get(child)
        if declared and declared != parent:
            raise CodexCloneError(f"Codex 文件与 SQLite 父关系冲突: {child}")
        parents.setdefault(child, parent)

    root_id = anchor.thread_id
    seen = set()
    while root_id in parents:
        if root_id in seen:
            raise CodexCloneError("Codex 会话树存在父链环")
        seen.add(root_id)
        root_id = parents[root_id]
    if root_id not in index:
        raise CodexCloneError(f"Codex 根会话 rollout 缺失: {root_id}")

    children: dict[str, list[str]] = {}
    for child, parent in parents.items():
        children.setdefault(parent, []).append(child)
    reachable, stack = set(), [root_id]
    while stack:
        current = stack.pop()
        if current in reachable:
            raise CodexCloneError("Codex 会话树存在环或重复父节点")
        if current not in index:
            raise CodexCloneError(f"Codex 子会话 rollout 缺失: {current}")
        reachable.add(current)
        stack.extend(children.get(current, []))
    nodes = {sid: _rollout(index[sid].path) for sid in reachable}
    relevant_parents = {child: parent for child, parent in parents.items()
                        if child in reachable}

    h = hashlib.sha256()
    for sid, node in sorted(nodes.items()):
        h.update(sid.encode())
        h.update(b"\0")
        h.update(str(node.path).encode())
        h.update(b"\0")
        h.update(node.digest.encode())
        h.update(b"\0")
    registry_revision = _registry_revision(store.state_db, set(nodes))
    return CodexClosure(anchor.thread_id, root_id, nodes, relevant_parents,
                        store, "sha256:" + h.hexdigest(), registry_revision)



def collect_thread_refs(value, known_ids: set[str], key: str | None = None) -> set[str]:
    refs = set()
    if isinstance(value, dict):
        for child_key, child in value.items():
            refs.update(collect_thread_refs(child, known_ids, child_key))
    elif isinstance(value, list):
        for child in value:
            refs.update(collect_thread_refs(child, known_ids, key))
    elif key in THREAD_KEYS and isinstance(value, str) and value in known_ids:
        refs.add(value)
    elif key in JSON_STRING_KEYS and isinstance(value, str) and value[:1] in "[{":
        try:
            refs.update(collect_thread_refs(json.loads(value), known_ids))
        except json.JSONDecodeError:
            pass
    return refs


def prune_referenced_subtrees(closure: CodexClosure, records: list[dict]) -> set[str]:
    direct = {child for child, parent in closure.parents.items()
              if parent == closure.anchor_id}
    referenced = collect_thread_refs(records, direct)
    if not referenced:
        return set()
    children: dict[str, list[str]] = {}
    for child, parent in closure.parents.items():
        children.setdefault(parent, []).append(child)
    removed, stack = set(), list(referenced)
    while stack:
        current = stack.pop()
        if current in removed:
            continue
        removed.add(current)
        stack.extend(children.get(current, []))
    for sid in removed:
        closure.nodes.pop(sid, None)
        closure.parents.pop(sid, None)
    closure.pruned_ids.update(removed)
    closure.registry_revision = _registry_revision(
        closure.store.state_db, set(closure.nodes))
    h = hashlib.sha256()
    for sid, node in sorted(closure.nodes.items()):
        h.update(sid.encode())
        h.update(b"\0")
        h.update(str(node.path).encode())
        h.update(b"\0")
        h.update(node.digest.encode())
        h.update(b"\0")
    closure.revision = "sha256:" + h.hexdigest()
    return removed





def _table_columns(db: sqlite3.Connection, table: str) -> list[str]:
    return [row[1] for row in db.execute(f'PRAGMA table_xinfo("{table}")')
            if not row[6]]


def _registry_revision(db_path: Path | None, ids: set[str]) -> str | None:
    if db_path is None or not db_path.exists() or not ids:
        return None
    uri = f"file:{db_path.resolve()}?mode=ro"
    with sqlite3.connect(uri, uri=True) as db:
        snapshot = {}
        placeholders = ",".join("?" for _ in ids)
        for table, where, args in (
            ("threads", f"id IN ({placeholders})", tuple(ids)),
            ("thread_spawn_edges",
             f"parent_thread_id IN ({placeholders}) OR child_thread_id IN ({placeholders})",
             tuple(ids) + tuple(ids)),
            ("thread_dynamic_tools", f"thread_id IN ({placeholders})", tuple(ids)),
        ):
            if not _table_columns(db, table):
                continue
            columns = _table_columns(db, table)
            rows = db.execute(f'SELECT * FROM "{table}" WHERE {where}', args).fetchall()
            snapshot[table] = {"columns": columns, "rows": sorted(rows, key=repr)}
    raw = json.dumps(snapshot, ensure_ascii=False, sort_keys=True, default=str).encode()
    return "sha256:" + hashlib.sha256(raw).hexdigest()





def recover_transactions(store: CodexStore) -> None:
    directory = store.home / ".ferry" / "transactions"
    if not directory.exists():
        return
    for journal in directory.glob("*.json"):
        try:
            data = json.loads(journal.read_text())
            ids = list(data.get("ids", []))
            paths = [Path(value) for value in data.get("paths", [])]
            if store.state_db and store.state_db.exists() and ids:
                with sqlite3.connect(store.state_db, timeout=5) as db:
                    placeholders = ",".join("?" for _ in ids)
                    if _table_columns(db, "thread_spawn_edges"):
                        db.execute(f"DELETE FROM thread_spawn_edges WHERE parent_thread_id IN ({placeholders}) OR child_thread_id IN ({placeholders})",
                                   tuple(ids) + tuple(ids))
                    if _table_columns(db, "threads"):
                        db.execute(f"DELETE FROM threads WHERE id IN ({placeholders})", tuple(ids))
            sessions = store.sessions_dir.resolve()
            for path in paths:
                resolved = path.resolve()
                if sessions in resolved.parents and resolved.exists():
                    try:
                        node = _rollout_identity(resolved)
                    except CodexCloneError:
                        continue
                    if node.thread_id in ids:
                        resolved.unlink()
            shutil.rmtree(data.get("stage_dir", ""), ignore_errors=True)
            journal.unlink(missing_ok=True)
        except (OSError, ValueError, sqlite3.Error, json.JSONDecodeError):
            # 无法证明归属时不做破坏性恢复，保留日志供人工检查。
            continue
