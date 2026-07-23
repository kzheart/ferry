import json
import sqlite3

import pytest

from engine.adapters.opencode import session as opencode_session
from engine.adapters.base.editing import EditDocument
from engine.adapters.opencode.editor import OpenCodeBackend
from engine.domain.errors import (
    AgentFormatChangedError,
    SessionStoreUnavailableError,
)


def test_all_reads_refuse_cli_and_tempfile_fallback(monkeypatch):
    def unavailable():
        raise SessionStoreUnavailableError("opencode", "fixture")

    monkeypatch.setattr(opencode_session, "_db_conn", unavailable)
    monkeypatch.setattr(
        opencode_session,
        "_oc_export",
        lambda _ref: pytest.fail("preview must not invoke opencode export"),
    )

    with pytest.raises(SessionStoreUnavailableError):
        OpenCodeBackend().load_preview("session-1")
    with pytest.raises(SessionStoreUnavailableError):
        opencode_session.read("session-1")


def test_current_sqlite_schema_mismatch_fails_explicitly(tmp_path, monkeypatch):
    database = tmp_path / "opencode.db"
    with sqlite3.connect(database) as connection:
        connection.execute("CREATE TABLE session (id TEXT PRIMARY KEY)")
        connection.execute(
            "CREATE TABLE message (id TEXT, session_id TEXT, data TEXT)"
        )
        connection.execute(
            "CREATE TABLE part (id TEXT, message_id TEXT, session_id TEXT, data TEXT)"
        )
    monkeypatch.setattr(opencode_session, "OPENCODE_DB", database)
    monkeypatch.setattr(
        opencode_session,
        "_oc_export",
        lambda _ref: pytest.fail("schema mismatch must not invoke opencode export"),
    )

    with pytest.raises(
        AgentFormatChangedError,
    ) as excinfo:
        opencode_session.read("session-1")
    assert excinfo.value.code == "agent.format_changed"
    assert excinfo.value.params["location"] == "sqlite.session"


def test_snapshot_restore_reads_unicode_line_separators_as_json(tmp_path, monkeypatch):
    payload = {
        "info": {"id": "session-1", "directory": "/tmp"},
        "messages": [{
            "info": {"id": "message-1", "sessionID": "session-1", "role": "user"},
            "parts": [{
                "id": "part-1", "messageID": "message-1", "sessionID": "session-1",
                "type": "text", "text": "before\u0085after",
            }],
        }],
    }
    snapshot = tmp_path / "snapshot.jsonl"
    snapshot.write_text(json.dumps(payload, ensure_ascii=False) + "\n")
    monkeypatch.setattr(opencode_session, "_oc_export", lambda _ref: payload)

    class Client:
        def __enter__(self):
            return self

        def __exit__(self, *_args):
            return None

        def patch_part(self, *_args):
            pytest.fail("unchanged snapshot must not write")

    doc = EditDocument("opencode", "session-1", "session-1", payload, "revision")
    OpenCodeBackend(api_factory=lambda _cwd: Client()).restore_snapshot(snapshot, doc)
