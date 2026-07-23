import json

from engine.adapters.codex.editor import CodexBackend
from engine.adapters.codex.native import CodexStore
from engine.application import editing


def _rollout(path, thread_id):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps({
        "type": "session_meta",
        "payload": {"id": thread_id, "session_id": thread_id, "cwd": "/work"},
    }) + "\n")


def test_codex_preview_load_does_not_recover_pending_transactions(tmp_path):
    home = tmp_path / "codex-home"
    sessions = home / "sessions"
    anchor = sessions / "2026/07/22/rollout-anchor.jsonl"
    victim = sessions / "2026/07/22/rollout-victim.jsonl"
    _rollout(anchor, "anchor")
    _rollout(victim, "victim")
    stage = home / ".resume-harness" / "staging" / "pending"
    stage.mkdir(parents=True)
    marker = stage / "marker"
    marker.write_text("must-survive")
    journal = home / ".resume-harness" / "transactions" / "pending.json"
    journal.parent.mkdir(parents=True)
    journal.write_text(json.dumps({
        "ids": ["victim"], "paths": [str(victim)], "stage_dir": str(stage),
    }))
    before = {path: path.read_bytes() for path in (anchor, victim, marker, journal)}
    store = CodexStore(home, sessions, None)

    backend = CodexBackend(store_factory=lambda _path: store)
    document = backend.load_preview(str(anchor))

    assert document.ref == str(anchor)
    assert {path: path.read_bytes() for path in before} == before

    editing.preview(backend, str(anchor), [], loader=backend.load_preview)
    assert {path: path.read_bytes() for path in before} == before
