"""跨工具会话扫描用例。"""

from .ports import current
from . import agent_tools


def scan() -> dict:
    tools, scanned = {}, []
    ports = current()
    cache = ports.cache_factory()
    for name in ports.adapters():
        tool = ports.adapter(name)
        source_path = tool.manifest.source_path
        try:
            rows = tool.browser.scan(cache)
            scanned.extend((name, tool, row) for row in rows)
            tools[name] = {"ok": True, "count": len(rows), "path": source_path}
        except Exception as error:
            tools[name] = {"ok": False, "error": str(error)[:200], "path": source_path}
    cache.flush()
    sessions = [
        {**record.row, "ref": record.opaque_ref, "revision": record.revision}
        for record in agent_tools._INDEX.index_rows(scanned)
    ]
    sessions.sort(key=lambda session: session["updated"], reverse=True)
    return {"tools": tools, "sessions": sessions}
