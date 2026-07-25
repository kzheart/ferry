import json

from engine.adapters.grok import scanner


def _bundle(root, sid, parent=None):
    path = root / sid
    path.mkdir(parents=True)
    value = {
        "info": {"id": sid, "cwd": "/tmp"}, "chat_format_version": 1,
        "num_chat_messages": 1,
    }
    if parent:
        value["parent_session_id"] = parent
    (path / "summary.json").write_text(json.dumps(value))
    (path / "chat_history.jsonl").write_text(
        '{"type":"user","content":[{"type":"text","text":"x"}]}\n'
    )


def test_scanner_builds_parent_child_topology(tmp_path, monkeypatch):
    root = tmp_path / "sessions"
    _bundle(root, "root")
    _bundle(root, "child", "root")
    monkeypatch.setattr(scanner, "grok_home", lambda: tmp_path)
    rows = scanner.scan(None)
    assert len(rows) == 1
    assert rows[0]["id"] == "root"
    assert rows[0]["children"][0]["id"] == "child"
