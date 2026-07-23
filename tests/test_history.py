"""迁移历史的 SQLite 读取与删除。"""

import pytest

from engine.application import history
from engine.infrastructure.state_db import StateDatabase


@pytest.fixture
def store(tmp_path, monkeypatch):
    database = StateDatabase(
        tmp_path / "ferry-state.sqlite3", recover_interrupted=False,
    )
    monkeypatch.setattr(history, "_database", lambda: database)
    return database


def test_empty_database_lists_nothing(store):
    assert history.list_entries() == []


def test_entries_are_newest_first_with_stable_database_ids(store):
    first = history.append({"title": "a", "time": 1})
    second = history.append({"title": "b", "time": 2})

    assert [row["title"] for row in history.list_entries()] == ["b", "a"]
    assert [row["id"] for row in history.list_entries()] == [second, first]


def test_delete_removes_only_the_named_entry(store):
    history.append({"title": "a", "time": 1})
    target = history.append({"title": "b", "time": 2})

    assert history.delete(target) == {"deleted": True, "id": target, "remaining": 1}
    assert [row["title"] for row in history.list_entries()] == ["a"]


def test_duplicate_entries_keep_independent_ids(store):
    first = history.append({"title": "dup", "time": 1})
    second = history.append({"title": "dup", "time": 1})

    assert first != second
    history.delete(second)
    assert [row["id"] for row in history.list_entries()] == [first]


def test_unknown_id_is_a_no_op(store):
    history.append({"title": "a", "time": 1})
    assert history.delete("history_missing") == {
        "deleted": False,
        "id": "history_missing",
        "remaining": 1,
    }
