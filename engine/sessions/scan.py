"""跨工具会话扫描。"""

from ..context import EngineContext
from .index import AgentSessionIndex
from .scan_progress import TRACKER


def scan(ports: EngineContext, index: AgentSessionIndex) -> dict:
    tools, scanned = {}, []
    cache = ports.cache_factory()
    names = list(ports.adapters())
    TRACKER.begin(names)
    try:
        for name in names:
            tool = ports.adapter(name)
            source_path = tool.manifest.source_path
            TRACKER.start_tool(name)
            try:
                rows = tool.browser.scan(cache)
                scanned.extend((name, tool, row) for row in rows)
                tools[name] = {"ok": True, "count": len(rows), "path": source_path}
            except Exception as error:
                tools[name] = {"ok": False, "error": str(error)[:200], "path": source_path}
            finally:
                TRACKER.finish_tool(name)
        TRACKER.finalize()
        cache.flush()
        sessions = [
            {**record.row, "ref": record.opaque_ref, "revision": record.revision}
            for record in index.index_rows(scanned)
        ]
        sessions.sort(key=lambda session: session["updated"], reverse=True)
        return {"tools": tools, "sessions": sessions}
    finally:
        TRACKER.end()


def scan_progress() -> dict:
    return TRACKER.snapshot()
