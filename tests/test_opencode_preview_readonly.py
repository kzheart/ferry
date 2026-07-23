import json
import sqlite3
from types import SimpleNamespace

import pytest

from engine.adapters.opencode import session as opencode_session
from engine.adapters.base.editing import EditDocument
from engine.adapters.opencode.editor import OpenCodeBackend, OpenCodeDocument
from engine.domain.errors import (
    AgentFormatChangedError,
    SessionStoreUnavailableError,
)
from engine.domain.model import Session


def _payload():
    return {
        "info": {"id": "session-1", "directory": "/tmp"},
        "messages": [{
            "info": {
                "id": "message-1",
                "sessionID": "session-1",
                "role": "user",
                "model": {
                    "providerID": "fixture-provider",
                    "modelID": "fixture-model",
                },
            },
            "parts": [{
                "id": "part-1",
                "messageID": "message-1",
                "sessionID": "session-1",
                "type": "text",
                "text": "original",
            }],
        }],
    }


def test_canonical_reader_does_not_retain_native_document():
    session, _edges = opencode_session._parse_session(_payload())

    assert session.model_provider == "fixture-provider"
    assert session.model == "fixture-model"
    assert not hasattr(session, "raw_records")
    assert all(not hasattr(message, "raw") for message in session.messages)
    assert [message.source_id for message in session.messages] == ["message-1"]


def test_canonical_reader_keeps_missing_model_explicitly_empty():
    session, _edges = opencode_session._parse_session({
        "info": {"id": "session-1", "directory": "/tmp"},
        "messages": [],
    })

    assert session.model_provider is None
    assert session.model is None


@pytest.mark.parametrize(
    ("method", "tree_loader"),
    [("load", "read"), ("load_preview", "read_preview")],
)
def test_editor_loads_private_native_document_without_canonical_meta(
        monkeypatch, method, tree_loader):
    payload = _payload()
    tree = SimpleNamespace()
    monkeypatch.setattr(
        opencode_session, "load_native_payload", lambda _ref: payload,
    )
    monkeypatch.setattr(opencode_session, tree_loader, lambda _ref: tree)

    document = getattr(OpenCodeBackend(), method)("session-1")

    assert isinstance(document, OpenCodeDocument)
    assert document.tree is tree
    assert document.data == payload
    assert document.original == payload
    document.data["messages"][0]["parts"][0]["text"] = "edited"
    assert document.original["messages"][0]["parts"][0]["text"] == "original"
    assert payload["messages"][0]["parts"][0]["text"] == "original"


def test_save_copy_passes_native_payload_without_mutating_canonical_meta(
        monkeypatch, tmp_path):
    payload = _payload()
    tree = Session("opencode", "session-1", "/tmp")
    document = OpenCodeDocument(
        tool="opencode",
        ref="session-1",
        handle="session-1",
        data=payload,
        revision="sha256:fixture",
        original=opencode_session._clone(payload),
        tree=tree,
    )
    captured = {}

    def write(session, **kwargs):
        captured.update(session=session, **kwargs)
        return "copy-1", tmp_path / "opencode.db"

    monkeypatch.setattr(opencode_session, "write", write)

    result = OpenCodeBackend().save_copy(document)

    assert result["session_id"] == "copy-1"
    assert captured["native_payloads"] == {"session-1": payload}


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
