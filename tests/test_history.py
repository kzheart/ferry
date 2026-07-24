"""迁移历史的 SQLite 读取与删除。"""

import pytest

from engine.application import history
from engine.application.ports import current
from engine.infrastructure.state_db import StateDatabase


@pytest.fixture
def store(tmp_path, monkeypatch):
    database = StateDatabase(
        tmp_path / "ferry-state.sqlite3", recover_interrupted=False,
    )
    monkeypatch.setattr(history, "_database", lambda _ports: database)
    return database


def test_empty_database_lists_nothing(store):
    assert history.list_entries(current()) == []


def test_entries_are_newest_first_with_stable_database_ids(store):
    first = history.append({"title": "a", "time": 1}, current())
    second = history.append({"title": "b", "time": 2}, current())

    assert [row["title"] for row in history.list_entries(current())] == ["b", "a"]
    assert [row["id"] for row in history.list_entries(current())] == [second, first]


def test_delete_removes_only_the_named_entry(store):
    history.append({"title": "a", "time": 1}, current())
    target = history.append({"title": "b", "time": 2}, current())

    assert history.delete(target, current()) == {"deleted": True, "id": target, "remaining": 1}
    assert [row["title"] for row in history.list_entries(current())] == ["a"]


def test_duplicate_entries_keep_independent_ids(store):
    first = history.append({"title": "dup", "time": 1}, current())
    second = history.append({"title": "dup", "time": 1}, current())

    assert first != second
    history.delete(second, current())
    assert [row["id"] for row in history.list_entries(current())] == [first]


def test_unknown_id_is_a_no_op(store):
    history.append({"title": "a", "time": 1}, current())
    assert history.delete("history_missing", current()) == {
        "deleted": False,
        "id": "history_missing",
        "remaining": 1,
    }
