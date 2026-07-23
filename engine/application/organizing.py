"""AI-native 整理壳。

LLM 只负责从 T2 digest 生成建议；本模块负责引用/指纹校验、提案审批、
本地元数据写入与行为信号。任何路径都不会打开或改写 Agent 原始会话。
"""
from __future__ import annotations

import hashlib
import json
import os
import secrets
import tempfile
import threading
import time
from pathlib import Path

from ..domain.errors import (
    ConcurrentModificationError,
    OrganizationProposalError,
    OrganizationProposalNotFoundError,
    OrganizationProposalStaleError,
)
from . import services, session_meta, summaries

PROPOSALS = Path.home() / ".resume-harness" / "organization-proposals.json"
SIGNALS = Path.home() / ".resume-harness" / "organization-signals.jsonl"
_LOCK = threading.RLock()
_PATCH_FIELDS = {
    "name", "summary", "tags", "cluster_id", "cluster_name",
    "dead_candidate", "dead_reason", "archived",
}
_FINAL_STATES = {"approved", "rejected", "stale"}


def _now_ms() -> int:
    return int(time.time() * 1000)


def _canonical(value) -> str:
    return json.dumps(value, ensure_ascii=False, sort_keys=True,
                      separators=(",", ":"), allow_nan=False)


def _digest(value) -> str:
    return hashlib.sha256(_canonical(value).encode()).hexdigest()


def _load(path: Path | None = None) -> dict:
    path = path or PROPOSALS
    try:
        data = json.loads(path.read_text())
        return data if isinstance(data, dict) else {}
    except (OSError, json.JSONDecodeError):
        return {}


def _write(data: dict) -> None:
    PROPOSALS.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(
        prefix="organization-proposals-", suffix=".tmp",
        dir=PROPOSALS.parent)
    try:
        with os.fdopen(fd, "w") as stream:
            json.dump(data, stream, ensure_ascii=False, indent=1)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, PROPOSALS)
        os.chmod(PROPOSALS, 0o600)
    finally:
        try:
            os.unlink(temporary)
        except OSError:
            pass


def _signal(event: str, proposal: dict, **extra) -> None:
    SIGNALS.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "event": event,
        "proposal_id": proposal["proposal_id"],
        "generation_key": proposal["generation_key"],
        "target_count": len(proposal["targets"]),
        "at": _now_ms(),
        **extra,
    }
    fd = os.open(SIGNALS, os.O_APPEND | os.O_CREAT | os.O_WRONLY, 0o600)
    with os.fdopen(fd, "ab") as stream:
        stream.write((_canonical(payload) + "\n").encode())
        stream.flush()
        os.fsync(stream.fileno())


def _backbone(tool: str, session_id: str) -> dict:
    record = summaries._load().get(f"{tool}:{session_id}")
    if not record:
        raise OrganizationProposalError(
            "会话尚无摘要底座", {"tool": tool, "id": session_id})
    return record


def digest_context(targets: list[dict]) -> dict:
    """返回 runtime 生成标题/标签/聚类所需的最小、无原文 digest 语料。"""
    if not isinstance(targets, list) or not targets:
        raise OrganizationProposalError("targets 必须是非空数组")
    result = []
    for target in targets:
        tool, session_id = target.get("tool"), target.get("id")
        if not isinstance(tool, str) or not isinstance(session_id, str):
            raise OrganizationProposalError("target 缺少 tool/id")
        record = _backbone(tool, session_id)
        invalidate_session(tool, session_id, record["fingerprint"])
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


def _validated_target(target: dict, metadata: dict) -> dict:
    tool, session_id = target.get("tool"), target.get("id")
    fingerprint = target.get("fingerprint")
    if not all(isinstance(value, str) and value
               for value in (tool, session_id, fingerprint)):
        raise OrganizationProposalError("target 缺少 tool/id/fingerprint")
    record = _backbone(tool, session_id)
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
        "current": metadata.get(session_meta.key(tool, session_id), {}),
        "suggested": suggested,
        "sources": [{
            "segment_hash": source,
            "anchor_locator": available[source].get("anchor_locator"),
            "digest": available[source]["digest"],
        } for source in sources],
    }


def propose(targets: list[dict]) -> dict:
    """接收 runtime 的结构化整理结果；同一内容指纹只产生一个提案。"""
    if not isinstance(targets, list) or not targets:
        raise OrganizationProposalError("targets 必须是非空数组")
    metadata = services.session_meta_list()
    normalized = [_validated_target(target, metadata) for target in targets]
    identities = [(target["tool"], target["id"]) for target in normalized]
    if len(set(identities)) != len(identities):
        raise OrganizationProposalError("targets 不得重复")
    generation_key = _digest([
        [target["tool"], target["id"], target["fingerprint"]]
        for target in normalized
    ])
    with _LOCK:
        data = _load()
        existing = next((
            item for item in data.values()
            if item.get("generation_key") == generation_key
            and item.get("status") != "stale"
        ), None)
        if existing:
            return {**existing, "cache_hit": True}
        now = _now_ms()
        proposal = {
            "proposal_id": "org_" + secrets.token_urlsafe(18),
            "generation_key": generation_key,
            "status": "pending",
            "targets": normalized,
            "created_at": now,
            "updated_at": now,
        }
        data[proposal["proposal_id"]] = proposal
        _write(data)
        return {**proposal, "cache_hit": False}


def list_proposals(status: str | None = None) -> list[dict]:
    with _LOCK:
        values = list(_load().values())
    if status is not None:
        values = [item for item in values if item.get("status") == status]
    return sorted(values, key=lambda item: item.get("created_at", 0),
                  reverse=True)


def _get_pending(data: dict, proposal_id: str) -> dict:
    proposal = data.get(proposal_id)
    if proposal is None:
        raise OrganizationProposalNotFoundError(
            "整理提案不存在", {"proposal_id": proposal_id})
    if proposal.get("status") != "pending":
        raise OrganizationProposalError(
            "整理提案已处理", {"proposal_id": proposal_id,
                            "status": proposal.get("status")})
    return proposal


def modify(proposal_id: str, changes: list[dict]) -> dict:
    """用户可在批准前改写建议值；修改本身不会落入会话元数据。"""
    if not isinstance(changes, list) or not changes:
        raise OrganizationProposalError("changes 必须是非空数组")
    by_identity = {}
    for change in changes:
        identity = (change.get("tool"), change.get("id"))
        by_identity[identity] = _validated_patch(change.get("suggested"))
    with _LOCK:
        data = _load()
        proposal = _get_pending(data, proposal_id)
        known = {(target["tool"], target["id"]) for target in proposal["targets"]}
        if not set(by_identity) <= known:
            raise OrganizationProposalError("changes 包含未知 target")
        for target in proposal["targets"]:
            patch = by_identity.get((target["tool"], target["id"]))
            if patch is not None:
                target["suggested"] = patch
        proposal["updated_at"] = _now_ms()
        proposal["modified"] = True
        _write(data)
        _signal("modified", proposal)
        return proposal


def decide(proposal_id: str, decision: str) -> dict:
    """批准才批量写 sidecar；拒绝仅改变提案状态并记录信号。"""
    if decision not in {"approve", "reject"}:
        raise OrganizationProposalError("decision 必须是 approve/reject")
    with _LOCK:
        data = _load()
        proposal = _get_pending(data, proposal_id)
        if decision == "reject":
            proposal["status"] = "rejected"
            proposal["updated_at"] = _now_ms()
            _write(data)
            _signal("rejected", proposal)
            return proposal
        for target in proposal["targets"]:
            record = _backbone(target["tool"], target["id"])
            if record["fingerprint"] != target["fingerprint"]:
                proposal["status"] = "stale"
                proposal["updated_at"] = _now_ms()
                _write(data)
                _signal("stale", proposal)
                raise OrganizationProposalStaleError(
                    "摘要内容已变化，请重新生成整理建议",
                    {"tool": target["tool"], "id": target["id"]})
        try:
            applied = services.session_meta_compare_and_set_many([{
                "tool": target["tool"],
                "id": target["id"],
                "expected": target["current"],
                "patch": target["suggested"],
            } for target in proposal["targets"]])
        except ConcurrentModificationError:
            proposal["status"] = "stale"
            proposal["updated_at"] = _now_ms()
            _write(data)
            _signal("stale", proposal, reason="metadata_changed")
            raise
        proposal["status"] = "approved"
        proposal["updated_at"] = _now_ms()
        proposal["applied"] = applied
        _write(data)
        _signal("accepted", proposal,
                modified=bool(proposal.get("modified")))
        return proposal


def invalidate_session(tool: str, session_id: str, fingerprint: str) -> int:
    """摘要重建时把旧内容的待处理提案标 stale，允许新指纹重新生成。"""
    changed = 0
    with _LOCK:
        data = _load()
        for proposal in data.values():
            if proposal.get("status") in _FINAL_STATES:
                continue
            if any(target["tool"] == tool and target["id"] == session_id
                   and target["fingerprint"] != fingerprint
                   for target in proposal.get("targets", [])):
                proposal["status"] = "stale"
                proposal["updated_at"] = _now_ms()
                _signal("stale", proposal)
                changed += 1
        if changed:
            _write(data)
    return changed
