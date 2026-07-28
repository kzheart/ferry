import json

from engine.adapters.claude.editing import load as load_claude_records
from engine.adapters.codex.editor import CodexBackend
from engine.adapters.codex.native import CodexStore, _read_jsonl
from engine.adapters.pi.editor import PiBackend
from engine.adapters.pi.reader import read as read_pi


CONTENT = "alpha\u0085beta\u2028gamma\u2029omega"


def _write_jsonl(path, records):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        "\n".join(json.dumps(record, ensure_ascii=False)
                  for record in records) + "\n"
    )


def test_claude_loader_preserves_unicode_line_separators(tmp_path):
    path = tmp_path / "session.jsonl"
    _write_jsonl(path, [{"type": "user", "message": {"content": CONTENT}}])

    records = load_claude_records(path)

    assert records[0]["message"]["content"] == CONTENT


def test_codex_native_and_editor_preserve_unicode_line_separators(tmp_path):
    home = tmp_path / "codex-home"
    sessions = home / "sessions"
    path = sessions / "2026/07/28/rollout-session.jsonl"
    _write_jsonl(path, [
        {
            "type": "session_meta",
            "payload": {
                "id": "session", "session_id": "session", "cwd": "/work",
            },
        },
        {"type": "event_msg", "payload": {"message": CONTENT}},
    ])
    store = CodexStore(home, sessions, None)

    native, _digest = _read_jsonl(path)
    document = CodexBackend(
        store_factory=lambda _path: store,
    ).load_preview(str(path))

    assert native[1]["payload"]["message"] == CONTENT
    assert document.data[1]["payload"]["message"] == CONTENT


def test_pi_reader_and_editor_preserve_unicode_line_separators(tmp_path):
    path = tmp_path / "session.jsonl"
    _write_jsonl(path, [
        {
            "type": "session", "version": 3, "id": "session",
            "timestamp": "2026-07-28T00:00:00Z", "cwd": str(tmp_path),
        },
        {
            "type": "message", "id": "user", "parentId": None,
            "timestamp": "2026-07-28T00:00:01Z",
            "message": {
                "role": "user", "content": CONTENT, "timestamp": 1,
            },
        },
    ])

    session = read_pi(str(path))
    document = PiBackend().load(str(path))

    assert session.messages[0].blocks[0].text == CONTENT
    assert document.data[1]["message"]["content"] == CONTENT
