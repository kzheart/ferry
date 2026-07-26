from concurrent.futures import ThreadPoolExecutor

import pytest

from engine.runtime import sessions as runtime_sessions
from engine.storage.database import StateDatabase, get_state_database


@pytest.fixture
def store(tmp_path, monkeypatch):
    database = StateDatabase(tmp_path / "ferry-state.sqlite3", recover_interrupted=False)
    monkeypatch.setattr(runtime_sessions, "_database", lambda _ports: database)
    return database


def _update(*, message="hello", event_type="run.started"):
    return {
        "metadata": {"session_id": "runtime-1", "provider_id": "test", "next_seq": 2},
        "messages": [{"ordinal": 0, "message": {"role": "user", "content": message}}],
        "events": [{"seq": 1, "type": event_type}],
        "timestamp": "2026-07-24T00:00:00.000Z",
    }


def test_runtime_records_are_opaque_and_replay_in_order(store, ports):
    runtime_sessions.commit(_update(), ports)
    runtime_sessions.commit({
        "metadata": {"session_id": "runtime-1", "provider_id": "test", "next_seq": 3},
        "messages": [],
        "events": [{"seq": 2, "type": "run.completed"}],
        "timestamp": "2026-07-24T00:00:01.000Z",
    }, ports)

    assert runtime_sessions.load_all(ports) == [{
        "state": {"session_id": "runtime-1", "provider_id": "test", "next_seq": 3,
                  "messages": [{"role": "user", "content": "hello"}]},
        "events": [{"seq": 1, "type": "run.started"},
                   {"seq": 2, "type": "run.completed"}],
    }]


def test_runtime_commit_rejects_conflicting_replay_record(store, ports):
    runtime_sessions.commit(_update(), ports)
    with pytest.raises(RuntimeError, match="记录冲突"):
        runtime_sessions.commit(_update(message="different"), ports)


def test_runtime_delete_cascades_messages_and_events(store, ports):
    runtime_sessions.commit(_update(), ports)
    assert runtime_sessions.delete("runtime-1", ports) == {
        "session_id": "runtime-1", "deleted": True,
    }
    assert runtime_sessions.load_all(ports) == []


def test_runtime_truncate_deletes_tail_and_allows_seq_reuse(store, ports):
    runtime_sessions.commit(_update(), ports)
    runtime_sessions.commit({
        "metadata": {"session_id": "runtime-1", "provider_id": "test", "next_seq": 3},
        "messages": [{"ordinal": 1, "message": {"role": "assistant", "content": "hi"}}],
        "events": [{"seq": 2, "type": "run.completed"}],
        "timestamp": "2026-07-24T00:00:01.000Z",
    }, ports)

    assert runtime_sessions.truncate("runtime-1", 1, 2, ports) == {
        "session_id": "runtime-1", "messages_deleted": 1, "events_deleted": 1,
    }
    # 截断后同键可以重新写入不同内容:编辑重发正是要重用这些 ordinal/seq
    runtime_sessions.commit({
        "metadata": {"session_id": "runtime-1", "provider_id": "test", "next_seq": 3},
        "messages": [{"ordinal": 1, "message": {"role": "assistant", "content": "changed"}}],
        "events": [{"seq": 2, "type": "run.failed"}],
        "timestamp": "2026-07-24T00:00:02.000Z",
    }, ports)
    assert runtime_sessions.load_all(ports)[0]["state"]["messages"] == [
        {"role": "user", "content": "hello"},
        {"role": "assistant", "content": "changed"},
    ]


def test_runtime_truncate_validates_params(store, ports):
    with pytest.raises(Exception, match="session_id"):
        runtime_sessions.truncate("", 0, 0, ports)
    with pytest.raises(Exception, match="from_seq"):
        runtime_sessions.truncate("runtime-1", 0, -1, ports)


def test_state_database_is_reused_per_path(tmp_path):
    path = tmp_path / "ferry-state.sqlite3"
    first = get_state_database(path, recover_interrupted=False)
    assert get_state_database(path, recover_interrupted=False) is first
    other = get_state_database(tmp_path / "other.sqlite3", recover_interrupted=False)
    assert other is not first


def test_concurrent_commits_on_the_shared_database(ports):
    """serial 池与读池会同时用上同一个实例:连接仍是每次操作现开的。"""
    def commit(index):
        runtime_sessions.commit({
            "metadata": {"session_id": f"runtime-{index}", "provider_id": "test",
                         "next_seq": 2},
            "messages": [{"ordinal": 0,
                          "message": {"role": "user", "content": f"m{index}"}}],
            "events": [{"seq": 1, "type": "run.started"}],
            "timestamp": "2026-07-24T00:00:00.000Z",
        }, ports)

    with ThreadPoolExecutor(max_workers=8) as pool:
        list(pool.map(commit, range(32)))

    assert len(runtime_sessions.load_all(ports)) == 32
