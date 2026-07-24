"""迁移历史的 SQLite 读取与删除。"""

import pytest

from engine.operations import history
from engine.storage.database import StateDatabase


@pytest.fixture
def store(tmp_path, monkeypatch):
    database = StateDatabase(
        tmp_path / "ferry-state.sqlite3", recover_interrupted=False,
    )
    monkeypatch.setattr(history, "_database", lambda _ports: database)
    return database


def test_empty_database_lists_nothing(store, ports):
    assert history.list_entries(ports) == []


def test_entries_are_newest_first_with_stable_database_ids(store, ports):
    first = history.append({"title": "a", "time": 1}, ports)
    second = history.append({"title": "b", "time": 2}, ports)

    assert [row["title"] for row in history.list_entries(ports)] == ["b", "a"]
    assert [row["id"] for row in history.list_entries(ports)] == [second, first]


def test_delete_removes_only_the_named_entry(store, ports):
    history.append({"title": "a", "time": 1}, ports)
    target = history.append({"title": "b", "time": 2}, ports)

    assert history.delete(target, ports) == {"deleted": True, "id": target, "remaining": 1}
    assert [row["title"] for row in history.list_entries(ports)] == ["a"]


def test_duplicate_entries_keep_independent_ids(store, ports):
    first = history.append({"title": "dup", "time": 1}, ports)
    second = history.append({"title": "dup", "time": 1}, ports)

    assert first != second
    history.delete(second, ports)
    assert [row["id"] for row in history.list_entries(ports)] == [first]


def test_unknown_id_is_a_no_op(store, ports):
    history.append({"title": "a", "time": 1}, ports)
    assert history.delete("history_missing", ports) == {
        "deleted": False,
        "id": "history_missing",
        "remaining": 1,
    }
