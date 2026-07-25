import json
from pathlib import Path

from engine.adapters.pi import scanner
from engine.system.paths import pi_session_roots


class Cache:
    def __init__(self):
        self.values = {}

    def get(self, path, stat):
        return self.values.get((str(path), stat.st_mtime_ns, stat.st_size))

    def put(self, path, stat, value):
        self.values[(str(path), stat.st_mtime_ns, stat.st_size)] = value


def _write(path, records):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(json.dumps(record) for record in records))


def test_pi_session_roots_priority_and_settings(tmp_path):
    assert pi_session_roots(
        environ={"PI_CODING_AGENT_SESSION_DIR": "/explicit"}, home=tmp_path,
    ) == (Path("/explicit"),)

    agent = tmp_path / "pi-agent"
    agent.mkdir()
    (agent / "settings.json").write_text(json.dumps({"sessionDir": "custom"}))
    assert pi_session_roots(
        environ={"PI_CODING_AGENT_DIR": str(agent)}, home=tmp_path,
    ) == (agent / "custom", agent / "sessions")


def test_scanner_accepts_only_v3_and_aggregates_usage(tmp_path, monkeypatch):
    root = tmp_path / "sessions"
    header = {"type": "session", "version": 3, "id": "valid",
              "timestamp": "2026-07-25T00:00:00Z", "cwd": "/raw/project"}
    _write(root / "bucket" / "valid.jsonl", [
        header,
        {"type": "message", "id": "u", "parentId": None,
         "timestamp": "2026-07-25T00:00:01Z",
         "message": {"role": "user", "content": "sk-test-title", "timestamp": 1}},
        {"type": "message", "id": "a", "parentId": "u",
         "timestamp": "2026-07-25T00:00:02Z",
         "message": {"role": "assistant", "content": [],
                     "model": "pi-model", "usage": {
                         "input": 10, "output": 4, "cacheRead": 3,
                         "cacheWrite": 2,
                     }, "timestamp": 2}},
    ])
    _write(root / "old.jsonl", [{**header, "version": 2, "id": "old"}])
    monkeypatch.setattr(scanner, "pi_session_roots", lambda: (root,))

    rows = scanner.scan(Cache())
    assert len(rows) == 1
    assert rows[0]["id"] == "valid"
    assert rows[0]["dir"] == "/raw/project"
    assert rows[0]["title"] == "sk-test-title"
    assert rows[0]["tokens"] == {
        "input": 10, "output": 4, "cache_read": 3, "cache_write": 2,
    }
    assert rows[0]["model"] == "pi-model"


def test_scanner_tolerates_malformed_final_line(tmp_path, monkeypatch):
    path = tmp_path / "session.jsonl"
    path.write_text(
        json.dumps({"type": "session", "version": 3, "id": "s",
                    "timestamp": "2026-07-25T00:00:00Z", "cwd": "/tmp"})
        + "\n"
        + json.dumps({"type": "message", "id": "u", "parentId": None,
                      "message": {"role": "user", "content": "kept"}})
        + "\n{broken"
    )
    monkeypatch.setattr(scanner, "pi_session_roots", lambda: (tmp_path,))
    assert scanner.scan(Cache())[0]["id"] == "s"
