"""待迁移的操作预览入口；会话查询已拆入独立能力模块。"""
from __future__ import annotations

import json

from ..errors import AgentRequestError, LocatorStaleError
from .agent_read import read_indexed_session
from .index import AgentSessionIndex, IndexedSession
from .safety import (
    bounded_int,
    bounded_json,
    finalize_dto,
    record_session_id,
    redact,
    validate_agent_edit_ops,
)


def resolve_edit_ops(index: AgentSessionIndex, record: IndexedSession,
                     ops: list[dict]) -> list[dict]:
    resolved = []
    for op in ops:
        item = dict(op)
        if item.get("op") == "rewrite":
            message = index.resolve_message_locator(record, item["locator"])
            if not message.editable:
                raise AgentRequestError(
                    "目标消息不支持文本改写",
                    {
                        "field": "locator",
                        "locator": item["locator"],
                        "hint": "仅使用 editable=true 的消息引用",
                    },
                )
            item["locator"] = message.native_locator
        resolved.append(item)
    return resolved


def public_locator_error(ops: list[dict]) -> LocatorStaleError:
    locator = next((
        op.get("locator")
        for op in ops
        if op.get("op") == "rewrite"
    ), None)
    return LocatorStaleError(
        "消息定位信息与当前会话不匹配",
        {
            "field": "locator",
            "locator": locator,
            "hint": (
                "重新调用 ferry_get_session_context，"
                "并原样使用 messages[].locator"
            ),
        },
    )


def preview_migration(source_tool: str, opaque_ref: str, target_tool: str,
                      max_turn: int | None = None, *,
                      index: AgentSessionIndex) -> dict:
    record = index.resolve(source_tool, opaque_ref)
    if target_tool not in index.ports.adapters():
        raise AgentRequestError(
            "未知目标 Agent", {"target_tool": target_tool},
        )
    session = read_indexed_session(index, record)
    if max_turn is not None:
        max_turn = bounded_int(max_turn, 1, 1, 1_000_000, "max_turn")
        from ..operations.migrate import _truncate_rounds
        _truncate_rounds(session, max_turn)
    target = index.ports.adapter(target_tool).migration_target
    loss = target.plan(session)
    from ..operations.migrate import _migration_counts
    tree_count, message_count = _migration_counts(session)
    edge_count = sum(len(node.agent_edges) for node in session.walk())
    return finalize_dto({
        "source_tool": source_tool,
        "target_tool": target_tool,
        "ref": opaque_ref,
        "revision": record.revision,
        "source_session_id": record_session_id(record, session),
        "message_count": message_count,
        "root_message_count": len(session.messages),
        "tree_count": tree_count,
        "child_count": tree_count - 1,
        "loss": bounded_json(loss),
        "topology": {
            "nodes": tree_count,
            "edges": max(0, tree_count - 1),
            "agent_edges": edge_count,
            "preserved": True,
        },
        "max_turn": max_turn,
    })


def preview_edit(tool: str, opaque_ref: str, *, ops,
                 index: AgentSessionIndex) -> dict:
    record = index.resolve(tool, opaque_ref)
    adapter = index.ports.adapter(tool)
    validate_agent_edit_ops(ops)
    if len(json.dumps(ops, ensure_ascii=False, default=str).encode()) \
            > 64 * 1024:
        raise AgentRequestError("ops 超过 64 KiB")
    from ..operations.edit import preview
    editor = adapter.editor
    native_ops = resolve_edit_ops(index, record, ops)
    try:
        result = preview(
            editor,
            record.canonical_ref,
            native_ops,
            loader=getattr(editor, "load_preview", None),
        )
    except LocatorStaleError as error:
        raise public_locator_error(ops) from error
    index.resolve(tool, opaque_ref)
    return finalize_dto({
        "tool": tool,
        "ref": opaque_ref,
        "mode": "edit",
        "session_id": record_session_id(record),
        "revision": redact(str(result["revision"]), 256),
        "before": bounded_json(result["before"], 12 * 1024),
        "after": bounded_json(result["after"], 12 * 1024),
        "changes": bounded_json(result["changes"], 12 * 1024),
    })
