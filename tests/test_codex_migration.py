import json
import sqlite3

import pytest

from engine.adapters import registry
from engine.adapters.claude.reader import read as read_claude
from engine.adapters.claude.writer import write as write_claude
from engine.adapters.codex.lifecycle import CodexLifecycle
from engine.adapters.codex.writer import write
from engine.adapters.opencode import session as opencode_session
from engine.domain.model import (
    Block, Message, Session, ToolCall, text_tool_result,
)
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
    thread_source TEXT, preview TEXT NOT NULL DEFAULT '',
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
    db = home / "state_5.sqlite"
    with sqlite3.connect(db) as conn:
        conn.executescript(SCHEMA)
    return sessions, db


def _tree(tmp_path):
    root = Session("claude", "source-root", str(tmp_path), title="Migrated")
    root.messages = [Message("user", [Block("text", "continue this work")])]
    child = Session("claude", "source-child", str(tmp_path), parent_id=root.source_id,
                    agent_path="/root/reviewer")
    child.messages = [Message("assistant", [Block("text", "review complete")])]
    root.children = [child]
    return root


MIGRATION_TARGET_TESTS = {"claude", "codex", "opencode"}


def test_every_migration_target_has_a_discovery_test():
    adapters = registry.create_registry()
    targets = {tool for tool in adapters.ids()
               if adapters.get(tool).migration_target is not None}
    assert targets == MIGRATION_TARGET_TESTS


def test_claude_writer_publishes_discoverable_session(tmp_path, monkeypatch):
    monkeypatch.setenv("HOME", str(tmp_path))

    session_id, path = write_claude(_tree(tmp_path))

    assert path == next((tmp_path / ".claude" / "projects").glob(
        f"*/{session_id}.jsonl"))
    restored = read_claude(str(path))
    assert restored.source_id == session_id
    assert restored.messages[0].blocks[0].text == "continue this work"
    records = [json.loads(line) for line in path.read_text().splitlines() if line.strip()]
    assert all(record.get("timestamp") for record in records
               if record.get("type") in {"user", "assistant"})
    assert all(record.get("userType") == "external" for record in records
               if record.get("type") in {"user", "assistant"})


def test_claude_semantic_rewrite_uses_current_shape_without_unknown_fields(
        tmp_path, monkeypatch):
    monkeypatch.setenv("HOME", str(tmp_path))
    native = tmp_path / "source.jsonl"
    native.write_text(json.dumps({
        "type": "user",
        "uuid": "u1",
        "parentUuid": None,
        "sessionId": "old",
        "cwd": "/fixture/path",
        "message": {"role": "user", "content": "hello from native"},
        "version": "2.1.204",
        "unknownNativeField": "must-not-copy",
    }) + "\n")
    source = read_claude(str(native))

    session_id, path = write_claude(source, cwd=str(tmp_path / "proj"))
    record = json.loads(path.read_text().splitlines()[0])
    assert record["sessionId"] == session_id
    assert record["cwd"] == str(tmp_path / "proj")
    assert record["timestamp"]
    assert record["userType"] == "external"
    assert record["message"]["content"] == "hello from native"
    assert "unknownNativeField" not in record
    assert not hasattr(source, "raw_records")
    assert all(not hasattr(message, "raw") for message in source.messages)


def test_opencode_writer_imports_every_session_for_discovery(tmp_path, monkeypatch):
    imported = []
    database = tmp_path / "opencode.db"
    monkeypatch.setattr(opencode_session, "OPENCODE_DB", database)
    monkeypatch.setattr(
        opencode_session, "_import_payload",
        lambda payload, sid, cwd: imported.append((payload, sid, cwd)),
    )

    root_id, destination = opencode_session.write(_tree(tmp_path), cwd=str(tmp_path))

    assert destination == database
    assert len(imported) == 2
    assert imported[0][1] == root_id
    assert imported[0][0]["info"]["id"] == root_id
    assert imported[1][0]["info"]["parentID"] == root_id
    assert all(payload["info"]["directory"] == str(tmp_path)
               for payload, _sid, _cwd in imported)
    root = imported[0][0]
    assert {"slug", "projectID", "path", "agent", "summary", "cost",
            "tokens", "time"} <= root["info"].keys()
    user, assistant = (message["info"] for message in root["messages"])
    assert {"agent", "model", "summary", "time"} <= user.keys()
    assert {"mode", "agent", "path", "cost", "tokens", "modelID",
            "providerID", "time", "finish"} <= assistant.keys()
    task = next(part for message in root["messages"]
                for part in message.get("parts", [])
                if part.get("tool") == "task")
    assert task["state"]["status"] == "completed"
    child_messages = imported[1][0]["messages"]
    assert child_messages[0]["info"]["role"] == "user"
    assert child_messages[1]["info"]["parentID"] == \
        child_messages[0]["info"]["id"]


def test_opencode_tool_parts_include_required_state_time(tmp_path, monkeypatch):
    imported = []
    monkeypatch.setattr(opencode_session, "OPENCODE_DB", tmp_path / "opencode.db")
    monkeypatch.setattr(
        opencode_session, "_import_payload",
        lambda payload, sid, cwd: imported.append(payload),
    )
    root = Session("claude", "tools-root", str(tmp_path), title="tools")
    root.messages = [
        Message("user", [Block("text", "run tools")]),
        Message("assistant", [
            Block("tool", tool=ToolCall(
                name="Bash",
                op=CanonicalOp.SHELL_EXEC,
                input={"command": "pwd"},
                result=text_tool_result("/tmp"),
            )),
        ]),
    ]

    opencode_session.write(root, cwd=str(tmp_path))

    tools = [part for message in imported[0]["messages"]
             for part in message.get("parts", []) if part.get("type") == "tool"]
    assert tools
    assert all("time" in (part.get("state") or {}) for part in tools)
    assert all({"start", "end"} <= set((part.get("state") or {})["time"])
               for part in tools)


def test_opencode_writer_preserves_source_message_chronology(tmp_path, monkeypatch):
    imported = []
    monkeypatch.setattr(opencode_session, "OPENCODE_DB", tmp_path / "opencode.db")
    monkeypatch.setattr(
        opencode_session, "_import_payload",
        lambda payload, sid, cwd: imported.append(payload),
    )
    root = Session("claude", "ordered-root", str(tmp_path), title="ordered")
    root.messages = [
        Message("user", [Block("text", "first")],
                created_at="2026-07-22T04:17:06.066Z"),
        Message("assistant", [Block("text", "second")],
                created_at="2026-07-22T04:17:28.433Z"),
        Message("user", [Block("text", "third")],
                created_at="2026-07-22T10:22:34.060Z"),
    ]

    opencode_session.write(root, cwd=str(tmp_path))

    messages = imported[0]["messages"]
    assert [message["parts"][0]["text"] for message in messages] == [
        "first", "second", "third",
    ]
    created = [message["info"]["time"]["created"] for message in messages]
    assert created == sorted(created)
    assert len(created) == len(set(created))


def test_opencode_writer_rolls_back_partially_imported_session(tmp_path, monkeypatch):
    deleted = []
    monkeypatch.setattr(opencode_session, "OPENCODE_DB", tmp_path / "opencode.db")
    monkeypatch.setattr(
        opencode_session, "_import_payload",
        lambda *_args, **_kwargs: (_ for _ in ()).throw(RuntimeError("invalid schema")),
    )
    monkeypatch.setattr(
        opencode_session, "_oc",
        lambda args, **_kwargs: deleted.append(args) or "",
    )

    with pytest.raises(RuntimeError, match="invalid schema"):
        opencode_session.write(_tree(tmp_path), cwd=str(tmp_path))

    assert len(deleted) == 1
    assert deleted[0][:2] == ["session", "delete"]
    assert deleted[0][2].startswith("ses_")


def test_codex_writer_registers_rollout_tree(tmp_path):
    sessions, db = _store(tmp_path)

    root_id, root_path = write(_tree(tmp_path), sessions_dir=sessions)

    with sqlite3.connect(db) as conn:
        rows = conn.execute(
            "SELECT id, rollout_path, cwd, title FROM threads ORDER BY title DESC"
        ).fetchall()
        edge = conn.execute(
            "SELECT parent_thread_id, child_thread_id, status FROM thread_spawn_edges"
        ).fetchone()
    assert len(rows) == 2
    assert rows[0] == (root_id, str(root_path.resolve()), str(tmp_path), "Migrated")
    assert edge[0] == root_id
    assert edge[1] in {row[0] for row in rows}
    assert edge[2] == "closed"

    records = [json.loads(line) for line in root_path.read_text().splitlines()]
    meta = records[0]
    assert meta["type"] == "session_meta"
    required = {"timestamp", "originator", "source", "thread_source",
                "model_provider"}
    assert required <= meta["payload"].keys()
    assert meta["payload"]["originator"] == "codex-tui"
    assert meta["payload"]["source"] == "cli"
    assert meta["payload"]["thread_source"] == "user"
    assert meta["payload"]["model_provider"] == "openai"
    assert not any(record["type"] == "turn_context" for record in records)
    child_path = next(
        path for path in sessions.glob("*/*/*/rollout-*.jsonl")
        if path != root_path
    )
    child_meta = json.loads(child_path.read_text().splitlines()[0])["payload"]
    assert child_meta["thread_source"] == "subagent"
    assert isinstance(child_meta["source"]["subagent"], dict)


def test_codex_cleanup_removes_files_and_registration(tmp_path):
    sessions, db = _store(tmp_path)
    root_id, root_path = write(_tree(tmp_path), sessions_dir=sessions)

    CodexLifecycle().cleanup(root_id, root_path)

    assert not list(sessions.glob("*/*/*/rollout-*.jsonl"))
    with sqlite3.connect(db) as conn:
        assert conn.execute("SELECT COUNT(*) FROM threads").fetchone()[0] == 0
        assert conn.execute("SELECT COUNT(*) FROM thread_spawn_edges").fetchone()[0] == 0


def test_codex_writer_rolls_back_files_when_registration_fails(tmp_path):
    sessions, db = _store(tmp_path)
    with sqlite3.connect(db) as conn:
        conn.execute("ALTER TABLE threads ADD COLUMN future_required TEXT NOT NULL DEFAULT ''")
        conn.execute("PRAGMA writable_schema=ON")
        sql = conn.execute(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='threads'"
        ).fetchone()[0]
        conn.execute(
            "UPDATE sqlite_master SET sql=? WHERE type='table' AND name='threads'",
            (sql.replace("future_required TEXT NOT NULL DEFAULT ''",
                         "future_required TEXT NOT NULL"),),
        )
        conn.execute("PRAGMA writable_schema=OFF")

    try:
        write(_tree(tmp_path), sessions_dir=sessions)
    except RuntimeError as error:
        assert "future_required" in str(error)
    else:
        raise AssertionError("不兼容的 Codex 注册 schema 应阻止发布")
    assert not list(sessions.glob("*/*/*/rollout-*.jsonl"))
