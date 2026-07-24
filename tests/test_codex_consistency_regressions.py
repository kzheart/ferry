import json
import sqlite3

from engine.adapters.codex import reader as codex_reader
from engine.adapters.codex.writer import write
from engine.sessions.model import (
    AgentEdge, Block, Message, Session, ToolCall, text_tool_result,
)
from engine.sessions.tool_ops import CanonicalOp


SCHEMA = """
CREATE TABLE threads (
    id TEXT PRIMARY KEY, rollout_path TEXT NOT NULL,
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
    created_at_ms INTEGER, updated_at_ms INTEGER,
    recency_at INTEGER, recency_at_ms INTEGER,
    source TEXT NOT NULL, model_provider TEXT NOT NULL, cwd TEXT NOT NULL,
    title TEXT NOT NULL, sandbox_policy TEXT NOT NULL,
    approval_mode TEXT NOT NULL, tokens_used INTEGER NOT NULL DEFAULT 0,
    has_user_event INTEGER NOT NULL DEFAULT 0, archived INTEGER NOT NULL DEFAULT 0,
    cli_version TEXT NOT NULL DEFAULT '', first_user_message TEXT NOT NULL DEFAULT '',
    agent_path TEXT, thread_source TEXT, preview TEXT NOT NULL DEFAULT '',
    history_mode TEXT NOT NULL DEFAULT 'legacy'
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
    db = home / "state_5.sqlite"
    with sqlite3.connect(db) as conn:
        conn.executescript(SCHEMA)
    return sessions, db


def _tree(tmp_path):
    root = Session("claude", "root", str(tmp_path), title="Migrated")
    root.messages = [
        Message("user", [Block("text", "first")], source_id="user-1",
                created_at="2026-07-22T04:17:06.066Z"),
        Message("assistant", [Block("text", "delegating")], source_id="assistant-1",
                created_at="2026-07-22T04:17:28.433Z"),
        Message("user", [Block("text", "after delegate")], source_id="user-2",
                created_at="2026-07-22T10:22:34.060Z"),
    ]
    for source_id, text in (("child-b", "B"), ("child-a", "A")):
        child = Session("claude", source_id, str(tmp_path))
        child.messages = [Message("assistant", [Block("text", text)],
                                  source_id=f"{source_id}-message",
                                  created_at="2026-07-22T04:18:00.000Z")]
        root.children.append(child)
        root.agent_edges.append(AgentEdge(
            parent_session_id=root.source_id,
            child_session_id=child.source_id,
            spawn_message_id="assistant-1",
            status="open",
            prompt=f"review {text}",
        ))
    return root


def test_codex_writer_preserves_message_time_and_spawn_position(tmp_path, monkeypatch):
    sessions, db = _store(tmp_path)
    monkeypatch.setattr(codex_reader, "_META_CACHE_PATH", tmp_path / "rollout-cache.json")

    root_id, path = write(_tree(tmp_path), sessions_dir=sessions)
    records = [json.loads(line) for line in path.read_text().splitlines()]
    messages = [record for record in records
                if record["type"] == "response_item" and
                record["payload"].get("type") == "message"]
    assert [record["timestamp"] for record in messages] == [
        "2026-07-22T04:17:06.066Z",
        "2026-07-22T04:17:28.433Z",
        "2026-07-22T10:22:34.060Z",
    ]
    assistant = messages[1]
    assert assistant["payload"]["id"].startswith("msg_")
    assert "id" not in messages[0]["payload"]
    assistant_index = records.index(assistant)
    assert records[assistant_index + 1]["payload"]["type"] == "function_call"
    assert records[assistant_index + 1]["payload"]["name"] == "spawn_agent"
    assert records[assistant_index + 1]["payload"]["status"] == "in_progress"

    with sqlite3.connect(db) as conn:
        rows = conn.execute(
            "SELECT child_thread_id, status FROM thread_spawn_edges ORDER BY child_thread_id"
        ).fetchall()
        paths = conn.execute(
            "SELECT agent_path FROM threads WHERE id != ? ORDER BY agent_path", (root_id,)
        ).fetchall()
    assert [status for _child, status in rows] == ["open", "open"]
    assert paths == [("/root/1",), ("/root/2",)]


def test_codex_reader_round_trips_edge_context_and_deterministic_siblings(tmp_path, monkeypatch):
    sessions, _db = _store(tmp_path)
    monkeypatch.setattr(codex_reader, "_META_CACHE_PATH", tmp_path / "rollout-cache.json")

    _root_id, path = write(_tree(tmp_path), sessions_dir=sessions)
    restored = codex_reader.read(str(path), sessions_dir=sessions)

    assert [message.created_at for message in restored.messages[:2]] == [
        "2026-07-22T04:17:06.066Z",
        "2026-07-22T04:17:28.433Z",
    ]
    assert [child.agent_path for child in restored.children] == ["/root/1", "/root/2"]
    assert [edge.status for edge in restored.agent_edges] == ["open", "open"]
    assert [edge.spawn_message_id for edge in restored.agent_edges] == [
        restored.messages[1].source_id,
        restored.messages[1].source_id,
    ]


def test_codex_writer_uses_parent_message_time_for_tools_without_own_time(tmp_path):
    sessions, _db = _store(tmp_path)
    root = Session("claude", "root", str(tmp_path))
    root.messages = [Message(
        "assistant",
        [Block("tool", tool=ToolCall(
            "Bash", CanonicalOp.SHELL_EXEC, {"command": "pwd"},
            text_tool_result("/tmp")))],
        source_id="tool-message",
        created_at="2026-07-22T04:17:28.433Z",
    )]

    _root_id, path = write(root, sessions_dir=sessions)
    records = [json.loads(line) for line in path.read_text().splitlines()]
    tool_records = [record for record in records
                    if record["type"] == "response_item"]

    assert len(tool_records) == 2
    assert {record["timestamp"] for record in tool_records} == {
        "2026-07-22T04:17:28.433Z",
    }


def test_codex_reader_preserves_spawn_order_over_agent_path_sort(tmp_path, monkeypatch):
    sessions, _db = _store(tmp_path)
    monkeypatch.setattr(codex_reader, "_META_CACHE_PATH", tmp_path / "rollout-cache.json")
    root = _tree(tmp_path)
    root.children[0].agent_path = "/root/z-last-by-name"
    root.children[1].agent_path = "/root/a-first-by-name"
    root.agent_edges[0].agent_path = root.children[0].agent_path
    root.agent_edges[1].agent_path = root.children[1].agent_path

    _root_id, path = write(root, sessions_dir=sessions)
    restored = codex_reader.read(str(path), sessions_dir=sessions)

    assert [child.agent_path for child in restored.children] == [
        "/root/z-last-by-name",
        "/root/a-first-by-name",
    ]
