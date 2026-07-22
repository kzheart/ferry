import json

import pytest

from engine.adapters.opencode import session as opencode_session
from engine.adapters.base.editing import EditDocument
from engine.adapters.opencode.editor import OpenCodeBackend


def test_preview_refuses_cli_and_tempfile_fallback(monkeypatch):
    monkeypatch.setattr(opencode_session, "_db_conn", lambda: None)
    monkeypatch.setattr(
        opencode_session,
        "_oc_export",
        lambda _ref: pytest.fail("preview must not invoke opencode export"),
    )

    with pytest.raises(RuntimeError, match="拒绝执行 Agent 预览"):
        OpenCodeBackend().load_preview("session-1")


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
