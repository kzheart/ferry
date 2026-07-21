import sqlite3

from engine.adapters import registry
from engine.adapters.claude.reader import read as read_claude
from engine.adapters.claude.writer import write as write_claude
from engine.adapters.codex.lifecycle import CodexLifecycle
from engine.adapters.codex.writer import write
from engine.adapters.opencode import session as opencode_session
from engine.domain.model import Block, Message, Session


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
    targets = {tool for tool in registry.adapters()
               if registry.adapter(tool).migration_target is not None}
    assert targets == MIGRATION_TARGET_TESTS


def test_claude_writer_publishes_discoverable_session(tmp_path, monkeypatch):
    monkeypatch.setenv("HOME", str(tmp_path))

    session_id, path = write_claude(_tree(tmp_path))

    assert path == next((tmp_path / ".claude" / "projects").glob(
        f"*/{session_id}.jsonl"))
    restored = read_claude(str(path))
    assert restored.source_id == session_id
    assert restored.messages[0].blocks[0].text == "continue this work"


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
