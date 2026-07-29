"""跨工具会话扫描。"""

import logging
import time

from ..context import EngineContext
from .index import AgentSessionIndex
from .scan_progress import TRACKER

log = logging.getLogger(__name__)


def scan(ports: EngineContext, index: AgentSessionIndex) -> dict:
    tools, scanned = {}, []
    cache = ports.cache_factory()
    names = list(ports.adapters())
    TRACKER.begin(names)
    started = time.monotonic()
    try:
        for name in names:
            tool = ports.adapter(name)
            source_path = tool.manifest.source_path
            TRACKER.start_tool(name)
            tool_started = time.monotonic()
            try:
                rows = tool.browser.scan(cache)
                scanned.extend((name, tool, row) for row in rows)
                tools[name] = {"ok": True, "count": len(rows), "path": source_path}
                log.info("扫描 %s: %d 条会话 耗时=%.1fs",
                         name, len(rows), time.monotonic() - tool_started)
            except Exception as error:
                tools[name] = {"ok": False, "error": str(error)[:200], "path": source_path}
                log.warning("扫描 %s 失败 耗时=%.1fs: %s",
                            name, time.monotonic() - tool_started, error)
            finally:
                TRACKER.finish_tool(name)
        TRACKER.finalize()
        cache.flush()
        index_started = time.monotonic()
        sessions = [
            {**record.row, "ref": record.opaque_ref, "revision": record.revision}
            for record in index.index_rows(scanned)
        ]
        sessions.sort(key=lambda session: session["updated"], reverse=True)
        log.info("索引 %d 条会话 耗时=%.1fs 全程=%.1fs", len(sessions),
                 time.monotonic() - index_started, time.monotonic() - started)
        return {"tools": tools, "sessions": sessions}
    finally:
        TRACKER.end()


def scan_progress() -> dict:
    return TRACKER.snapshot()
