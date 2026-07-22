"""迁移历史的读取与删除。"""

import json

import pytest

from engine.application import history


@pytest.fixture
def store(tmp_path, monkeypatch):
    path = tmp_path / "history.jsonl"
    monkeypatch.setattr(history, "HISTORY", path)
    return path


def _write(path, entries):
    path.write_text("".join(json.dumps(e, ensure_ascii=False) + "\n" for e in entries))


def test_missing_file_lists_nothing(store):
    assert history.list_entries() == []


def test_entries_are_newest_first_with_stable_ids(store):
    _write(store, [{"title": "a", "time": 1}, {"title": "b", "time": 2}])
    rows = history.list_entries()
    assert [r["title"] for r in rows] == ["b", "a"]
    assert history.list_entries()[0]["id"] == rows[0]["id"]


def test_delete_removes_only_the_named_entry(store):
    _write(store, [{"title": "a", "time": 1}, {"title": "b", "time": 2}])
    target = next(r for r in history.list_entries() if r["title"] == "a")
    result = history.delete(target["id"])
    assert result == {"deleted": True, "id": target["id"], "remaining": 1}
    assert [r["title"] for r in history.list_entries()] == ["b"]


def test_ids_survive_an_unrelated_delete(store):
    _write(store, [{"title": t, "time": i} for i, t in enumerate("abc")])
    before = {r["title"]: r["id"] for r in history.list_entries()}
    history.delete(before["b"])
    after = {r["title"]: r["id"] for r in history.list_entries()}
    assert after == {"a": before["a"], "c": before["c"]}


def test_identical_lines_delete_one_at_a_time(store):
    _write(store, [{"title": "dup", "time": 1}] * 2)
    rows = history.list_entries()
    assert rows[0]["id"] != rows[1]["id"]
    history.delete(rows[0]["id"])
    assert len(history.list_entries()) == 1


def test_unknown_id_is_a_no_op(store):
    _write(store, [{"title": "a", "time": 1}])
    assert history.delete("nope") == {"deleted": False, "id": "nope", "remaining": 1}
    assert len(history.list_entries()) == 1


def test_append_then_delete_leaves_the_file_parsable(store):
    history.append({"title": "a", "time": 1})
    history.append({"title": "b", "time": 2})
    history.delete(history.list_entries()[0]["id"])
    assert [r["title"] for r in history.list_entries()] == ["a"]
