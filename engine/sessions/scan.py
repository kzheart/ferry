"""跨工具会话扫描。

活索引就绪后 scan 直接返回内存快照(毫秒级),同时 nudge 活索引在后台
校准;增量经 sessions.changed 事件推给前端。只有首次(快照尚未建立)
才阻塞在全量扫描上,且与启动预热单飞合并。
"""

from ..context import EngineContext
from .index import AgentSessionIndex, session_dto
from .scan_progress import TRACKER


def scan(
    _ports: EngineContext,
    index: AgentSessionIndex,
    live=None,
) -> dict:
    snapshot = index.snapshot_with_status()
    if snapshot is None:
        tools, records = index.refresh_with_status()
        generation = index.generation
    else:
        tools, records, generation = snapshot
        if live is not None:
            live.nudge()
    sessions = [session_dto(record) for record in records]
    sessions.sort(key=lambda session: session["updated"], reverse=True)
    return {"tools": tools, "sessions": sessions, "generation": generation}


def scan_progress() -> dict:
    return TRACKER.snapshot()
