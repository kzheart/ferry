import copy
import json
import sqlite3
from concurrent.futures import ThreadPoolExecutor
from dataclasses import replace
from threading import Barrier

import pytest

from engine.application import operations, services, session_meta
from engine.application.ports import current
from engine.domain.edit import AssistantReply
from engine.domain.errors import (
    AgentRequestError,
    ConcurrentModificationError,
    InvalidReplyError,
    OperationUnsupportedError,
)
from engine.infrastructure.state_db import StateDatabase
from test_agent_tools import _claude_ref, agent_environment


@pytest.fixture(autouse=True)
def operation_service():
    operations.reset_service()
    yield
    operations.reset_service()


def _plan(ops=None, probe=False):
    return operations.plan({
        "kind": "edit",
        "tool": "claude",
        "ref": _claude_ref(),
        "ops": ops or [{"op": "delete-turn", "turn": 1}],
        "probe": probe,
    })


def _migration_plan(**overrides):
    value = {
        "kind": "migration",
        "source_tool": "claude",
        "ref": _claude_ref(),
        "target_tool": "opencode",
        "probe": False,
    }
    value.update(overrides)
    return operations.plan(value)


def _metadata_plan(**overrides):
    value = {
        "kind": "metadata",
        "tool": "claude",
        "ref": _claude_ref(),
        "patch": {"name": "新名称"},
    }
    value.update(overrides)
    return operations.plan(value)


def _delete_plan(**overrides):
    value = {
        "kind": "delete",
        "tool": "claude",
        "ref": _claude_ref(),
    }
    value.update(overrides)
    return operations.plan(value)


def _attach_reply_editing(monkeypatch, *, inplace=True):
    editor = current().adapter("claude").editor
    calls = []
    original_capabilities = editor.capabilities

    def capabilities():
        result = original_capabilities()
        result["operation_modes"] = dict(result["operation_modes"])
        result["operation_modes"]["replace-assistant-reply"] = (
            ["inplace"] if inplace else []
        )
        result["operations"] = [
            *result["operations"],
            "replace-assistant-reply",
        ]
        return result

    def replace_reply(_doc, turn, reply: AssistantReply):
        calls.append((turn, reply.to_dict()))
        return [{"code": "reply.replaced", "turn": turn}]

    monkeypatch.setattr(editor, "capabilities", capabilities)
    monkeypatch.setattr(editor, "replace_reply", replace_reply, raising=False)
    return calls


def _attach_lifecycle(monkeypatch, transcript):
    class Lifecycle:
        delete_undoable = True

        def __init__(self):
            self.calls = []

        def delete(self, _plugin, ref):
            self.calls.append(ref)
            transcript.unlink()
            return {
                "ok": True,
                "snapshot": "snapshot-before-delete",
                "undoable": True,
            }

    ports = current()
    original_adapter = ports.adapter
    lifecycle = Lifecycle()
    plugin = replace(original_adapter("claude"), lifecycle=lifecycle)
    monkeypatch.setattr(
        ports,
        "adapter",
        lambda tool: plugin if tool == "claude" else original_adapter(tool),
    )
    return lifecycle


def test_plan_freezes_input_and_apply_only_uses_plan_id(agent_environment):
    requested_ops = [{"op": "delete-turn", "turn": 1}]
    plan = _plan(requested_ops)
    requested_ops[0]["turn"] = 2

    assert plan["status"] == "planned"
    assert plan["base_revision"]
    assert plan["document_revision"] == "revision-1"
    assert plan["input_digest"]
    assert plan["preview_digest"]

    applied = operations.apply(plan["plan_id"])

    assert applied["status"] == "applied"
    assert agent_environment["editor"].last_ops == [
        {"op": "delete-turn", "turn": 1},
    ]
    assert applied["result"]["snapshot"] == "snapshot-before-agent-edit"
    assert operations.status(plan["plan_id"])["status"] == "applied"


def test_plan_survives_operation_service_restart(agent_environment):
    plan = _plan()

    operations.reset_service()

    assert operations.status(plan["plan_id"])["status"] == "planned"
    result = operations.apply(plan["plan_id"])
    assert result["status"] == "applied"
    assert agent_environment["editor"].commits == 1


def test_restart_marks_interrupted_apply_failed(agent_environment):
    plan = _plan()
    database = StateDatabase(
        agent_environment["root"].parent / "ferry-state.sqlite3",
    )
    assert database.claim(plan["plan_id"], 2_000)

    operations.reset_service()

    status = operations.status(plan["plan_id"])
    assert status["status"] == "failed"
    assert status["error_type"] == "EngineRestarted"


def test_operation_audit_contains_digests_not_raw_content(
        agent_environment, monkeypatch):
    _attach_reply_editing(monkeypatch)
    secret = "Bearer operation-audit-secret"
    plan = _plan([{
        "op": "replace-assistant-reply",
        "turn": 1,
        "reply": {"items": [{"kind": "text", "text": secret}]},
    }])

    operations.apply(plan["plan_id"])
    encoded = json.dumps(
        operations.audit(plan["plan_id"]), ensure_ascii=False,
    )

    assert secret not in encoded
    assert [item["event"] for item in operations.audit(plan["plan_id"])] == [
        "planned", "applying", "applied",
    ]


def test_state_database_rejects_unknown_exact_schema(tmp_path):
    path = tmp_path / "ferry-state.sqlite3"
    with sqlite3.connect(path) as connection:
        connection.execute("PRAGMA user_version = 99")

    with pytest.raises(RuntimeError, match="schema 不受支持"):
        StateDatabase(path)


def test_state_database_rejects_previous_schema_without_migration(tmp_path):
    path = tmp_path / "ferry-state.sqlite3"
    with sqlite3.connect(path) as connection:
        connection.execute("PRAGMA user_version = 1")

    with pytest.raises(RuntimeError, match="schema 不受支持"):
        StateDatabase(path)


@pytest.mark.parametrize("version", [2, 3, 4, 5, 6, 7])
def test_state_database_rejects_previous_current_schema_without_migration(tmp_path, version):
    path = tmp_path / "ferry-state.sqlite3"
    with sqlite3.connect(path) as connection:
        connection.execute(f"PRAGMA user_version = {version}")

    with pytest.raises(RuntimeError, match="schema 不受支持"):
        StateDatabase(path)


def test_session_metadata_batch_cas_is_atomic(tmp_path):
    database = StateDatabase(tmp_path / "ferry-state.sqlite3", recover_interrupted=False)
    database.set_session_metadata("claude", "one", {"name": "before"}, 1)
    database.set_session_metadata("codex", "two", {"pinned": True}, 1)

    changed = database.compare_and_set_session_metadata([
        ("claude", "one", {"name": "before"}, {"name": "after"}),
        ("codex", "two", {}, {"archived": True}),
    ], 2)

    assert changed is None
    assert database.list_session_metadata() == {
        "claude\0one": {"name": "before"},
        "codex\0two": {"pinned": True},
    }


def test_session_metadata_isolated_by_tool_and_native_session_id(tmp_path):
    database = StateDatabase(tmp_path / "ferry-state.sqlite3", recover_interrupted=False)
    database.set_session_metadata("claude", "shared-id", {"name": "Claude"}, 1)
    database.set_session_metadata("codex", "shared-id", {"name": "Codex"}, 2)

    assert database.list_session_metadata() == {
        "claude\0shared-id": {"name": "Claude"},
        "codex\0shared-id": {"name": "Codex"},
    }


def test_metadata_query_does_not_fail_an_applying_operation(agent_environment):
    plan = _plan()
    database = StateDatabase(agent_environment["root"].parent / "ferry-state.sqlite3")
    assert database.claim(plan["plan_id"], 2_000)

    assert services.session_meta_list() == {}
    assert operations.status(plan["plan_id"])["status"] == "applying"


def test_metadata_plan_applies_with_independent_cas(agent_environment):
    plan = _metadata_plan()

    assert plan["kind"] == "metadata"
    assert plan["risk"] == "low"
    assert plan["preview"]["before"] == {}
    assert plan["preview"]["after_patch"] == {"name": "新名称"}

    applied = operations.apply(plan["plan_id"])

    assert applied["result"]["metadata"] == {"name": "新名称"}
    assert services.session_meta_list()["claude\0private-id"] == {"name": "新名称"}


def test_metadata_plan_rejects_concurrent_metadata_change(agent_environment):
    plan = _metadata_plan()
    session_meta.set_entry(
        "claude", "private-id", {"name": "并发名称"}, current())

    with pytest.raises(
            ConcurrentModificationError, match="元数据在审批后已变化"):
        operations.apply(plan["plan_id"])

    assert operations.status(plan["plan_id"])["status"] == "failed"
    assert services.session_meta_list()["claude\0private-id"] == {"name": "并发名称"}


@pytest.mark.parametrize("patch", [
    {},
    {"unknown": True},
    {"pinned": "yes"},
    {"tags": [""]},
])
def test_metadata_plan_rejects_invalid_patch(agent_environment, patch):
    with pytest.raises(AgentRequestError):
        _metadata_plan(patch=patch)


def test_delete_plan_is_read_only_and_apply_uses_lifecycle_snapshot(
        agent_environment, monkeypatch):
    lifecycle = _attach_lifecycle(
        monkeypatch, agent_environment["transcript"],
    )

    plan = _delete_plan()

    assert plan["kind"] == "delete"
    assert plan["risk"] == "high"
    assert plan["preview"]["undoable"] is True
    assert plan["preview"]["session_id"] == "private-id"
    assert agent_environment["transcript"].exists()
    assert lifecycle.calls == []

    applied = operations.apply(plan["plan_id"])

    assert applied["result"]["recovery_id"].startswith("recovery_")
    assert "snapshot" not in applied["result"]
    assert lifecycle.calls == [str(agent_environment["transcript"])]
    assert not agent_environment["transcript"].exists()

    restored = []
    monkeypatch.setattr(
        operations.OperationService,
        "_restore_deleted_session",
        lambda _self, snapshot: restored.append(snapshot) or {
            "ok": True,
            "target": str(agent_environment["transcript"]),
        },
    )
    restore_plan = operations.plan({
        "kind": "restore-delete",
        "recovery_id": applied["result"]["recovery_id"],
    })
    restore_result = operations.apply(restore_plan["plan_id"])

    assert restore_plan["preview"]["tool"] == "claude"
    assert restore_result["result"]["recovery_id"] == \
        applied["result"]["recovery_id"]
    assert restored == ["snapshot-before-delete"]
    with pytest.raises(AgentRequestError):
        operations.plan({
            "kind": "restore-delete",
            "recovery_id": applied["result"]["recovery_id"],
        })


def test_delete_plan_rejects_revision_change(
        agent_environment, monkeypatch):
    lifecycle = _attach_lifecycle(
        monkeypatch, agent_environment["transcript"],
    )
    plan = _delete_plan()
    agent_environment["claude_browser"].fingerprint_value = "changed"

    with pytest.raises(ConcurrentModificationError):
        operations.apply(plan["plan_id"])

    assert lifecycle.calls == []
    assert agent_environment["transcript"].exists()


def test_replace_assistant_reply_plans_and_applies_as_edit(
        agent_environment, monkeypatch):
    calls = _attach_reply_editing(monkeypatch)
    requested = {
        "items": [{"kind": "text", "text": "新的回答"}],
    }
    plan = _plan([{
        "op": "replace-assistant-reply",
        "turn": 1,
        "reply": requested,
    }])
    requested["items"][0]["text"] = "计划后篡改"

    assert plan["preview"]["changes"] == [{
        "code": "reply.replaced",
        "turn": 1,
    }]
    result = operations.apply(plan["plan_id"])["result"]

    assert result["snapshot"] == "snapshot-before-agent-edit"
    assert calls == [
        (1, {"items": [{"kind": "text", "text": "新的回答"}]}),
        (1, {"items": [{"kind": "text", "text": "新的回答"}]}),
    ]
    assert agent_environment["editor"].commits == 1


def test_replace_reply_can_be_combined_with_native_edit_ops(
        agent_environment, monkeypatch):
    _attach_reply_editing(monkeypatch)

    plan = _plan([
        {"op": "delete-turn", "turn": 1},
        {
            "op": "replace-assistant-reply",
            "turn": 1,
            "reply": {"items": [{"kind": "text", "text": "保留轮次的新回答"}]},
        },
    ])

    assert plan["preview"]["changes"] == [
        {"code": "turn.deleted"},
        {"code": "reply.replaced", "turn": 1},
    ]
    operations.apply(plan["plan_id"])
    assert agent_environment["editor"].last_ops == [{
        "op": "delete-turn",
        "turn": 1,
    }]


@pytest.mark.parametrize("op,error", [
    (
        {
            "op": "replace-assistant-reply",
            "turn": 0,
            "reply": {"items": [{"kind": "text", "text": "x"}]},
        },
        AgentRequestError,
    ),
    (
        {
            "op": "replace-assistant-reply",
            "turn": 1,
            "reply": {"items": []},
        },
        InvalidReplyError,
    ),
    (
        {
            "op": "replace-assistant-reply",
            "turn": 1,
            "reply": {"items": [{"kind": "text", "text": "x"}]},
            "unexpected": True,
        },
        AgentRequestError,
    ),
])
def test_replace_reply_rejects_invalid_current_shape(
        agent_environment, op, error):
    with pytest.raises(error):
        _plan([op])


def test_replace_reply_requires_editor_operation(agent_environment):
    with pytest.raises(OperationUnsupportedError):
        _plan([{
            "op": "replace-assistant-reply",
            "turn": 1,
            "reply": {"items": [{"kind": "text", "text": "x"}]},
        }])


def test_replace_reply_requires_inplace_editor_operation(
        agent_environment, monkeypatch):
    _attach_reply_editing(monkeypatch, inplace=False)

    with pytest.raises(OperationUnsupportedError):
        _plan([{
            "op": "replace-assistant-reply",
            "turn": 1,
            "reply": {"items": [{"kind": "text", "text": "x"}]},
        }])
    assert agent_environment["editor"].commits == 0


def test_replace_reply_keeps_document_revision_cas(
        agent_environment, monkeypatch):
    calls = _attach_reply_editing(monkeypatch)
    plan = _plan([{
        "op": "replace-assistant-reply",
        "turn": 1,
        "reply": {"items": [{"kind": "text", "text": "x"}]},
    }])
    editor = agent_environment["editor"]
    original_load = editor.load

    def stale_load(ref):
        document = original_load(ref)
        document.revision = "revision-2"
        return document

    monkeypatch.setattr(editor, "load", stale_load)

    with pytest.raises(ConcurrentModificationError):
        operations.apply(plan["plan_id"])
    assert calls == [
        (1, {"items": [{"kind": "text", "text": "x"}]}),
    ]
    assert editor.commits == 0


def test_replace_reply_keeps_probe_in_mutation_finish(
        agent_environment, monkeypatch):
    _attach_reply_editing(monkeypatch)
    calls = []
    monkeypatch.setattr(
        operations.OperationService,
        "_finish_mutation",
        lambda _self, tool, editor, result, doc, snapshot, probe:
            calls.append((probe, snapshot)) or result,
    )

    plan = _plan([{
        "op": "replace-assistant-reply",
        "turn": "turn-locator-1",
        "reply": {"items": [{"kind": "text", "text": "x"}]},
    }], probe=True)
    operations.apply(plan["plan_id"])

    assert calls == [(True, "snapshot-before-agent-edit")]


def test_migration_plan_and_apply_reuse_current_migration_service(
        agent_environment, monkeypatch):
    calls = []

    def preview_migration(_self, src, dst, ref, **kwargs):
        calls.append((src, dst, ref, kwargs))
        return {
            "src": src,
            "dst": dst,
            "loss": {"degrade": 1},
            "preview": {"target_tool": dst, "root": {"messages": []}},
        }

    def apply_migration(_self, src, dst, ref, **kwargs):
        calls.append((src, dst, ref, kwargs))
        return {
            "src": src,
            "dst": dst,
            "session_id": "migrated",
            "validation": {
                "structure": {"ok": True, "detail": "ok"},
                "runtime": {"status": "passed"},
            },
        }

    monkeypatch.setattr(operations.MigrationService, "preview", preview_migration)
    monkeypatch.setattr(operations.MigrationService, "apply", apply_migration)
    plan = _migration_plan(
        max_turn=3,
        probe=True,
        probe_model="provider/model",
    )

    assert plan["kind"] == "migration"
    assert plan["document_revision"] is None
    assert plan["preview"]["loss"] == {"degrade": 1}
    assert plan["preview"]["preview"]["target_tool"] == "opencode"
    assert calls[0][2] == str(agent_environment["transcript"])
    assert calls[0][3]["max_turn"] == 3
    assert calls[0][3]["probe_model"] == "provider/model"
    assert calls[0][3]["session"].source_id == "private-source-id"

    applied = operations.apply(plan["plan_id"])

    assert applied["status"] == "applied"
    assert applied["result"]["session_id"] == "migrated"
    assert calls[1][2] == str(agent_environment["transcript"])
    assert calls[1][3]["max_turn"] == 3
    assert calls[1][3]["probe"] is True
    assert calls[1][3]["probe_model"] == "provider/model"
    assert operations.status(plan["plan_id"])["status"] == "applied"


def test_migration_apply_rejects_changed_source_revision(
        agent_environment, monkeypatch):
    calls = []

    def preview_migration(_self, src, dst, ref, **kwargs):
        calls.append(kwargs)
        return {
            "preview": {},
            "loss": {},
        }

    monkeypatch.setattr(operations.MigrationService, "preview", preview_migration)
    plan = _migration_plan()
    agent_environment["claude_browser"].fingerprint_value = "changed"

    with pytest.raises(ConcurrentModificationError):
        operations.apply(plan["plan_id"])

    assert len(calls) == 1
    assert operations.status(plan["plan_id"])["status"] == "failed"


@pytest.mark.parametrize("actual", [
    {
        "rolled_back": True,
        "validation": {"structure": {"ok": False}},
    },
    {
        "validation": {"structure": {"ok": False}},
    },
])
def test_migration_failed_validation_marks_operation_failed(
        agent_environment, monkeypatch, actual):
    def preview_migration(_self, _src, _dst, _ref, **_kwargs):
        return {"preview": {}, "loss": {}}

    monkeypatch.setattr(operations.MigrationService, "preview", preview_migration)
    monkeypatch.setattr(
        operations.MigrationService, "apply",
        lambda _self, _src, _dst, _ref, **_kwargs: actual,
    )
    plan = _migration_plan()

    with pytest.raises(RuntimeError, match="结构校验失败"):
        operations.apply(plan["plan_id"])

    status = operations.status(plan["plan_id"])
    assert status["status"] == "failed"
    assert status["error_type"] == "RuntimeError"


def test_migration_plan_rejects_source_change_during_preview(
        agent_environment, monkeypatch):
    def preview_migration(_self, _src, _dst, _ref, **_kwargs):
        agent_environment["claude_browser"].fingerprint_value = "changed"
        return {"preview": {}, "loss": {}}

    monkeypatch.setattr(operations.MigrationService, "preview", preview_migration)

    with pytest.raises(ConcurrentModificationError):
        _migration_plan()


@pytest.mark.parametrize("patch", [
    {"unexpected": True},
    {"source_tool": ""},
    {"target_tool": ""},
    {"max_turn": True},
    {"max_turn": 0},
    {"probe": "yes"},
    {"probe_model": ""},
])
def test_migration_plan_rejects_invalid_current_input(
        agent_environment, patch):
    with pytest.raises(AgentRequestError):
        _migration_plan(**patch)


def test_probe_setting_is_frozen_in_the_plan(agent_environment, monkeypatch):
    calls = []
    monkeypatch.setattr(
        operations.OperationService,
        "_finish_mutation",
        lambda _self, tool, editor, result, doc, snapshot, probe:
            calls.append(probe) or result,
    )

    plan = _plan(probe=True)
    operations.apply(plan["plan_id"])

    assert calls == [True]


def test_cancelled_plan_never_writes(agent_environment):
    plan = _plan()

    assert operations.cancel(plan["plan_id"]) == {
        "plan_id": plan["plan_id"],
        "status": "cancelled",
    }
    with pytest.raises(AgentRequestError, match="不可执行"):
        operations.apply(plan["plan_id"])

    assert agent_environment["editor"].commits == 0
    assert operations.status(plan["plan_id"])["status"] == "cancelled"


def test_plan_can_only_be_applied_once(agent_environment):
    plan = _plan()

    operations.apply(plan["plan_id"])

    with pytest.raises(AgentRequestError, match="不可执行"):
        operations.apply(plan["plan_id"])
    assert agent_environment["editor"].commits == 1


def test_concurrent_apply_only_commits_once(agent_environment):
    plan = _plan()
    barrier = Barrier(2)

    def apply_plan():
        barrier.wait()
        try:
            return ("ok", operations.apply(plan["plan_id"]))
        except Exception as error:
            return ("error", error)

    with ThreadPoolExecutor(max_workers=2) as executor:
        results = list(executor.map(lambda _: apply_plan(), range(2)))

    successes = [value for kind, value in results if kind == "ok"]
    failures = [value for kind, value in results if kind == "error"]
    assert len(successes) == 1
    assert len(failures) == 1
    assert isinstance(failures[0], AgentRequestError)
    assert agent_environment["editor"].commits == 1
    assert operations.status(plan["plan_id"])["status"] == "applied"


def test_apply_rejects_changed_index_revision(agent_environment):
    plan = _plan()
    agent_environment["claude_browser"].fingerprint_value = "changed"

    with pytest.raises(ConcurrentModificationError):
        operations.apply(plan["plan_id"])

    status = operations.status(plan["plan_id"])
    assert status["status"] == "failed"
    assert status["error_type"] == "ConcurrentModificationError"
    assert agent_environment["editor"].commits == 0


def test_apply_rejects_changed_document_revision(
        agent_environment, monkeypatch):
    plan = _plan()
    editor = agent_environment["editor"]
    original_load = editor.load

    def stale_load(ref):
        document = original_load(ref)
        document.revision = "revision-2"
        return document

    monkeypatch.setattr(editor, "load", stale_load)

    with pytest.raises(ConcurrentModificationError):
        operations.apply(plan["plan_id"])

    assert operations.status(plan["plan_id"])["status"] == "failed"
    assert editor.commits == 0


def test_commit_failure_restores_snapshot_and_marks_plan_failed(
        agent_environment, monkeypatch):
    plan = _plan()
    editor = agent_environment["editor"]
    restored = []

    monkeypatch.setattr(
        editor, "commit",
        lambda _doc: (_ for _ in ()).throw(RuntimeError("commit failed")),
    )
    monkeypatch.setattr(
        editor, "restore_snapshot",
        lambda snapshot, doc: restored.append((snapshot, copy.copy(doc))),
    )

    with pytest.raises(RuntimeError, match="commit failed"):
        operations.apply(plan["plan_id"])

    assert restored[0][0] == "snapshot-before-agent-edit"
    assert operations.status(plan["plan_id"])["status"] == "failed"


def test_expired_plan_cannot_be_applied(
        agent_environment, monkeypatch):
    clock = [1_000]
    monkeypatch.setattr(operations, "_now_ms", lambda: clock[0])
    plan = _plan()
    clock[0] += operations.PLAN_TTL_MS + 1

    assert operations.status(plan["plan_id"])["status"] == "expired"
    with pytest.raises(AgentRequestError, match="不可执行"):
        operations.apply(plan["plan_id"])
    assert agent_environment["editor"].commits == 0


def test_unknown_operation_kind_is_rejected(agent_environment):
    with pytest.raises(AgentRequestError, match="kind 非法"):
        operations.plan({
            "kind": "unknown",
        })


def test_plan_rejects_edit_without_inplace_support(
        agent_environment, monkeypatch):
    monkeypatch.setattr(
        agent_environment["editor"], "capabilities",
        lambda: {"inplace": False, "operations": [],
                 "operation_modes": {}},
    )

    with pytest.raises(OperationUnsupportedError):
        _plan()
