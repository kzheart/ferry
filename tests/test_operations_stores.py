"""operations 三个 SQLite store 的「写入→重开→读回」往返回归网。"""
import pytest

from engine.operations.metadata_store import merge_metadata, metadata_key
from engine.operations.plan_store import OperationPlan
from engine.storage.database import StateDatabase


def _open(tmp_path, *, recover_interrupted=False):
    return StateDatabase(
        tmp_path / "ferry-state.sqlite3",
        recover_interrupted=recover_interrupted,
    )


def _plan(plan_id="plan_0123456789abcdef"):
    return OperationPlan(
        plan_id=plan_id,
        kind="edit",
        input_json='{"kind":"edit"}',
        preview_json='{"turns":1}',
        input_digest="digest-input",
        preview_digest="digest-preview",
        base_revision="rev-1",
        document_revision="doc-1",
        created_at=1_000,
        expires_at=601_000,
    )


def test_operation_plan_round_trips_across_reopen(tmp_path):
    _open(tmp_path).operations.store_plan(_plan(), 1_000)

    row = _open(tmp_path).operations.get("plan_0123456789abcdef")

    assert row["kind"] == "edit"
    assert row["status"] == "planned"
    assert row["input_json"] == '{"kind":"edit"}'
    assert row["base_revision"] == "rev-1"
    assert row["document_revision"] == "doc-1"
    assert row["expires_at"] == 601_000


def test_unknown_plan_reads_back_as_none(tmp_path):
    assert _open(tmp_path).operations.get("plan_missing") is None


def test_plan_lifecycle_transitions_are_persisted(tmp_path):
    database = _open(tmp_path)
    database.operations.store_plan(_plan(), 1_000)

    assert database.operations.enqueue("plan_0123456789abcdef", 1_100)
    assert database.operations.claim_queued("plan_0123456789abcdef", 1_200)
    database.operations.finish(
        "plan_0123456789abcdef", '{"ok":true}', "digest-result", 1_300,
    )

    row = _open(tmp_path).operations.get("plan_0123456789abcdef")
    assert row["status"] == "applied"
    assert row["result_json"] == '{"ok":true}'
    assert row["error_type"] is None
    assert [
        event["event"]
        for event in _open(tmp_path).operations.audit("plan_0123456789abcdef")
    ] == ["planned", "queued", "applying", "applied"]


def test_transition_from_unexpected_status_is_refused(tmp_path):
    database = _open(tmp_path)
    database.operations.store_plan(_plan(), 1_000)
    assert database.operations.claim("plan_0123456789abcdef", 1_100)

    assert database.operations.claim("plan_0123456789abcdef", 1_200) is False
    database.operations.expire("plan_0123456789abcdef", 1_300)
    assert database.operations.get("plan_0123456789abcdef")["status"] == "applying"
    with pytest.raises(RuntimeError, match="失败状态提交失败"):
        database.operations.fail("plan_missing", "Boom", 1_400)


def test_recover_interrupted_fails_queued_and_applying_on_reopen(tmp_path):
    database = _open(tmp_path)
    database.operations.store_plan(_plan("plan_aaaaaaaaaaaaaaaa"), 1_000)
    database.operations.store_plan(_plan("plan_bbbbbbbbbbbbbbbb"), 1_000)
    database.operations.store_plan(_plan("plan_cccccccccccccccc"), 1_000)
    assert database.operations.enqueue("plan_aaaaaaaaaaaaaaaa", 1_100)
    assert database.operations.claim("plan_bbbbbbbbbbbbbbbb", 1_100)

    reopened = _open(tmp_path, recover_interrupted=True).operations

    assert reopened.get("plan_aaaaaaaaaaaaaaaa")["status"] == "failed"
    assert reopened.get("plan_bbbbbbbbbbbbbbbb")["error_type"] == "EngineRestarted"
    assert reopened.get("plan_cccccccccccccccc")["status"] == "planned"


def test_deletion_recovery_round_trips_and_transitions(tmp_path):
    database = _open(tmp_path)
    database.operations.store_recovery(
        "recovery_0123456789abcdef", "claude", "snapshot-body", 2_000,
    )

    reopened = _open(tmp_path).operations
    recovery = reopened.get_recovery("recovery_0123456789abcdef")
    assert recovery["tool"] == "claude"
    assert recovery["snapshot"] == "snapshot-body"
    assert recovery["status"] == "available"

    assert reopened.claim_recovery("recovery_0123456789abcdef", 2_100)
    assert reopened.claim_recovery("recovery_0123456789abcdef", 2_200) is False
    assert reopened.complete_recovery("recovery_0123456789abcdef", 2_300)
    assert _open(tmp_path).operations.get_recovery(
        "recovery_0123456789abcdef",
    )["status"] == "restored"


def test_released_recovery_returns_to_available(tmp_path):
    database = _open(tmp_path)
    database.operations.store_recovery("recovery_x", "claude", "snap", 1)
    assert database.operations.claim_recovery("recovery_x", 2)

    assert database.operations.release_recovery("recovery_x", 3)
    assert _open(tmp_path).operations.get_recovery("recovery_x")["status"] == (
        "available"
    )


def test_migration_history_round_trips_newest_first(tmp_path):
    database = _open(tmp_path)
    database.migration_history.append("history-1", {"tool": "claude", "at": 1})
    database.migration_history.append("history-2", {"tool": "codex", "at": 2})

    entries = _open(tmp_path).migration_history.list_all()

    assert entries == [
        {"id": "history-2", "tool": "codex", "at": 2},
        {"id": "history-1", "tool": "claude", "at": 1},
    ]


def test_migration_history_delete_reports_remaining(tmp_path):
    database = _open(tmp_path)
    database.migration_history.append("history-1", {"tool": "claude"})
    database.migration_history.append("history-2", {"tool": "codex"})

    assert database.migration_history.delete("history-1") == {
        "deleted": True, "id": "history-1", "remaining": 1,
    }
    assert database.migration_history.delete("history-1") == {
        "deleted": False, "id": "history-1", "remaining": 1,
    }
    assert [
        entry["id"] for entry in _open(tmp_path).migration_history.list_all()
    ] == ["history-2"]


def test_session_metadata_round_trips_across_reopen(tmp_path):
    _open(tmp_path).metadata.set(
        "claude", "session-1", {"name": "名称", "tags": ["a", "b"]}, 5,
    )

    assert _open(tmp_path).metadata.list_all() == {
        metadata_key("claude", "session-1"): {
            "name": "名称", "tags": ["a", "b"],
        },
    }


def test_session_metadata_patch_merges_and_prunes_empty_values(tmp_path):
    database = _open(tmp_path)
    database.metadata.set("claude", "session-1", {"name": "旧", "pinned": True}, 1)

    merged = database.metadata.set("claude", "session-1", {"name": "新"}, 2)

    assert merged == {"name": "新", "pinned": True}
    assert database.metadata.set(
        "claude", "session-1", {"pinned": False}, 3,
    ) == {"name": "新"}
    assert _open(tmp_path).metadata.list_all() == {
        metadata_key("claude", "session-1"): {"name": "新"},
    }


def test_session_metadata_entry_is_deleted_when_it_becomes_empty(tmp_path):
    database = _open(tmp_path)
    database.metadata.set("claude", "session-1", {"pinned": True}, 1)

    assert database.metadata.set("claude", "session-1", {"pinned": False}, 2) == {}
    assert _open(tmp_path).metadata.list_all() == {}


def test_session_metadata_compare_and_set_rolls_back_on_mismatch(tmp_path):
    database = _open(tmp_path)
    database.metadata.set("claude", "one", {"name": "before"}, 1)

    assert database.metadata.compare_and_set([
        ("claude", "one", {"name": "wrong"}, {"name": "after"}),
        ("codex", "two", {}, {"pinned": True}),
    ], 2) is None
    assert _open(tmp_path).metadata.list_all() == {
        metadata_key("claude", "one"): {"name": "before"},
    }


def test_merge_metadata_drops_falsey_values():
    assert merge_metadata(
        {"name": "旧", "tags": ["a"], "pinned": True},
        {"name": "新", "tags": [], "pinned": None},
    ) == {"name": "新"}
