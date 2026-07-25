import json

from engine.adapters.grok import scanner


class Cache:
    def get(self, *_): return None
    def put(self, *_): pass


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
