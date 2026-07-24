"""T4 整理壳：摘要引用、提案审批、跨 Agent 聚类与行为信号。"""
from __future__ import annotations

import json
from pathlib import Path

import pytest

from engine.application import session_meta
from engine.application.organization import proposals as organizing
from engine.application.organization import summaries
from engine.composition import create_ports
from engine.domain.errors import (
    OrganizationProposalError,
    OrganizationProposalStaleError,
)
from engine.interfaces.rpc import PROTOCOL, rpc
from engine.infrastructure.state_db import StateDatabase


@pytest.fixture
def organization_environment(tmp_path, monkeypatch):
    database = StateDatabase(
        tmp_path / "ferry-state.sqlite3", recover_interrupted=False,
    )
    monkeypatch.setattr(summaries, "_database", lambda _ports: database)
    monkeypatch.setattr(organizing, "_database", lambda _ports: database)
    monkeypatch.setattr(session_meta, "_database", lambda _ports: database)
    return tmp_path


def _seed(tool: str, session_id: str, digest: str,
          fingerprint: str | None = None) -> dict:
    segment_hash = "sha256:" + (session_id[-8:] or "segment")
    record = {
        "tool": tool,
        "id": session_id,
        "fingerprint": fingerprint or "sha256:fp-" + session_id,
        "segments": [{
            "turn": 1,
            "anchor_locator": "message-" + session_id,
            "message_start": 0,
            "message_end": 1,
            "after_compaction": False,
            "hash": segment_hash,
            "char_count": 10,
            "digest": digest,
        }],
    }
    database = summaries._database(_ports())
    database.store_session_summary(record, 0)
    database.invalidate_organization_proposals(
        tool, session_id, record["fingerprint"], 0,
    )
    return record


def _target(record: dict, suggested: dict) -> dict:
    return {
        "tool": record["tool"],
        "id": record["id"],
        "fingerprint": record["fingerprint"],
        "sources": [record["segments"][0]["hash"]],
        "suggested": suggested,
    }


def _ports():
    return create_ports()


def _digest_context(targets: list[dict]) -> dict:
    return organizing.digest_context(targets, _ports())


def _propose(targets: list[dict]) -> dict:
    return organizing.propose(targets, _ports())


def _list_proposals(status: str | None = None) -> list[dict]:
    return organizing.list_proposals(status, _ports())


def _modify(proposal_id: str, changes: list[dict]) -> dict:
    return organizing.modify(proposal_id, changes, _ports())


def _decide(proposal_id: str, decision: str) -> dict:
    return organizing.decide(proposal_id, decision, _ports())


def _set_metadata(tool: str, session_id: str, patch: dict) -> dict:
    return session_meta.set_entry(tool, session_id, patch, _ports())


def _signals() -> list[dict]:
    return organizing._database(_ports()).list_organization_signals()


def test_digest_context_only_exposes_cached_digest(organization_environment):
    record = _seed("claude", "session-a", "修复支付回调并补测试")

    context = _digest_context([{
        "tool": "claude", "id": "session-a",
    }])

    assert context == {"sessions": [{
        "tool": "claude",
        "id": "session-a",
        "fingerprint": record["fingerprint"],
        "pending": [],
        "segments": [{
            "hash": record["segments"][0]["hash"],
            "digest": "修复支付回调并补测试",
            "anchor_locator": "message-session-a",
            "turn": 1,
        }],
    }]}


def test_proposal_caches_by_content_fingerprint_and_has_sources(
        organization_environment):
    record = _seed("codex", "session-a", "实现支付重试")
    target = _target(record, {
        "name": "支付重试",
        "summary": "实现并验证支付重试逻辑",
        "tags": ["支付", "可靠性"],
        "dead_candidate": False,
    })

    first = _propose([target])
    second = _propose([{
        **target, "suggested": {"name": "不同的重复生成结果"},
    }])

    assert first["cache_hit"] is False
    assert second["cache_hit"] is True
    assert second["proposal_id"] == first["proposal_id"]
    assert first["targets"][0]["sources"][0] == {
        "segment_hash": record["segments"][0]["hash"],
        "anchor_locator": "message-session-a",
        "digest": "实现支付重试",
    }


def test_reject_records_signal_without_changing_metadata(
        organization_environment):
    record = _seed("claude", "session-a", "一次性探索")
    _set_metadata("claude", "session-a", {"name": "原名"})
    proposal = _propose([
        _target(record, {"name": "建议名", "dead_candidate": True}),
    ])

    result = _decide(proposal["proposal_id"], "reject")

    assert result["status"] == "rejected"
    assert session_meta.list_all(_ports())["claude\0session-a"] == {"name": "原名"}
    assert _signals()[-1]["event"] == "rejected"


def test_modify_then_approve_writes_only_local_metadata(
        organization_environment):
    original = organization_environment / "external-session.jsonl"
    original.write_text('{"original": true}\n')
    before = original.read_bytes()
    record = _seed("claude", "session-a", "完成登录流程")
    proposal = _propose([_target(record, {
        "name": "登录",
        "summary": "完成登录流程",
        "tags": ["认证"],
    })])

    changed = _modify(proposal["proposal_id"], [{
        "tool": "claude", "id": "session-a",
        "suggested": {
            "name": "登录与认证",
            "summary": "完成登录和认证流程",
            "tags": ["认证", "前端"],
            "dead_candidate": False,
        },
    }])
    assert session_meta.list_all(_ports()) == {}
    result = _decide(changed["proposal_id"], "approve")

    assert result["status"] == "approved"
    assert session_meta.list_all(_ports())["claude\0session-a"] == {
        "name": "登录与认证",
        "summary": "完成登录和认证流程",
        "tags": ["认证", "前端"],
    }
    assert original.read_bytes() == before
    assert [signal["event"] for signal in _signals()] == [
        "modified", "accepted",
    ]
    assert _signals()[-1]["modified"] is True


def test_cross_agent_cluster_is_approved_atomically(
        organization_environment):
    claude = _seed("claude", "session-claude", "设计支付接口")
    codex = _seed("codex", "session-codex", "实现支付接口")
    proposal = _propose([
        _target(claude, {
            "cluster_id": "project:payments",
            "cluster_name": "支付项目",
            "tags": ["支付"],
            "dead_candidate": False,
        }),
        _target(codex, {
            "cluster_id": "project:payments",
            "cluster_name": "支付项目",
            "tags": ["支付"],
            "dead_candidate": False,
        }),
    ])

    result = _decide(proposal["proposal_id"], "approve")

    assert set(result["applied"]) == {
        "claude\0session-claude", "codex\0session-codex",
    }
    metadata = session_meta.list_all(_ports())
    assert metadata["claude\0session-claude"]["cluster_id"] == "project:payments"
    assert metadata["codex\0session-codex"]["cluster_id"] == "project:payments"
    assert metadata["claude\0session-claude"]["cluster_name"] == "支付项目"


def test_same_native_id_from_different_tools_keeps_metadata_and_cas_isolated(
        organization_environment):
    claude = _seed("claude", "shared-id", "分析需求")
    codex = _seed("codex", "shared-id", "实现需求")
    _set_metadata("claude", "shared-id", {"name": "Claude 原名"})
    _set_metadata("codex", "shared-id", {"name": "Codex 原名"})

    proposal = _propose([
        _target(claude, {"name": "Claude 新名"}),
        _target(codex, {"name": "Codex 新名"}),
    ])

    assert [target["current"] for target in proposal["targets"]] == [
        {"name": "Claude 原名"},
        {"name": "Codex 原名"},
    ]
    _decide(proposal["proposal_id"], "approve")
    assert session_meta.list_all(_ports()) == {
        "claude\0shared-id": {"name": "Claude 新名"},
        "codex\0shared-id": {"name": "Codex 新名"},
    }


def test_changed_fingerprint_invalidates_and_can_regenerate(
        organization_environment):
    first_record = _seed("opencode", "session-a", "初始工作")
    old = _propose([
        _target(first_record, {"name": "初始工作"}),
    ])
    new_record = _seed(
        "opencode", "session-a", "续写后的工作", fingerprint="sha256:new")

    assert _list_proposals()[0]["status"] == "stale"
    fresh = _propose([
        _target(new_record, {"name": "续写后的工作"}),
    ])

    assert fresh["proposal_id"] != old["proposal_id"]
    assert fresh["cache_hit"] is False
    with pytest.raises(OrganizationProposalStaleError):
        _decide(old["proposal_id"], "approve")


def test_approval_detects_stale_content_without_metadata_pollution(
        organization_environment):
    record = _seed("claude", "session-a", "旧摘要")
    proposal = _propose([
        _target(record, {"name": "旧建议"}),
    ])
    _seed("claude", "session-a", "新摘要", fingerprint="sha256:changed")

    with pytest.raises(OrganizationProposalStaleError):
        _decide(proposal["proposal_id"], "approve")

    assert session_meta.list_all(_ports()) == {}
    assert _list_proposals()[0]["status"] == "stale"


def test_metadata_cas_failure_does_not_partially_apply_cluster(
        organization_environment):
    first = _seed("claude", "session-a", "A")
    second = _seed("codex", "session-b", "B")
    _set_metadata("codex", "session-b", {"name": "before"})
    proposal = _propose([
        _target(first, {"cluster_id": "cluster:x"}),
        _target(second, {"cluster_id": "cluster:x"}),
    ])
    _set_metadata("codex", "session-b", {"name": "concurrent"})

    with pytest.raises(Exception):
        _decide(proposal["proposal_id"], "approve")

    assert "claude\0session-a" not in session_meta.list_all(_ports())
    assert session_meta.list_all(_ports())["codex\0session-b"] == {"name": "concurrent"}
    assert _list_proposals()[0]["status"] == "stale"
    assert _signals()[-1]["reason"] == "metadata_changed"


def test_invalid_source_is_retryable_without_persisting_failure(
        organization_environment):
    record = _seed("claude", "session-a", "摘要")
    target = _target(record, {"name": "有效建议"})
    target["sources"] = ["sha256:not-current"]

    with pytest.raises(OrganizationProposalError):
        _propose([target])
    assert _list_proposals() == []

    target["sources"] = [record["segments"][0]["hash"]]
    assert _propose([target])["status"] == "pending"


def test_incomplete_digest_blocks_proposal_but_reports_pending(
        organization_environment):
    record = _seed("claude", "session-a", "摘要")
    record["segments"][0]["digest"] = None
    summaries._database(_ports()).store_session_summary(record, 0)
    context = _digest_context([
        {"tool": "claude", "id": "session-a"},
    ])
    assert context["sessions"][0]["pending"] == [
        record["segments"][0]["hash"],
    ]

    with pytest.raises(OrganizationProposalError):
        _propose([
            _target(record, {"name": "不完整建议"}),
        ])
    assert _list_proposals() == []


def test_rpc_exposes_context_proposal_list_and_decision(
        organization_environment):
    record = _seed("claude", "session-a", "整理 RPC")
    context = rpc(json.dumps({
        "protocol": PROTOCOL,
        "id": "organization-context",
        "method": "organization_digest_context",
        "params": {"targets": [{"tool": "claude", "id": "session-a"}]},
    }))
    assert context["ok"] is True
    assert context["result"]["sessions"][0]["segments"][0]["digest"] == "整理 RPC"

    proposed = rpc(json.dumps({
        "protocol": PROTOCOL,
        "id": "organization-propose",
        "method": "organization_propose",
        "params": {"targets": [_target(record, {
            "name": "RPC 整理", "dead_candidate": True,
        })]},
    }))
    proposal_id = proposed["result"]["proposal_id"]
    listed = rpc(json.dumps({
        "protocol": PROTOCOL,
        "id": "organization-list",
        "method": "organization_proposals_list",
        "params": {"status": "pending"},
    }))
    assert [item["proposal_id"] for item in listed["result"]] == [proposal_id]

    rejected = rpc(json.dumps({
        "protocol": PROTOCOL,
        "id": "organization-decide",
        "method": "organization_proposal_decide",
        "params": {"proposal_id": proposal_id, "decision": "reject"},
    }))
    assert rejected["result"]["status"] == "rejected"
