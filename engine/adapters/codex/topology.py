"""Codex rollout 索引与子 Agent 拓扑恢复。"""

from __future__ import annotations

import json
import sqlite3
from pathlib import Path
from typing import Callable

from ...sessions.model import AgentEdge, Session, ToolCall, tool_result_text
from ...sessions.tool_ops import CanonicalOp
from ...sessions.scan_cache import ScanCache

_META_CACHE_PATH = Path.home() / ".resume-harness" / "rollout-meta-cache.json"


def session_id(meta: dict, fallback: str) -> str:
    del fallback
    return str(meta["id"])


def _subagent_meta(meta: dict) -> dict:
    source = meta.get("source")
    if not isinstance(source, dict):
        return {}
    subagent = source.get("subagent", {})
    return subagent if isinstance(subagent, dict) else {}


def identity(meta: dict, fallback: str) -> dict:
    subagent = _subagent_meta(meta)
    spawn = subagent.get("thread_spawn", {})
    if not isinstance(spawn, dict):
        spawn = {}
    current_id = session_id(meta, fallback)
    root_id = meta.get("session_id") or spawn.get("session_id") or current_id
    parent_id = (
        meta.get("parent_thread_id")
        or spawn.get("parent_thread_id")
        or subagent.get("parent_thread_id")
    )
    return {
        "id": current_id,
        "root_id": root_id,
        "parent_id": parent_id,
        "forked_from_id": (
            meta.get("forked_from_id") or spawn.get("forked_from_id") or parent_id
        ),
        "agent_id": (
            subagent.get("agent_id") or spawn.get("agent_id") or meta.get("agent_id")
        ),
        "agent_path": (
            subagent.get("agent_path")
            or spawn.get("agent_path")
            or meta.get("agent_path")
        ),
        "agent_type": (
            subagent.get("agent_type")
            or spawn.get("agent_type")
            or meta.get("agent_type")
        ),
        "agent_nickname": (spawn.get("agent_nickname") or meta.get("agent_nickname")),
        "agent_role": spawn.get("agent_role") or meta.get("agent_role"),
        "model_provider": meta.get("model_provider"),
        "model": meta.get("model"),
        "depth": subagent.get("depth", spawn.get("depth")),
    }


def _first_meta(path: Path) -> dict:
    try:
        with path.open() as stream:
            for line in stream:
                if not line.strip():
                    continue
                record = json.loads(line)
                if record.get("type") == "session_meta":
                    return record.get("payload") or {}
    except (OSError, json.JSONDecodeError):
        pass
    return {}


def sessions_root(path: Path) -> Path:
    for parent in (path.parent, *path.parents):
        if parent.name == "sessions":
            return parent
    return path.parent


def rollout_index(
    path: Path,
    sessions_dir: str | Path | None,
) -> dict[str, tuple[Path, dict]]:
    root = Path(sessions_dir).expanduser() if sessions_dir else sessions_root(path)
    candidates = list(root.rglob("rollout*.jsonl")) if root.exists() else []
    if path not in candidates:
        candidates.append(path)
    cache = ScanCache(_META_CACHE_PATH, version=2)
    dirty = False
    index = {}
    for candidate in candidates:
        try:
            stat = candidate.stat()
        except OSError:
            continue
        ident = cache.get(candidate, stat)
        if ident is None:
            meta = _first_meta(candidate)
            ident = identity(meta, candidate.stem) if meta else {}
            cache.put(candidate, stat, ident)
            dirty = True
        if ident:
            index[ident["id"]] = (candidate, ident)
    if dirty:
        try:
            cache.flush()
        except OSError:
            pass
    return index


def _spawn_calls(session: Session) -> list[ToolCall]:
    return [
        block.tool
        for message in session.messages
        for block in message.blocks
        if block.kind == "tool"
        and block.tool
        and block.tool.op == CanonicalOp.AGENT_SPAWN
    ]


def _contains_identity(tool: ToolCall, child: Session) -> bool:
    values = [child.source_id, child.agent_id, child.agent_path]
    haystack = json.dumps(
        {
            "input": tool.input,
            "output": tool_result_text(tool.result),
        },
        ensure_ascii=False,
    )
    return any(value and value in haystack for value in values)


def _canonical_edge_status(value: str | None) -> str | None:
    if value in {"open", "closed"}:
        return value
    if value in {"completed", "failed", "cancelled", "canceled"}:
        return "closed"
    if value in {"in_progress", "queued"}:
        return "open"
    return None


def _attach_tree(
    session: Session,
    by_parent: dict[str, list[Session]],
    seen: set[str],
    edge_statuses: dict[str, str],
):
    if session.source_id in seen:
        return
    seen.add(session.source_id)
    spawn_calls = _spawn_calls(session)
    candidates = list(by_parent.get(session.source_id, []))
    ordered_children = []
    selected_children: set[int] = set()
    for tool in spawn_calls:
        child = next(
            (
                candidate
                for candidate in candidates
                if id(candidate) not in selected_children
                and _contains_identity(tool, candidate)
            ),
            None,
        )
        if child is not None:
            selected_children.add(id(child))
            ordered_children.append(child)
    ordered_children.extend(
        candidate for candidate in candidates if id(candidate) not in selected_children
    )
    used_calls: set[int] = set()
    for child in ordered_children:
        if child.source_id in seen:
            continue
        matched = next(
            (
                tool
                for tool in spawn_calls
                if id(tool) not in used_calls and _contains_identity(tool, child)
            ),
            None,
        )
        if matched:
            used_calls.add(id(matched))
        elif spawn_calls:
            session.lose(
                "session.subagent_unlinked",
                child_id=child.source_id,
            )
        prompt = ""
        if matched and isinstance(matched.input, dict):
            prompt = str(matched.input.get("prompt") or "")
        edge = AgentEdge(
            parent_session_id=session.source_id,
            child_session_id=child.source_id,
            source_call_id=matched.source_call_id if matched else None,
            spawn_message_id=matched.source_message_id if matched else None,
            result_message_id=matched.source_result_id if matched else None,
            agent_id=child.agent_id,
            agent_path=child.agent_path,
            agent_type=child.agent_type,
            prompt=prompt,
            status=(
                _canonical_edge_status(matched.result.status)
                if matched and matched.result
                else None
            )
            or edge_statuses.get(child.source_id),
            association=(
                "spawn-call"
                if matched
                else child.parent_association or "parent-metadata"
            ),
            confidence=(
                1.0
                if matched
                else 0.95
                if child.parent_association == "sqlite-parent"
                else 0.75
            ),
        )
        session.children.append(child)
        session.agent_edges.append(edge)
        _attach_tree(child, by_parent, seen, edge_statuses)


def _registry_edges(
    root: Path,
) -> dict[str, tuple[str, str]]:
    db_path = root.parent / "state_5.sqlite"
    if not db_path.exists():
        return {}
    try:
        with sqlite3.connect(
            f"file:{db_path.resolve()}?mode=ro",
            uri=True,
        ) as database:
            return {
                str(child): (str(parent), str(status))
                for parent, child, status in database.execute(
                    "SELECT parent_thread_id, child_thread_id, status "
                    "FROM thread_spawn_edges"
                )
            }
    except sqlite3.Error:
        return {}


def read_tree(
    rollout: Path,
    read_one: Callable[[Path], Session],
    sessions_dir: str | Path | None = None,
) -> Session:
    index = rollout_index(rollout, sessions_dir)
    root = read_one(rollout)
    registry_edges = _registry_edges(sessions_root(rollout))
    sessions = {root.source_id: root}
    reachable = {root.source_id}
    while True:
        added = False
        for current_id, (candidate, ident) in index.items():
            registry_parent = registry_edges.get(
                current_id,
                (None, None),
            )[0]
            parent_id = ident["parent_id"] or registry_parent
            if current_id in reachable or parent_id not in reachable:
                continue
            reachable.add(current_id)
            child = read_one(candidate)
            if child.parent_id is None and registry_parent:
                child.parent_id = registry_parent
                child.parent_association = "sqlite-parent"
            sessions[current_id] = child
            added = True
        if not added:
            break
    by_parent: dict[str, list[Session]] = {}
    for candidate in sessions.values():
        if candidate.parent_id:
            by_parent.setdefault(candidate.parent_id, []).append(candidate)
    for children in by_parent.values():
        children.sort(
            key=lambda child: (
                child.agent_path or "",
                child.source_id,
            )
        )
    _attach_tree(
        root,
        by_parent,
        set(),
        {child: status for child, (_parent, status) in registry_edges.items()},
    )
    return root
