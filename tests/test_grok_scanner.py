import json

from engine.adapters.contracts import _resolved_root
from engine.adapters.grok.adapter import build
from engine.adapters.grok import scanner
from engine.context import EngineContext
from engine.sessions.index import AgentSessionIndex


class Cache:
    def get(self, *_): return None
    def put(self, *_): pass
    def flush(self): pass


def test_scanner_reads_summary_without_history(tmp_path, monkeypatch):
    bundle = tmp_path / "sessions" / "project" / "sid"
    bundle.mkdir(parents=True)
    (bundle / "summary.json").write_text(json.dumps({
        "info": {"id": "sid", "cwd": "/raw/project"},
        "chat_format_version": 1, "generated_title": "Raw title",
        "num_chat_messages": 2, "current_model_id": "grok-model",
        "created_at": "2026-07-25T00:00:00Z",
        "updated_at": "2026-07-25T00:00:01Z",
    }))
    (bundle / "updates.jsonl").write_text("{not read by scanner")
    monkeypatch.setattr(scanner, "grok_home", lambda: tmp_path)
    rows = scanner.scan(Cache())
    assert rows[0]["id"] == "sid"
    assert rows[0]["dir"] == "/raw/project"
    assert rows[0]["authoritative_members"] == [
        "summary.json", "updates.jsonl",
    ]


def test_scanned_bundle_enters_the_opaque_reference_index(tmp_path, monkeypatch):
    home = tmp_path / ".grok"
    bundle = home / "sessions" / "project" / "sid"
    bundle.mkdir(parents=True)
    (bundle / "summary.json").write_text(json.dumps({
        "info": {"id": "sid", "cwd": "/raw/project"},
        "chat_format_version": 1,
        "num_chat_messages": 1,
        "updated_at": "2026-07-25T00:00:01Z",
    }))
    (bundle / "updates.jsonl").write_text(
        '{"method":"session/update","params":{"update":{}}}\n'
    )
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("GROK_HOME", str(home))
    _resolved_root.cache_clear()
    adapter = build()
    ports = EngineContext(
        adapter=lambda _tool: adapter,
        adapters=lambda: ("grok",),
        cache_factory=Cache,
        resource_path=lambda *_: tmp_path,
        snapshot_dir=lambda: tmp_path,
        version="test",
    )
    try:
        records = AgentSessionIndex(ports).refresh()
    finally:
        _resolved_root.cache_clear()

    assert len(records) == 1
    assert records[0].tool == "grok"
    assert records[0].storage_kind == "directory"
    assert records[0].canonical_ref == str(bundle)
