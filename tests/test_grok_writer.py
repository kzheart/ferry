import json
import sqlite3
from pathlib import Path

import pytest

from engine.adapters.grok import probe as grok_probe
from engine.adapters.grok import adapter as grok_adapter
from engine.adapters.grok.migration import GrokMigrationTarget
from engine.adapters.grok.reader import read
from engine.adapters.grok.writer import _blake3, write
from engine.sessions.model import (
    Block, Message, Session, ToolCall, text_tool_result,
)
from engine.sessions.tool_ops import CanonicalOp
from engine.system import executables


def _passing_probe(_path):
    return {"status": "passed"}


def _session(tmp_path, title="sentinel-grok-writer"):
    tool = ToolCall(
        "read", CanonicalOp.TOOL_INVOKE,
        {"namespace": "grok", "name": "read",
         "input": {"path": "/raw/input.txt"}},
        text_tool_result("raw output"), source_call_id="call-fixed",
    )
    return Session(
        "fixture", "source", str(tmp_path), title=title,
        messages=[
            Message("user", [Block("text", "read input")]),
            Message("assistant", [
                Block("text", "before"),
                Block("tool", tool=tool),
                Block("text", "after"),
            ]),
        ],
    )


def test_blake3_matches_official_vectors():
    assert _blake3(b"") == (
        "af1349b9f5f9a1a6a0404dea36dcc949"
        "9bcb25c9adc112b7cc9a93cae41f3262"
    )
    assert _blake3(b"abc") == (
        "6437b3ac38465133ffb63b75273a8db5"
        "48c558465d79db03fd359c6cd5bd9d85"
    )


def test_writer_roundtrips_order_and_indexes_search_document(
        tmp_path, monkeypatch):
    monkeypatch.setattr(grok_probe, "probe_bundle", _passing_probe)
    sessions = tmp_path / "sessions"
    source = _session(tmp_path)

    sid, path = write(
        source, str(tmp_path), sessions,
        tool_decider=GrokMigrationTarget().evaluate_tool,
    )
    migrated = read(str(path))

    assert migrated.source_id == sid
    assert [block.kind for block in migrated.messages[1].blocks] == [
        "text", "tool", "text",
    ]
    tool = migrated.messages[1].blocks[1].tool
    assert tool.name == "read"
    assert tool.input == {
        "namespace": "grok", "name": "read",
        "input": {"path": "/raw/input.txt"},
    }
    assert tool.result.blocks[0].text == "raw output"

    database = sqlite3.connect(sessions / "session_search.sqlite")
    row = database.execute(
        """SELECT title, content, content_hash, last_indexed_offset
           FROM session_docs WHERE session_id=?""",
        (sid,),
    ).fetchone()
    search = database.execute(
        """SELECT d.session_id FROM session_docs_fts
           JOIN session_docs d ON d.rowid=session_docs_fts.rowid
           WHERE session_docs_fts MATCH ?""",
        ("sentinel",),
    ).fetchall()
    database.close()
    assert row[0] == "sentinel-grok-writer"
    assert row[2] == _blake3(
        row[0].encode() + b"\0" + row[1].encode()
    )
    assert row[3] == (path / "updates.jsonl").stat().st_size
    assert search == [(sid,)]
    database = sqlite3.connect(sessions / "session_search.sqlite")
    marker = database.execute(
        "SELECT value FROM meta WHERE key='last_bootstrap_at'"
    ).fetchone()
    database.close()
    assert marker is None

    chat = [
        json.loads(line) for line in
        (path / "chat_history.jsonl").read_text().splitlines()
    ]
    updates = [
        json.loads(line)["params"]["update"] for line in
        (path / "updates.jsonl").read_text().splitlines()
    ]
    native_call = next(item for item in chat if item["type"] == "assistant")
    assert native_call["tool_calls"][0]["arguments"] == (
        '{"path":"/raw/input.txt"}'
    )
    call_update = next(
        item for item in updates if item["sessionUpdate"] == "tool_call"
    )
    result_update = next(
        item for item in updates
        if item["sessionUpdate"] == "tool_call_update"
    )
    assert call_update["kind"] == "read"
    assert call_update["status"] == "pending"
    assert result_update["status"] == "completed"


def test_writer_indexes_unicode_line_separators_as_content(
        tmp_path, monkeypatch):
    monkeypatch.setattr(grok_probe, "probe_bundle", _passing_probe)
    sessions = tmp_path / "sessions"
    source = _session(tmp_path)
    content = "alpha\u0085beta\u2028gamma\u2029omega"
    source.messages[0].blocks[0].text = content

    sid, path = write(source, str(tmp_path), sessions)
    migrated = read(str(path))
    database = sqlite3.connect(sessions / "session_search.sqlite")
    indexed = database.execute(
        "SELECT content FROM session_docs WHERE session_id=?",
        (sid,),
    ).fetchone()[0]
    database.close()

    assert migrated.messages[0].blocks[0].text == content
    assert content in indexed


def test_writer_preserves_tree_links_and_existing_bundle(
        tmp_path, monkeypatch):
    monkeypatch.setattr(grok_probe, "probe_bundle", _passing_probe)
    sessions = tmp_path / "sessions"
    existing = sessions / "existing-project" / "existing"
    existing.mkdir(parents=True)
    sentinel = existing / "summary.json"
    sentinel.write_bytes(b'{"existing":true}\n')
    before = sentinel.read_bytes()
    source = _session(tmp_path, "root-sentinel")
    source.children = [
        Session(
            "fixture", "child", str(tmp_path), title="child-sentinel",
            messages=[Message("user", [Block("text", "child")])],
        )
    ]

    root_id, root_path = write(source, str(tmp_path), sessions)
    summaries = [
        json.loads(path.read_text())
        for path in sessions.rglob("summary.json") if path != sentinel
    ]
    child = next(item for item in summaries
                 if item["info"]["id"] != root_id)
    root_updates = (root_path / "updates.jsonl").read_text()

    assert child["parent_session_id"] == root_id
    assert child["root_session_id"] == root_id
    assert child["info"]["id"] in root_updates
    assert "subagent_spawned" in root_updates
    assert "subagent_finished" in root_updates
    assert sentinel.read_bytes() == before

    monkeypatch.setattr(grok_adapter, "grok_home", lambda: tmp_path)
    restored = grok_adapter.GrokBrowser().read(str(root_path))
    assert restored.root_id == root_id
    assert [child.title for child in restored.children] == [
        "child-sentinel",
    ]
    assert restored.children[0].root_id == root_id


def test_writer_preserves_failed_tool_status(tmp_path, monkeypatch):
    monkeypatch.setattr(grok_probe, "probe_bundle", _passing_probe)
    source = _session(tmp_path)
    source.messages[1].blocks[1].tool.result.status = "error"

    _, path = write(
        source, str(tmp_path), tmp_path / "sessions",
        tool_decider=GrokMigrationTarget().evaluate_tool,
    )
    migrated = read(str(path))

    assert migrated.messages[1].blocks[1].tool.result.status == "error"


def test_writer_uses_requested_target_cwd_not_stale_source(
        tmp_path, monkeypatch):
    monkeypatch.setattr(grok_probe, "probe_bundle", _passing_probe)
    source = _session(tmp_path)
    source.cwd = "/deleted/source/project"

    _, path = write(source, str(tmp_path), tmp_path / "sessions")

    summary = json.loads((path / "summary.json").read_text())
    assert summary["info"]["cwd"] == str(tmp_path.resolve())


def test_equal_child_values_keep_identity_based_parents(
        tmp_path, monkeypatch):
    monkeypatch.setattr(grok_probe, "probe_bundle", _passing_probe)
    duplicate_a = Session(
        "fixture", "same", "/ignored", title="duplicate",
        messages=[Message("user", [Block("text", "same")])],
    )
    duplicate_b = Session(
        "fixture", "same", "/ignored", title="duplicate",
        messages=[Message("user", [Block("text", "same")])],
    )
    first = Session("fixture", "first", "/ignored", title="first")
    second = Session("fixture", "second", "/ignored", title="second")
    first.children = [duplicate_a]
    second.children = [duplicate_b]
    root = Session("fixture", "root", "/ignored", title="root")
    root.children = [first, second]

    write(root, str(tmp_path), tmp_path / "sessions")
    summaries = [
        json.loads(path.read_text())
        for path in (tmp_path / "sessions").rglob("summary.json")
    ]
    parent_ids = {
        item["generated_title"]: item["info"]["id"]
        for item in summaries if item["generated_title"] in {"first", "second"}
    }
    duplicate_parents = {
        item["parent_session_id"] for item in summaries
        if item["generated_title"] == "duplicate"
    }

    assert duplicate_parents == set(parent_ids.values())


def test_writer_removes_every_generated_artifact_when_probe_fails(
        tmp_path, monkeypatch):
    monkeypatch.setattr(
        grok_probe, "probe_bundle",
        lambda _path: {"status": "failed", "diagnostic": {}},
    )
    sessions = tmp_path / "sessions"

    with pytest.raises(RuntimeError, match="无法验收"):
        write(_session(tmp_path), str(tmp_path), sessions)

    assert list(sessions.rglob("summary.json")) == []
    assert not (sessions / "session_search.sqlite").exists()


def test_writer_removes_published_bundles_when_index_transaction_fails(
        tmp_path, monkeypatch):
    monkeypatch.setattr(grok_probe, "probe_bundle", _passing_probe)
    from engine.adapters.grok import writer

    monkeypatch.setattr(
        writer, "index_bundles",
        lambda *_args, **_kwargs: (_ for _ in ()).throw(
            RuntimeError("index failed")
        ),
    )
    sessions = tmp_path / "sessions"

    with pytest.raises(RuntimeError, match="index failed"):
        write(_session(tmp_path), str(tmp_path), sessions)

    assert list(sessions.rglob("summary.json")) == []


def test_existing_index_is_backed_up_before_next_transaction(
        tmp_path, monkeypatch):
    monkeypatch.setattr(grok_probe, "probe_bundle", _passing_probe)
    sessions = tmp_path / "sessions"
    first_id, _ = write(
        _session(tmp_path, "first-sentinel"), str(tmp_path), sessions,
    )
    second_id, _ = write(
        _session(tmp_path, "second-sentinel"), str(tmp_path), sessions,
    )
    backup = sessions / "session_search.sqlite.ferry-backup"

    assert backup.is_file()
    database = sqlite3.connect(backup)
    ids = {
        row[0] for row in database.execute(
            "SELECT session_id FROM session_docs"
        )
    }
    database.close()
    assert ids == {first_id}
    assert second_id not in ids


def test_schema_version_mismatch_aborts_without_publishing_bundle(
        tmp_path, monkeypatch):
    monkeypatch.setattr(grok_probe, "probe_bundle", _passing_probe)
    sessions = tmp_path / "sessions"
    sessions.mkdir()
    database_path = sessions / "session_search.sqlite"
    database = sqlite3.connect(database_path)
    database.execute(
        "CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL)"
    )
    database.execute(
        "INSERT INTO meta(key, value) VALUES(?, ?)",
        ("session_search_schema_version", "3"),
    )
    database.commit()
    database.close()

    with pytest.raises(RuntimeError, match="结构或版本不受支持"):
        write(_session(tmp_path), str(tmp_path), sessions)

    database = sqlite3.connect(database_path)
    version = database.execute(
        "SELECT value FROM meta WHERE key='session_search_schema_version'"
    ).fetchone()
    database.close()
    assert version == ("3",)
    assert list(sessions.rglob("summary.json")) == []
    assert not (
        sessions / "session_search.sqlite.ferry-backup"
    ).exists()


@pytest.mark.skipif(
    not Path(executables.argv("grok", "--version")[0]).exists(),
    reason="未安装 Grok Build CLI",
)
def test_current_cli_accepts_generated_tool_bundle(tmp_path):
    sid, path = write(
        _session(tmp_path), str(tmp_path), tmp_path / "sessions",
        tool_decider=GrokMigrationTarget().evaluate_tool,
    )

    assert path.name == sid
