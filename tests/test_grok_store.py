import json
from pathlib import Path

import pytest

from engine.adapters.grok.store import fingerprint, load_grok_bundle
from engine.errors import AgentFormatChangedError


FIXTURES = Path(__file__).parent / "fixtures" / "agent_formats" / "grok"


def test_loads_only_authoritative_bundle_members():
    path = FIXTURES / "case-02-tools"
    bundle = load_grok_bundle(path)
    assert bundle.summary["chat_format_version"] == 1
    assert [member.name for member in bundle.authoritative_members] == [
        "summary.json", "updates.jsonl",
    ]
    before = fingerprint(path)
    extra = path / "events.jsonl"
    extra.write_text('{"ignored":true}\n')
    try:
        assert fingerprint(path) == before
    finally:
        extra.unlink()


def test_rejects_missing_or_old_summary(tmp_path):
    with pytest.raises(AgentFormatChangedError):
        load_grok_bundle(tmp_path)
    (tmp_path / "summary.json").write_text(json.dumps({
        "info": {"id": "x", "cwd": "/tmp"}, "chat_format_version": 0,
    }))
    (tmp_path / "chat_history.jsonl").write_text('{"type":"user"}\n')
    with pytest.raises(AgentFormatChangedError):
        load_grok_bundle(tmp_path)


def test_bad_tail_keeps_valid_history(tmp_path):
    (tmp_path / "summary.json").write_text(json.dumps({
        "info": {"id": "x", "cwd": "/tmp"}, "chat_format_version": 1,
    }))
    (tmp_path / "updates.jsonl").write_text(
        '{"method":"session/update"}\n{broken'
    )
    bundle = load_grok_bundle(tmp_path)
    assert len(bundle.updates) == 1
    assert bundle.diagnostics == []
