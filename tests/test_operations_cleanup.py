import json
from pathlib import Path

import pytest

from engine.app import EngineService
from engine.errors import AgentRequestError
from engine.server.rpc import PROTOCOL, RpcDispatcher
from engine.operations import metadata
from engine.operations import executor as operation_executor
from engine.operations.service import OperationService
from engine.sessions.cleanup import CleanupService

from test_agent_tools import agent_environment as _base_agent_environment


@pytest.fixture
def agent_environment(tmp_path, monkeypatch):
    yield from _base_agent_environment.__wrapped__(tmp_path, monkeypatch)


@pytest.fixture
def cleanup_operations(agent_environment):
    cleanup = CleanupService(
        agent_environment["index"], agent_environment["ports"],
    )
    service = OperationService(
        agent_environment["ports"], agent_environment["index"], cleanup,
    )
    yield service, cleanup
    service.shutdown()


def _inventory_and_triage(service, cleanup, *, agents=None):
    inventory = cleanup.inventory(
        {"agents": agents} if agents is not None else None,
    )
    cleanup.triage(inventory["scope_id"], [
        {"tool": row["tool"], "ref": row["ref"], "verdict": "delete"}
        for row in inventory["page"]
    ])
    return inventory


def _plan(service, inventory, rows=None):
    rows = rows or inventory["page"]
    return service.plan({
        "kind": "cleanup",
        "scope_id": inventory["scope_id"],
        "targets": [
            {"tool": row["tool"], "ref": row["ref"], "reason": "测试清理"}
            for row in rows
        ],
    })


def _apply(service, plan):
    accepted = service.apply(plan["plan_id"])
    assert accepted["status"] in {"queued", "applying", "applied"}
    return service.wait(plan["plan_id"], timeout=5)


def _add_claude_candidates(environment):
    root = Path(environment["root"])
    browser = environment["claude_browser"]
    candidates = [
        ("pinned-id", "pinned-title"),
        ("archived-id", "archived-title"),
        ("tagged-id", "tagged-title"),
    ]
    for offset, (session_id, title) in enumerate(candidates, start=1):
        path = root / f"cleanup-{offset}.jsonl"
        path.write_text("{}\n")
        browser.rows.append({
            "tool": "claude",
            "id": session_id,
            "path": str(path),
            "dir": "/Users/private/secret-project",
            "title": title,
            "updated": 3000 - offset,
            "count": 1,
            "size": path.stat().st_size,
        })
    environment["index"].refresh()
    ports = environment["ports"]
    metadata.set_entry("claude", "pinned-id", {"pinned": True}, ports)
    metadata.set_entry("claude", "archived-id", {"archived": True}, ports)
    metadata.set_entry("claude", "tagged-id", {"tags": ["retain"]}, ports)


def test_plan_rejects_incomplete_coverage(cleanup_operations, agent_environment):
    service, cleanup = cleanup_operations
    inventory = cleanup.inventory({"agents": ["claude"]})
    row = inventory["page"][0]

    with pytest.raises(AgentRequestError, match="尚有 1 条"):
        _plan(service, inventory, [row])


def test_plan_rejects_target_without_delete_verdict(
        cleanup_operations, agent_environment):
    service, cleanup = cleanup_operations
    inventory = cleanup.inventory({"agents": ["claude"]})
    cleanup.triage(inventory["scope_id"], [{
        "tool": "claude", "ref": inventory["page"][0]["ref"], "verdict": "keep",
    }])

    with pytest.raises(AgentRequestError, match="没有 delete 裁决"):
        _plan(service, inventory)


def test_plan_excludes_pinned_archived_tagged(cleanup_operations, agent_environment):
    _add_claude_candidates(agent_environment)
    service, cleanup = cleanup_operations
    inventory = _inventory_and_triage(
        service, cleanup, agents=["claude"],
    )

    plan = _plan(service, inventory)

    assert plan["preview"]["totals"]["count"] == 1
    assert plan["preview"]["by_tool"] == [{
        "tool": "claude", "count": 1, "size_bytes": plan["preview"]["totals"]["size_bytes"],
    }]
    assert plan["preview"]["undoable"] == {"count": 0, "total": 1}
    assert plan["preview"]["sessions"][0]["project"] == "/Users/private/secret-project"
    assert {entry["cause"] for entry in plan["preview"]["excluded"]} == {
        "pinned", "archived", "tagged",
    }
    assert plan["preview"]["coverage"] == {
        "covered": 4, "total": 4, "scope": inventory["scope_id"],
    }


def test_plan_rejects_stale_generation(cleanup_operations, agent_environment):
    service, cleanup = cleanup_operations
    inventory = _inventory_and_triage(
        service, cleanup, agents=["claude"],
    )
    agent_environment["claude_browser"].fingerprint_value = "changed"
    agent_environment["index"].refresh()

    with pytest.raises(AgentRequestError, match="过期"):
        _plan(service, inventory)


def test_apply_deletes_and_records_recoveries(
        cleanup_operations, agent_environment, monkeypatch):
    service, cleanup = cleanup_operations
    inventory = _inventory_and_triage(
        service, cleanup, agents=["claude"],
    )
    lifecycle = agent_environment["ports"].adapter("claude").lifecycle
    monkeypatch.setattr(lifecycle, "delete_undoable", True, raising=False)
    monkeypatch.setattr(
        lifecycle, "delete",
        lambda _adapter, _ref: {
            "ok": True, "undoable": True, "snapshot": "snapshot-cleanup",
        },
    )
    plan = _plan(service, inventory)

    result = _apply(service, plan)

    assert len(result["result"]["succeeded"]) == 1
    assert len(result["result"]["recovery_ids"]) == 1
    recovery_id = result["result"]["recovery_ids"][0]
    assert service._database().operations.get_recovery(recovery_id)["status"] == "available"


def test_apply_skips_changed_revision_and_continues(
        cleanup_operations, agent_environment):
    service, cleanup = cleanup_operations
    inventory = _inventory_and_triage(
        service, cleanup,
    )
    plan = _plan(service, inventory)
    agent_environment["claude_browser"].fingerprint_value = "changed"

    result = _apply(service, plan)

    assert result["result"]["skipped"] == [{
        "tool": "claude",
        "ref": next(row["ref"] for row in inventory["page"] if row["tool"] == "claude"),
        "cause": "changed",
    }]
    assert [entry["tool"] for entry in result["result"]["succeeded"]] == ["opencode"]


def test_apply_reports_failed_without_rollback(
        cleanup_operations, agent_environment, monkeypatch):
    service, cleanup = cleanup_operations
    inventory = _inventory_and_triage(
        service, cleanup,
    )
    claude_lifecycle = agent_environment["ports"].adapter("claude").lifecycle
    monkeypatch.setattr(
        claude_lifecycle, "delete",
        lambda _adapter, _ref: (_ for _ in ()).throw(RuntimeError("delete failed")),
    )
    plan = _plan(service, inventory)

    result = _apply(service, plan)

    assert result["result"]["failed"][0]["tool"] == "claude"
    assert result["result"]["failed"][0]["error"] == "delete failed"
    assert [entry["tool"] for entry in result["result"]["succeeded"]] == ["opencode"]


def test_restore_each_recovery_roundtrip(
        cleanup_operations, agent_environment, monkeypatch):
    service, cleanup = cleanup_operations
    inventory = _inventory_and_triage(
        service, cleanup,
    )
    for tool in ("claude", "opencode"):
        lifecycle = agent_environment["ports"].adapter(tool).lifecycle
        monkeypatch.setattr(lifecycle, "delete_undoable", True, raising=False)
        monkeypatch.setattr(
            lifecycle, "delete",
            lambda _adapter, ref, tool=tool: {
                "ok": True,
                "undoable": True,
                "snapshot": f"snapshot-{tool}-{ref}",
            },
        )
    plan = _plan(service, inventory)
    result = _apply(service, plan)
    recovery_ids = result["result"]["recovery_ids"]

    monkeypatch.setattr(
        operation_executor.SessionDeletionService,
        "restore",
        lambda _self, snapshot: {"ok": True, "snapshot": snapshot},
    )
    for recovery_id in recovery_ids:
        restore_plan = service.plan({
            "kind": "restore-delete",
            "recovery_id": recovery_id,
        })
        restored = _apply(service, restore_plan)
        assert restored["result"]["recovery_id"] == recovery_id
        assert service._database().operations.get_recovery(recovery_id)["status"] == "restored"


def _record_deletions(agent_environment, monkeypatch, tool="claude"):
    deleted = []
    lifecycle = agent_environment["ports"].adapter(tool).lifecycle
    monkeypatch.setattr(
        lifecycle, "delete",
        lambda _adapter, ref: deleted.append(ref) or {
            "ok": True, "undoable": False,
        },
    )
    return deleted


def test_excluded_sessions_never_reach_plan_input_or_apply(
        cleanup_operations, agent_environment, monkeypatch):
    """预览说"已保护"就必须真的不删:H1 的回归护栏。"""
    _add_claude_candidates(agent_environment)
    service, cleanup = cleanup_operations
    inventory = _inventory_and_triage(service, cleanup, agents=["claude"])
    deleted = _record_deletions(agent_environment, monkeypatch)

    plan = _plan(service, inventory)
    stored, _state = service._plans.get(plan["plan_id"])
    targets = stored.input()["targets"]

    assert [target["session_id"] for target in targets] == ["private-id"]
    assert {entry["cause"] for entry in plan["preview"]["excluded"]} == {
        "pinned", "archived", "tagged",
    }
    assert plan["affected_refs"] == [targets[0]["ref"]]

    result = _apply(service, plan)

    assert [entry["ref"] for entry in result["result"]["succeeded"]] == [
        targets[0]["ref"],
    ]
    survivor = agent_environment["index"].resolve(
        "claude", targets[0]["ref"],
    )
    assert deleted == [survivor.canonical_ref]


def test_apply_skips_sessions_protected_after_planning(
        cleanup_operations, agent_environment, monkeypatch):
    service, cleanup = cleanup_operations
    inventory = _inventory_and_triage(service, cleanup)
    plan = _plan(service, inventory)
    deleted = _record_deletions(agent_environment, monkeypatch)
    metadata.set_entry(
        "claude", "private-id", {"pinned": True}, agent_environment["ports"],
    )

    result = _apply(service, plan)

    assert result["result"]["skipped"] == [{
        "tool": "claude",
        "ref": next(
            row["ref"] for row in inventory["page"] if row["tool"] == "claude"
        ),
        "cause": "protected",
        "protection": "pinned",
    }]
    assert [entry["tool"] for entry in result["result"]["succeeded"]] == [
        "opencode",
    ]
    assert deleted == []


def test_apply_skips_when_indexed_revision_moved_after_plan(
        cleanup_operations, agent_environment):
    """走 revision 逐条比对分支,而不是 resolve 抛 session_changed 那条路。"""
    service, cleanup = cleanup_operations
    inventory = _inventory_and_triage(service, cleanup)
    plan = _plan(service, inventory)
    stored, _state = service._plans.get(plan["plan_id"])
    planned = {
        target["tool"]: target["revision"] for target in stored.input()["targets"]
    }
    agent_environment["claude_browser"].fingerprint_value = "changed"
    agent_environment["index"].refresh()
    record = agent_environment["index"].resolve(
        "claude",
        next(row["ref"] for row in inventory["page"] if row["tool"] == "claude"),
    )

    assert record.revision != planned["claude"]

    result = _apply(service, plan)

    assert [entry["cause"] for entry in result["result"]["skipped"]] == ["changed"]
    assert [entry["tool"] for entry in result["result"]["succeeded"]] == [
        "opencode",
    ]


def test_relative_scope_survives_pagination_and_reaches_plan(
        cleanup_operations, agent_environment):
    """"now-7d" 这类相对时间每次调用解析都不同,续页必须认账本而不是重算 scope。"""
    service, cleanup = cleanup_operations
    first = cleanup.inventory({"updated_before": "now-7d"}, page_size=1)
    second = cleanup.inventory(cursor=first["next_cursor"], page_size=1)

    assert first["scope_id"] == second["scope_id"]
    assert first["total"] == second["total"] == 2
    assert isinstance(first["scope"]["updated_before"], int)
    assert second["next_cursor"] is None
    rows = first["page"] + second["page"]
    assert {row["id"] for row in rows} == {"private-id", "oc-1"}

    cleanup.triage(first["scope_id"], [
        {"tool": row["tool"], "ref": row["ref"], "verdict": "delete"}
        for row in rows
    ])
    plan = _plan(service, {"scope_id": first["scope_id"]}, rows)

    assert plan["preview"]["coverage"] == {
        "covered": 2, "total": 2, "scope": first["scope_id"],
    }


def test_engine_service_and_rpc_share_one_cleanup_ledger(agent_environment):
    """端到端跑 rpc 分发,守住 EngineService 与 planner 必须用同一个账本。"""
    cleanup = CleanupService(
        agent_environment["index"], agent_environment["ports"],
    )
    service = OperationService(
        agent_environment["ports"], agent_environment["index"], cleanup,
    )
    application = EngineService(
        agent_environment["ports"], agent_environment["index"], service,
        cleanup=cleanup,
    )
    dispatcher = RpcDispatcher(application)

    def call(method, params):
        response = json.loads(json.dumps(dispatcher.handle(json.dumps({
            "protocol": PROTOCOL, "id": "cleanup-1",
            "method": method, "params": params,
        }))))
        assert response["ok"] is True, response
        return response["result"]

    try:
        inventory = call("agent_cleanup_inventory", {"scope": {"agents": ["claude"]}})
        assert [row["id"] for row in inventory["page"]] == ["private-id"]

        triaged = call("agent_cleanup_triage", {
            "scope_id": inventory["scope_id"],
            "verdicts": [{
                "tool": "claude",
                "ref": inventory["page"][0]["ref"],
                "verdict": "delete",
            }],
        })
        assert triaged == {"covered": 1, "total": 1, "remaining_sample": []}

        plan = service.plan({
            "kind": "cleanup",
            "scope_id": inventory["scope_id"],
            "targets": [{
                "tool": "claude", "ref": inventory["page"][0]["ref"],
            }],
        })
        assert plan["preview"]["totals"]["count"] == 1
    finally:
        service.shutdown()
