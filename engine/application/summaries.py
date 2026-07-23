"""摘要底座:把规范会话切成'段'(一个用户诉求 + 回应它的那段工作),
按内容 hash 缓存,存进本地元数据层(不碰原始会话文件)。

分工:engine 负责分段 / 内容指纹 / 缓存失效 / 存储 / RPC;真正的一句蒸馏摘要
(digest)由常驻、持有 LLM 栈的 agent-runtime 生成后经 set_summaries 写回。
召回 / 整理 / 记忆文件三个上层功能共用这一份底座。
"""

import hashlib
import time
from pathlib import Path

from ..domain.errors import SummaryBackboneMissingError
from ..infrastructure.state_db import StateDatabase
from .ports import current
from .sessions import read_tree

MAX_DIGEST_CHARS = 4000


def _database() -> StateDatabase:
    return StateDatabase(
        Path(current().snapshot_dir()) / "ferry-state.sqlite3",
        recover_interrupted=False,
    )


def _now_ms() -> int:
    return int(time.time() * 1000)


def get_backbone(tool: str, session_id: str) -> dict | None:
    return _database().get_session_summary(tool, session_id)


def _locator(message, index: int) -> str:
    if isinstance(message.source_id, str) and message.source_id:
        return message.source_id
    return f"index:{index}"


def _message_text(message) -> str:
    return "\n".join(block.text for block in message.blocks
                     if block.kind == "text" and getattr(block, "text", ""))


def segment_session(session) -> list[dict]:
    """按轮次切段:每条 user 消息开一个新段,并入其后到下一条 user 之间的
    assistant 文本。只取可见文本(跳过工具输出等噪声)。上下文压缩边界作为
    段的附加标记(after_compaction)。"""
    internal = {compaction.summary_message_id
                for compaction in session.context_compactions
                if compaction.summary_message_id}
    messages = [message for message in session.messages
                if message.source_id not in internal]

    turn = 0
    turn_by_locator: dict[str, int] = {}
    for message in messages:
        if message.role == "user":
            turn += 1
        if message.source_id:
            turn_by_locator[message.source_id] = turn
    compacted_after_turns = {
        turn_by_locator.get(compaction.after_message_id, 0)
        for compaction in session.context_compactions
    }

    segments: list[dict] = []
    turn = 0
    current: dict | None = None
    for index, message in enumerate(messages):
        if message.role == "user":
            turn += 1
            if current is not None:
                segments.append(current)
            current = {
                "turn": turn,
                "anchor_locator": _locator(message, index),
                "message_start": index,
                "message_end": index,
                "after_compaction": turn > 1 and (turn - 1) in compacted_after_turns,
                "_texts": [_message_text(message)],
            }
        elif current is not None:
            text = _message_text(message)
            if text:
                current["_texts"].append(text)
            current["message_end"] = index
    if current is not None:
        segments.append(current)

    result = []
    for segment in segments:
        text = "\n".join(part for part in segment.pop("_texts") if part).strip()
        segment["hash"] = "sha256:" + hashlib.sha256(text.encode("utf-8")).hexdigest()
        segment["char_count"] = len(text)
        segment["digest"] = None
        segment["_source_text"] = text
        result.append(segment)
    return result


def session_fingerprint(segments: list[dict]) -> str:
    """会话级内容指纹:段数 + 各段内容 hash 的有序拼接。会话被改 / 续写时
    指纹变化,借此决定是否重算摘要底座。"""
    joined = "\n".join(segment["hash"] for segment in segments)
    payload = f"{len(segments)}:{joined}".encode("utf-8")
    return "sha256:" + hashlib.sha256(payload).hexdigest()


def _view(record: dict) -> dict:
    pending = [segment["hash"] for segment in record["segments"]
               if not segment.get("digest")]
    return {
        "tool": record["tool"],
        "id": record["id"],
        "fingerprint": record["fingerprint"],
        "segment_count": len(record["segments"]),
        "pending": pending,
        "segments": record["segments"],
    }


def build_backbone(tool: str, ref: str) -> dict:
    """读取会话 → 分段 → 算指纹。每次都以当前会话重建段结构,并按内容
    hash 复用既有摘要(编辑结构或某段时不牵连未变内容的摘要)。"""
    session = read_tree(tool, ref)
    segments = segment_session(session)
    fingerprint = session_fingerprint(segments)
    source_by_hash = {
        segment["hash"]: segment.pop("_source_text", "")
        for segment in segments
    }
    database = _database()
    previous = database.get_session_summary(tool, session.source_id)
    prior = {
        segment["hash"]: segment["digest"]
        for segment in (previous or {}).get("segments", [])
        if segment.get("digest")
    }
    for segment in segments:
        if segment["hash"] in prior:
            segment["digest"] = prior[segment["hash"]]
    record = {
        "tool": tool,
        "id": session.source_id,
        "fingerprint": fingerprint,
        "segments": segments,
    }
    if previous != record:
        now = _now_ms()
        database.store_session_summary(record, now)
        database.invalidate_organization_proposals(
            tool, session.source_id, fingerprint, now,
        )
    view = _view(record)
    view["pending_sources"] = [
        {"hash": segment["hash"], "text": source_by_hash[segment["hash"]]}
        for segment in record["segments"]
        if not segment.get("digest") and source_by_hash.get(segment["hash"])
    ]
    return view


def set_summaries(tool: str, session_id: str, digests: dict) -> dict:
    """agent-runtime 生成蒸馏摘要后按段内容 hash 写回。以 hash 为键,对
    编辑后仍存在的段稳健。"""
    updates = digests if isinstance(digests, dict) else {}
    database = _database()
    record = database.get_session_summary(tool, session_id)
    if not record:
        raise SummaryBackboneMissingError(
            "请先调用 session_backbone 建立底座",
            {"tool": tool, "id": session_id})
    applied = 0
    for segment in record["segments"]:
        digest = updates.get(segment["hash"])
        if isinstance(digest, str) and digest.strip():
            segment["digest"] = digest.strip()[:MAX_DIGEST_CHARS]
            applied += 1
    database.store_session_summary(record, _now_ms())
    return {**_view(record), "applied": applied}
