"""operations/planner.py 的计划生成回归网。

覆盖五种 operation kind 的正常路径，以及分发、引用、能力、并发四类拒绝路径。
"""
from dataclasses import replace

import pytest

from engine.contracts.agents import AGENT_CAPABILITIES
from engine.errors import (
    AgentCapabilityError,
    AgentReferenceError,
    AgentRequestError,
    ConcurrentModificationError,
)
from engine.operations import service as operations
from test_agent_tools import _claude_ref, agent_environment


@pytest.fixture(autouse=True)
def operation_service(monkeypatch, agent_environment):
    service = operations.OperationService(
        agent_environment["ports"], agent_environment["index"],
    )
    for name in ("plan", "apply", "status", "cancel", "wait", "audit"):
        monkeypatch.setattr(
            operations, name, getattr(service, name), raising=False,
        )
    yield service
    service.shutdown()


def _edit_plan(**overrides):
    value = {
        "kind": "edit",
        "tool": "claude",
        "ref": _claude_ref(),
        "ops": [{"op": "delete-turn", "turn": 1}],
        "probe": False,
    }
    value.update(overrides)
    return operations.plan(value)


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
    value = {"kind": "delete", "tool": "claude", "ref": _claude_ref()}
    value.update(overrides)
    return operations.plan(value)


def _attach_lifecycle(monkeypatch, ports, *, undoable=True):
    class Lifecycle:
        delete_undoable = undoable

        def delete(self, _adapter, ref):
            return {"ok": True, "snapshot": "snap", "undoable": undoable}

    original_adapter = ports.adapter
    adapter = replace(original_adapter("claude"), lifecycle=Lifecycle())
    monkeypatch.setattr(
        ports,
        "adapter",
        lambda tool: adapter if tool == "claude" else original_adapter(tool),
    )


def test_migration_plan_previews_without_writing(agent_environment):
    plan = _migration_plan()

    assert plan["kind"] == "migration"
    assert plan["status"] == "planned"
    assert plan["document_revision"] is None
    assert plan["preview"]["dst"] == "opencode"
    assert plan["preview"]["loss"] == {"events": [], "lossless": True}


def test_delete_plan_preview_exposes_undoable_and_truncated_title(
        agent_environment, monkeypatch):
    _attach_lifecycle(monkeypatch, agent_environment["ports"])

    plan = _delete_plan()

    assert plan["preview"]["tool"] == "claude"
    assert plan["preview"]["undoable"] is True
    assert plan["preview"]["title"] == "支付重构"


def test_planner_rejects_non_object_input(agent_environment):
    with pytest.raises(AgentRequestError, match="必须是 object"):
        operations.plan(["edit"])


@pytest.mark.parametrize("kind", [None, "", "purge", 1])
def test_planner_rejects_unknown_kind(agent_environment, kind):
    with pytest.raises(AgentRequestError, match="kind 非法"):
        operations.plan({"kind": kind, "tool": "claude", "ref": _claude_ref()})


@pytest.mark.parametrize("plan_factory", [
    _edit_plan, _migration_plan, _metadata_plan, _delete_plan,
])
def test_planner_rejects_reference_not_issued_by_index(
        agent_environment, plan_factory):
    with pytest.raises(AgentReferenceError):
        plan_factory(ref="ref-not-issued-by-index")


def test_edit_plan_requires_probe_capability_only_when_requested(
        agent_environment, monkeypatch):
    ports = agent_environment["ports"]
    original_adapter = ports.adapter
    manifest = replace(
        original_adapter("claude").manifest,
        capabilities=tuple(
            capability
            for capability in AGENT_CAPABILITIES
            if capability not in {"probe", "prompt"}
        ),
    )
    adapter = replace(
        original_adapter("claude"), manifest=manifest, verifier=None,
    )
    monkeypatch.setattr(
        ports,
        "adapter",
        lambda tool: adapter if tool == "claude" else original_adapter(tool),
    )

    assert _edit_plan()["status"] == "planned"
    with pytest.raises(AgentCapabilityError, match="probe"):
        _edit_plan(probe=True)


def _drift_revision_after_first_resolve(monkeypatch, index):
    """让 planner 的第二次 resolve 拿到不同 revision，模拟计划期间会话被改写。"""
    original_resolve = index.resolve
    resolved = []

    def drifting_resolve(tool, ref):
        record = original_resolve(tool, ref)
        resolved.append(record)
        if len(resolved) > 1:
            return replace(record, revision=f"revision-{len(resolved)}")
        return record

    monkeypatch.setattr(index, "resolve", drifting_resolve)


def test_edit_plan_rejects_revision_change_during_preview(
        agent_environment, monkeypatch):
    ref = _claude_ref()
    _drift_revision_after_first_resolve(monkeypatch, agent_environment["index"])

    with pytest.raises(ConcurrentModificationError, match="请重新计划"):
        operations.plan({
            "kind": "edit",
            "tool": "claude",
            "ref": ref,
            "ops": [{"op": "delete-turn", "turn": 1}],
            "probe": False,
        })


def test_metadata_plan_rejects_revision_change_during_preview(
        agent_environment, monkeypatch):
    ref = _claude_ref()
    _drift_revision_after_first_resolve(monkeypatch, agent_environment["index"])

    with pytest.raises(ConcurrentModificationError, match="请重新计划"):
        operations.plan({
            "kind": "metadata",
            "tool": "claude",
            "ref": ref,
            "patch": {"name": "新名称"},
        })


def test_delete_plan_rejects_revision_change_during_preview(
        agent_environment, monkeypatch):
    _attach_lifecycle(monkeypatch, agent_environment["ports"])
    ref = _claude_ref()
    _drift_revision_after_first_resolve(monkeypatch, agent_environment["index"])

    with pytest.raises(ConcurrentModificationError, match="请重新计划"):
        operations.plan({"kind": "delete", "tool": "claude", "ref": ref})


def test_restore_delete_plan_rejects_unknown_recovery(agent_environment):
    with pytest.raises(AgentRequestError, match="删除恢复记录不可用"):
        operations.plan({
            "kind": "restore-delete",
            "recovery_id": "recovery_0123456789abcdef",
        })


def test_restore_delete_plan_accepts_available_recovery(
        agent_environment, monkeypatch, operation_service):
    _attach_lifecycle(monkeypatch, agent_environment["ports"])
    database = operation_service._database()
    database.operations.store_recovery(
        "recovery_0123456789abcdef", "claude", "snapshot-body", 1_000,
    )

    plan = operations.plan({
        "kind": "restore-delete",
        "recovery_id": "recovery_0123456789abcdef",
    })

    assert plan["kind"] == "restore-delete"
    assert plan["preview"] == {
        "recovery_id": "recovery_0123456789abcdef", "tool": "claude",
    }
    assert plan["base_revision"] == "available"


def test_restore_delete_plan_rejects_already_restored_recovery(
        agent_environment, monkeypatch, operation_service):
    _attach_lifecycle(monkeypatch, agent_environment["ports"])
    database = operation_service._database()
    database.operations.store_recovery("recovery_x0123456789abc", "claude", "s", 1)
    assert database.operations.claim_recovery("recovery_x0123456789abc", 2)
    assert database.operations.complete_recovery("recovery_x0123456789abc", 3)

    with pytest.raises(AgentRequestError, match="删除恢复记录不可用"):
        operations.plan({
            "kind": "restore-delete",
            "recovery_id": "recovery_x0123456789abc",
        })
