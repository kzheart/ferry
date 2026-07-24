"""待迁移的迁移预览入口。"""
from __future__ import annotations

from ..errors import AgentRequestError
from .agent_read import read_indexed_session
from .index import AgentSessionIndex
from .safety import (
    bounded_int,
    bounded_json,
    finalize_dto,
    record_session_id,
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
