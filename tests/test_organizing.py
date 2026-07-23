"""T4 整理壳：摘要引用、提案审批、跨 Agent 聚类与行为信号。"""
from __future__ import annotations

import json
from pathlib import Path

import pytest

from engine.application import organizing, services, session_meta, summaries
from engine.domain.errors import (
    OrganizationProposalError,
    OrganizationProposalStaleError,
)
from engine.interfaces.rpc import rpc


@pytest.fixture
def organization_environment(tmp_path, monkeypatch):
    monkeypatch.setattr(summaries, "SUMMARIES", tmp_path / "summaries.json")
    monkeypatch.setattr(organizing, "PROPOSALS", tmp_path / "proposals.json")
    monkeypatch.setattr(organizing, "SIGNALS", tmp_path / "signals.jsonl")
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
    data = summaries._load()
    data[f"{tool}:{session_id}"] = record
    summaries._write(data)
    return record


def _target(record: dict, suggested: dict) -> dict:
    return {
        "tool": record["tool"],
        "id": record["id"],
        "fingerprint": record["fingerprint"],
        "sources": [record["segments"][0]["hash"]],
        "suggested": suggested,
    }


def _signals() -> list[dict]:
    return [
        json.loads(line)
        for line in organizing.SIGNALS.read_text().splitlines()
    ]


def test_digest_context_only_exposes_cached_digest(organization_environment):
    record = _seed("claude", "session-a", "修复支付回调并补测试")

    context = organizing.digest_context([{
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

    first = organizing.propose([target])
    second = organizing.propose([{
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
    assert organizing.PROPOSALS.stat().st_mode & 0o777 == 0o600


def test_reject_records_signal_without_changing_metadata(
        organization_environment):
    record = _seed("claude", "session-a", "一次性探索")
    session_meta.set_entry("session-a", {"name": "原名"})
    proposal = organizing.propose([
        _target(record, {"name": "建议名", "dead_candidate": True}),
    ])

    result = organizing.decide(proposal["proposal_id"], "reject")

    assert result["status"] == "rejected"
    assert services.session_meta_list()["session-a"] == {"name": "原名"}
    assert _signals()[-1]["event"] == "rejected"
    assert organizing.SIGNALS.stat().st_mode & 0o777 == 0o600


def test_modify_then_approve_writes_only_local_metadata(
        organization_environment):
    original = organization_environment / "external-session.jsonl"
    original.write_text('{"original": true}\n')
    before = original.read_bytes()
    record = _seed("claude", "session-a", "完成登录流程")
    proposal = organizing.propose([_target(record, {
        "name": "登录",
        "summary": "完成登录流程",
        "tags": ["认证"],
    })])

    changed = organizing.modify(proposal["proposal_id"], [{
        "tool": "claude", "id": "session-a",
        "suggested": {
            "name": "登录与认证",
            "summary": "完成登录和认证流程",
            "tags": ["认证", "前端"],
            "dead_candidate": False,
        },
    }])
    assert services.session_meta_list() == {}
    result = organizing.decide(changed["proposal_id"], "approve")

    assert result["status"] == "approved"
    assert services.session_meta_list()["session-a"] == {
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
    proposal = organizing.propose([
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

    result = organizing.decide(proposal["proposal_id"], "approve")

    assert set(result["applied"]) == {"session-claude", "session-codex"}
    metadata = services.session_meta_list()
    assert metadata["session-claude"]["cluster_id"] == "project:payments"
    assert metadata["session-codex"]["cluster_id"] == "project:payments"
    assert metadata["session-claude"]["cluster_name"] == "支付项目"


def test_changed_fingerprint_invalidates_and_can_regenerate(
        organization_environment):
    first_record = _seed("opencode", "session-a", "初始工作")
    old = organizing.propose([
        _target(first_record, {"name": "初始工作"}),
    ])
    new_record = _seed(
        "opencode", "session-a", "续写后的工作", fingerprint="sha256:new")

    organizing.digest_context([{"tool": "opencode", "id": "session-a"}])
    assert organizing.list_proposals()[0]["status"] == "stale"
    fresh = organizing.propose([
        _target(new_record, {"name": "续写后的工作"}),
    ])

    assert fresh["proposal_id"] != old["proposal_id"]
    assert fresh["cache_hit"] is False
    with pytest.raises(OrganizationProposalError):
        organizing.decide(old["proposal_id"], "approve")


def test_approval_detects_stale_content_without_metadata_pollution(
        organization_environment):
    record = _seed("claude", "session-a", "旧摘要")
    proposal = organizing.propose([
        _target(record, {"name": "旧建议"}),
    ])
    _seed("claude", "session-a", "新摘要", fingerprint="sha256:changed")

    with pytest.raises(OrganizationProposalStaleError):
        organizing.decide(proposal["proposal_id"], "approve")

    assert services.session_meta_list() == {}
    assert organizing.list_proposals()[0]["status"] == "stale"


def test_metadata_cas_failure_does_not_partially_apply_cluster(
        organization_environment):
    first = _seed("claude", "session-a", "A")
    second = _seed("codex", "session-b", "B")
    session_meta.set_entry("session-b", {"name": "before"})
    proposal = organizing.propose([
        _target(first, {"cluster_id": "cluster:x"}),
        _target(second, {"cluster_id": "cluster:x"}),
    ])
    session_meta.set_entry("session-b", {"name": "concurrent"})

    with pytest.raises(Exception):
        organizing.decide(proposal["proposal_id"], "approve")

    assert "session-a" not in services.session_meta_list()
    assert services.session_meta_list()["session-b"] == {"name": "concurrent"}


def test_invalid_source_is_retryable_without_persisting_failure(
        organization_environment):
    record = _seed("claude", "session-a", "摘要")
    target = _target(record, {"name": "有效建议"})
    target["sources"] = ["sha256:not-current"]

    with pytest.raises(OrganizationProposalError):
        organizing.propose([target])
    assert not organizing.PROPOSALS.exists()

    target["sources"] = [record["segments"][0]["hash"]]
    assert organizing.propose([target])["status"] == "pending"


def test_incomplete_digest_blocks_proposal_but_reports_pending(
        organization_environment):
    record = _seed("claude", "session-a", "摘要")
    record["segments"][0]["digest"] = None
    summaries._write({"claude:session-a": record})
    context = organizing.digest_context([
        {"tool": "claude", "id": "session-a"},
    ])
    assert context["sessions"][0]["pending"] == [
        record["segments"][0]["hash"],
    ]

    with pytest.raises(OrganizationProposalError):
        organizing.propose([
            _target(record, {"name": "不完整建议"}),
        ])
    assert not organizing.PROPOSALS.exists()


def test_rpc_exposes_context_proposal_list_and_decision(
        organization_environment):
    record = _seed("claude", "session-a", "整理 RPC")
    context = rpc(json.dumps({
        "method": "organization_digest_context",
        "params": {"targets": [{"tool": "claude", "id": "session-a"}]},
    }))
    assert context["ok"] is True
    assert context["result"]["sessions"][0]["segments"][0]["digest"] == "整理 RPC"

    proposed = rpc(json.dumps({
        "method": "organization_propose",
        "params": {"targets": [_target(record, {
            "name": "RPC 整理", "dead_candidate": True,
        })]},
    }))
    proposal_id = proposed["result"]["proposal_id"]
    listed = rpc(json.dumps({
        "method": "organization_proposals_list",
        "params": {"status": "pending"},
    }))
    assert [item["proposal_id"] for item in listed["result"]] == [proposal_id]

    rejected = rpc(json.dumps({
        "method": "organization_proposal_decide",
        "params": {"proposal_id": proposal_id, "decision": "reject"},
    }))
    assert rejected["result"]["status"] == "rejected"
