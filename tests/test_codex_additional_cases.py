import json
import sqlite3
from pathlib import Path

import pytest

from engine.adapters.codex import reader as codex_reader
from engine.adapters.codex import writer as codex_writer
from engine.adapters.codex.writer import write
from engine.domain.model import AgentEdge, Block, Message, Session, ToolCall
from engine.domain.tool_ops import CanonicalOp


SCHEMA = """
CREATE TABLE threads (
    id TEXT PRIMARY KEY, rollout_path TEXT NOT NULL,
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
    source TEXT NOT NULL, model_provider TEXT NOT NULL, cwd TEXT NOT NULL,
    title TEXT NOT NULL, sandbox_policy TEXT NOT NULL,
    approval_mode TEXT NOT NULL, tokens_used INTEGER NOT NULL DEFAULT 0,
    has_user_event INTEGER NOT NULL DEFAULT 0, archived INTEGER NOT NULL DEFAULT 0,
    cli_version TEXT NOT NULL DEFAULT '', first_user_message TEXT NOT NULL DEFAULT '',
    agent_path TEXT, thread_source TEXT, preview TEXT NOT NULL DEFAULT '',
    recency_at INTEGER NOT NULL DEFAULT 0, history_mode TEXT NOT NULL DEFAULT 'legacy'
);
CREATE TABLE thread_spawn_edges (
    parent_thread_id TEXT NOT NULL, child_thread_id TEXT NOT NULL PRIMARY KEY,
    status TEXT NOT NULL
);
"""


def _store(tmp_path):
    home = tmp_path / ".codex"
    sessions = home / "sessions"
    sessions.mkdir(parents=True)
    database = home / "state_5.sqlite"
    with sqlite3.connect(database) as connection:
        connection.executescript(SCHEMA)
    return sessions, database


def _root_with_children(tmp_path, statuses=("running", "completed")):
    root = Session("claude", "root", str(tmp_path))
    root.messages = [Message("assistant", [Block("text", "delegate")],
                             source_id="anchor",
                             created_at="2026-07-22T04:17:28.433Z")]
    for index, status in enumerate(statuses, 1):
        child = Session("claude", f"child-{index}", str(tmp_path),
                        agent_path="/root/reviewer")
        child.messages = [Message("assistant", [Block("text", f"child {index}")])]
        root.children.append(child)
        root.agent_edges.append(AgentEdge(
            parent_session_id="root", child_session_id=child.source_id,
            spawn_message_id="anchor", status=status,
        ))
    return root


def test_codex_writer_handles_empty_and_tool_only_sessions(tmp_path, monkeypatch):
    sessions, _database = _store(tmp_path)
    monkeypatch.setattr(codex_reader, "_META_CACHE_PATH", tmp_path / "cache.json")

    empty_id, empty_path = write(Session("claude", "empty", str(tmp_path)), sessions_dir=sessions)
    assert codex_reader.read(str(empty_path), sessions_dir=sessions).source_id == empty_id

    tool_only = Session("claude", "tool-only", str(tmp_path))
    tool_only.messages = [Message("assistant", [Block("tool", tool=ToolCall(
        name="Bash", op=CanonicalOp.SHELL_EXEC, input={"command": "pwd"}, output="/tmp",
    ))], source_id="tool-message")]
    _tool_id, tool_path = write(tool_only, sessions_dir=sessions)
    restored = codex_reader.read(str(tool_path), sessions_dir=sessions)
    assert any(block.kind == "tool" for message in restored.messages for block in message.blocks)


def test_codex_writer_disambiguates_duplicate_agent_paths_and_maps_statuses(tmp_path, monkeypatch):
    sessions, database = _store(tmp_path)
    monkeypatch.setattr(codex_reader, "_META_CACHE_PATH", tmp_path / "cache.json")

    _root_id, path = write(_root_with_children(tmp_path), sessions_dir=sessions)
    restored = codex_reader.read(str(path), sessions_dir=sessions)

    assert [child.agent_path for child in restored.children] == [
        "/root/reviewer", "/root/reviewer-2",
    ]
    assert [edge.status for edge in restored.agent_edges] == ["open", "closed"]
    with sqlite3.connect(database) as connection:
        rows = connection.execute(
            "SELECT status FROM thread_spawn_edges ORDER BY child_thread_id"
        ).fetchall()
    assert sorted(status for status, in rows) == ["closed", "open"]


def test_codex_reader_tolerates_missing_or_invalid_registry(tmp_path, monkeypatch):
    sessions, database = _store(tmp_path)
    monkeypatch.setattr(codex_reader, "_META_CACHE_PATH", tmp_path / "cache.json")
    _root_id, path = write(_root_with_children(tmp_path, statuses=(None,)), sessions_dir=sessions)

    database.unlink()
    assert len(codex_reader.read(str(path), sessions_dir=sessions).children) == 1

    database.write_text("not a sqlite database")
    assert len(codex_reader.read(str(path), sessions_dir=sessions).children) == 1


def test_codex_writer_cleans_temporary_file_when_publish_fails(tmp_path, monkeypatch):
    sessions, _database = _store(tmp_path)
    original_rename = Path.rename

    def fail_publish(path, target):
        if path.suffix == ".tmp":
            raise OSError("publish failed")
        return original_rename(path, target)

    monkeypatch.setattr(Path, "rename", fail_publish)
    with pytest.raises(OSError, match="publish failed"):
        write(Session("claude", "broken", str(tmp_path)), sessions_dir=sessions)

    assert not list(sessions.rglob("rollout-*.jsonl"))
    assert not list(sessions.rglob("*.tmp"))
