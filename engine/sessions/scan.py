"""跨工具会话扫描。

扫库、进度跟踪与索引重建都收敛在 AgentSessionIndex.refresh_with_status:
UI 扫描、启动预热、agent 搜索共用同一次单飞刷新,这里只做 UI 出参整形。
"""

from ..context import EngineContext
from .index import AgentSessionIndex
from .scan_progress import TRACKER


def scan(_ports: EngineContext, index: AgentSessionIndex) -> dict:
    tools, records = index.refresh_with_status()
    sessions = [
        {**record.row, "ref": record.opaque_ref, "revision": record.revision}
        for record in records
    ]
    sessions.sort(key=lambda session: session["updated"], reverse=True)
    return {"tools": tools, "sessions": sessions}


def scan_progress() -> dict:
    return TRACKER.snapshot()
