"""AI-native 整理壳。

LLM 只负责从 T2 digest 生成建议；本模块负责引用/指纹校验、提案审批、
本地元数据写入与行为信号。任何路径都不会打开或改写 Agent 原始会话。
"""
from __future__ import annotations

import hashlib
import json
import secrets
import time
from pathlib import Path

from ..context import EngineContext
from ..errors import (
    ConcurrentModificationError,
    OrganizationProposalError,
    OrganizationProposalNotFoundError,
    OrganizationProposalStaleError,
)
from ..storage.database import StateDatabase
from ..operations import metadata as metadata_store
from . import summaries

_PATCH_FIELDS = {
    "name", "summary", "tags", "cluster_id", "cluster_name",
    "dead_candidate", "dead_reason", "archived",
}


def _database(ports: EngineContext) -> StateDatabase:
    return StateDatabase(
        Path(ports.snapshot_dir()) / "ferry-state.sqlite3",
        recover_interrupted=False,
    )


def _now_ms() -> int:
    return int(time.time() * 1000)


def _canonical(value) -> str:
    return json.dumps(value, ensure_ascii=False, sort_keys=True,
                      separators=(",", ":"), allow_nan=False)


def _digest(value) -> str:
    return hashlib.sha256(_canonical(value).encode()).hexdigest()


def _backbone(tool: str, session_id: str, ports: EngineContext) -> dict:
    record = summaries.get_backbone(tool, session_id, ports)
    if not record:
        raise OrganizationProposalError(
            "会话尚无摘要底座", {"tool": tool, "id": session_id})
    return record


def digest_context(targets: list[dict], ports: EngineContext) -> dict:
    """返回 runtime 生成标题/标签/聚类所需的最小、无原文 digest 语料。"""
    if not isinstance(targets, list) or not targets:
        raise OrganizationProposalError("targets 必须是非空数组")
    result = []
    for target in targets:
        tool, session_id = target.get("tool"), target.get("id")
        if not isinstance(tool, str) or not isinstance(session_id, str):
            raise OrganizationProposalError("target 缺少 tool/id")
        record = _backbone(tool, session_id, ports)
        segments = [{
            "hash": item["hash"],
            "digest": item["digest"],
            "anchor_locator": item.get("anchor_locator"),
            "turn": item.get("turn"),
        } for item in record["segments"] if item.get("digest")]
        result.append({
            "tool": tool, "id": session_id,
            "fingerprint": record["fingerprint"],
            "pending": [
                item["hash"] for item in record["segments"]
                if not item.get("digest")
            ],
            "segments": segments,
        })
    return {"sessions": result}


def _validated_patch(patch) -> dict:
    if not isinstance(patch, dict) or not patch or not set(patch) <= _PATCH_FIELDS:
        raise OrganizationProposalError("suggested 字段非法")
    result = dict(patch)
    for field in ("name", "summary", "cluster_id", "cluster_name",
                  "dead_reason"):
        if field in result and (
                not isinstance(result[field], str)
                or not result[field].strip()
                or len(result[field]) > (
                    4000 if field == "summary"
                    else 1000 if field == "dead_reason"
                    else 200)):
            raise OrganizationProposalError(f"{field} 非法")
        if field in result:
            result[field] = result[field].strip()
    if "tags" in result:
        tags = result["tags"]
        if (not isinstance(tags, list) or len(tags) > 20
                or not all(isinstance(tag, str) and 1 <= len(tag.strip()) <= 64
                           for tag in tags)):
            raise OrganizationProposalError("tags 非法")
        result["tags"] = list(dict.fromkeys(tag.strip() for tag in tags))
    for field in ("dead_candidate", "archived"):
        if field in result and not isinstance(result[field], bool):
            raise OrganizationProposalError(f"{field} 必须是 boolean")
    return result


def _validated_target(target: dict, current_metadata: dict,
                      ports: EngineContext) -> dict:
    tool, session_id = target.get("tool"), target.get("id")
    fingerprint = target.get("fingerprint")
    if not all(isinstance(value, str) and value
               for value in (tool, session_id, fingerprint)):
        raise OrganizationProposalError("target 缺少 tool/id/fingerprint")
    record = _backbone(tool, session_id, ports)
    if record["fingerprint"] != fingerprint:
        raise OrganizationProposalStaleError(
            "摘要内容已变化，请重新生成整理建议",
            {"tool": tool, "id": session_id})
    pending = [segment["hash"] for segment in record["segments"]
               if not segment.get("digest")]
    if pending:
        raise OrganizationProposalError(
            "摘要尚未生成完整，请先补齐 digest",
            {"tool": tool, "id": session_id, "pending": pending})
    available = {
        segment["hash"]: segment for segment in record["segments"]
        if segment.get("digest")
    }
    sources = target.get("sources")
    if sources is None:
        sources = list(available)
    if (not isinstance(sources, list) or not sources
            or not all(isinstance(source, str) and source in available
                       for source in sources)):
        raise OrganizationProposalError(
            "sources 必须引用当前摘要 hash", {"tool": tool, "id": session_id})
    suggested = _validated_patch(target.get("suggested"))
    return {
        "tool": tool,
        "id": session_id,
        "fingerprint": fingerprint,
        "current": current_metadata.get(metadata_store.key(tool, session_id), {}),
        "suggested": suggested,
        "sources": [{
            "segment_hash": source,
            "anchor_locator": available[source].get("anchor_locator"),
            "digest": available[source]["digest"],
        } for source in sources],
    }


def propose(targets: list[dict], ports: EngineContext) -> dict:
    """接收 runtime 的结构化整理结果；同一内容指纹只产生一个提案。"""
    if not isinstance(targets, list) or not targets:
        raise OrganizationProposalError("targets 必须是非空数组")
    current_metadata = metadata_store.list_all(ports)
    normalized = [
        _validated_target(target, current_metadata, ports)
        for target in targets
    ]
    identities = [(target["tool"], target["id"]) for target in normalized]
    if len(set(identities)) != len(identities):
        raise OrganizationProposalError("targets 不得重复")
    generation_key = _digest([
        [target["tool"], target["id"], target["fingerprint"]]
        for target in normalized
    ])
    now = _now_ms()
    result = _database(ports).create_or_get_organization_proposal({
        "proposal_id": "org_" + secrets.token_urlsafe(18),
        "generation_key": generation_key,
        "status": "pending",
        "targets": normalized,
        "created_at": now,
        "updated_at": now,
    })
    return {**result["proposal"], "cache_hit": result["cache_hit"]}


def list_proposals(status: str | None, ports: EngineContext) -> list[dict]:
    return _database(ports).list_organization_proposals(status)


def _get_pending(proposal: dict | None, proposal_id: str) -> dict:
    if proposal is None:
        raise OrganizationProposalNotFoundError(
            "整理提案不存在", {"proposal_id": proposal_id})
    if proposal.get("status") != "pending":
        raise OrganizationProposalError(
            "整理提案已处理", {"proposal_id": proposal_id,
                            "status": proposal.get("status")})
    return proposal


def modify(proposal_id: str, changes: list[dict], ports: EngineContext) -> dict:
    """用户可在批准前改写建议值；修改本身不会落入会话元数据。"""
    if not isinstance(changes, list) or not changes:
        raise OrganizationProposalError("changes 必须是非空数组")
    by_identity = {}
    for change in changes:
        identity = (change.get("tool"), change.get("id"))
        by_identity[identity] = _validated_patch(change.get("suggested"))
    database = _database(ports)
    proposal = _get_pending(
        database.get_organization_proposal(proposal_id), proposal_id,
    )
    known = {(target["tool"], target["id"]) for target in proposal["targets"]}
    if not set(by_identity) <= known:
        raise OrganizationProposalError("changes 包含未知 target")
    for target in proposal["targets"]:
        patch = by_identity.get((target["tool"], target["id"]))
        if patch is not None:
            target["suggested"] = patch
    result = database.modify_organization_proposal(proposal, _now_ms())
    if result["outcome"] == "missing":
        raise OrganizationProposalNotFoundError(
            "整理提案不存在", {"proposal_id": proposal_id})
    if result["outcome"] == "not-pending":
        raise OrganizationProposalError(
            "整理提案已处理", {
                "proposal_id": proposal_id,
                "status": result["proposal"]["status"],
            },
        )
    return result["proposal"]


def decide(proposal_id: str, decision: str, ports: EngineContext) -> dict:
    """批准才批量写 sidecar；拒绝仅改变提案状态并记录信号。"""
    if decision not in {"approve", "reject"}:
        raise OrganizationProposalError("decision 必须是 approve/reject")
    result = _database(ports).decide_organization_proposal(
        proposal_id, decision, _now_ms(),
    )
    if result["outcome"] == "missing":
        raise OrganizationProposalNotFoundError(
            "整理提案不存在", {"proposal_id": proposal_id})
    if result["outcome"] == "not-pending":
        if result["proposal"]["status"] == "stale":
            raise OrganizationProposalStaleError(
                "摘要内容已变化，请重新生成整理建议",
            )
        raise OrganizationProposalError(
            "整理提案已处理", {
                "proposal_id": proposal_id,
                "status": result["proposal"]["status"],
            },
        )
    if result["outcome"] == "stale-summary":
        raise OrganizationProposalStaleError(
            "摘要内容已变化，请重新生成整理建议", result["target"],
        )
    if result["outcome"] == "stale-metadata":
        raise ConcurrentModificationError("会话元数据在审批后已变化")
    return result["proposal"]
