import copy
from concurrent.futures import ThreadPoolExecutor
from threading import Barrier

import pytest

from engine.application import operations
from engine.domain.errors import (
    AgentRequestError,
    ConcurrentModificationError,
    OperationUnsupportedError,
)
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


def test_probe_setting_is_frozen_in_the_plan(agent_environment, monkeypatch):
    calls = []
    monkeypatch.setattr(
        operations.services,
        "_finish_mutation",
        lambda tool, editor, result, doc, snapshot, probe, save_as:
            calls.append((probe, save_as)) or result,
    )

    plan = _plan(probe=True)
    operations.apply(plan["plan_id"])

    assert calls == [(True, False)]


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


def test_only_inplace_edit_is_supported(agent_environment):
    with pytest.raises(AgentRequestError, match="仅支持 edit"):
        operations.plan({
            "kind": "migration",
            "tool": "claude",
            "ref": _claude_ref(),
            "ops": [],
        })


def test_plan_rejects_edit_without_inplace_support(
        agent_environment, monkeypatch):
    monkeypatch.setattr(
        agent_environment["editor"], "supports_mode",
        lambda _ops, _save_as: False,
    )

    with pytest.raises(OperationUnsupportedError):
        _plan()
