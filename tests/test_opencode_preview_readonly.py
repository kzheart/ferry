import pytest

from engine.adapters.opencode import session as opencode_session
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
