import pytest

from engine.application import runtime_sessions
from engine.infrastructure.state_db import StateDatabase


@pytest.fixture
def store(tmp_path, monkeypatch):
    database = StateDatabase(tmp_path / "ferry-state.sqlite3", recover_interrupted=False)
    monkeypatch.setattr(runtime_sessions, "_database", lambda: database)
    return database


def _update(*, message="hello", event_type="run.started"):
    return {
        "metadata": {"session_id": "runtime-1", "provider_id": "test", "next_seq": 2},
        "messages": [{"ordinal": 0, "message": {"role": "user", "content": message}}],
        "events": [{"seq": 1, "event": {"type": event_type, "seq": 1}}],
        "timestamp": "2026-07-24T00:00:00.000Z",
    }


def test_runtime_records_are_opaque_and_replay_in_order(store):
    runtime_sessions.commit(_update())
    runtime_sessions.commit({
        "metadata": {"session_id": "runtime-1", "provider_id": "test", "next_seq": 3},
        "messages": [],
        "events": [{"seq": 2, "event": {"type": "run.completed", "seq": 2}}],
        "timestamp": "2026-07-24T00:00:01.000Z",
    })

    assert runtime_sessions.load_all() == [{
        "state": {"session_id": "runtime-1", "provider_id": "test", "next_seq": 3,
                  "messages": [{"role": "user", "content": "hello"}]},
        "events": [{"type": "run.started", "seq": 1},
                   {"type": "run.completed", "seq": 2}],
    }]


def test_runtime_commit_rejects_conflicting_replay_record(store):
    runtime_sessions.commit(_update())
    with pytest.raises(RuntimeError, match="记录冲突"):
        runtime_sessions.commit(_update(message="different"))


def test_runtime_delete_cascades_messages_and_events(store):
    runtime_sessions.commit(_update())
    assert runtime_sessions.delete("runtime-1") == {
        "session_id": "runtime-1", "deleted": True,
    }
    assert runtime_sessions.load_all() == []
