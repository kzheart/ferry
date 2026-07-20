"""跨工具会话扫描用例。"""

from ..adapters.registry import adapter, adapters
from ..infrastructure.scan_cache import ScanCache


def scan() -> dict:
    tools, sessions = {}, []
    cache = ScanCache()
    for name in adapters():
        tool = adapter(name)
        try:
            rows = tool.scanner(cache)
            sessions.extend(rows)
            tools[name] = {"ok": True, "count": len(rows), "path": tool.source_path}
        except Exception as error:
            tools[name] = {"ok": False, "error": str(error)[:200], "path": tool.source_path}
    cache.flush()
    sessions.sort(key=lambda session: session["updated"], reverse=True)
    return {"tools": tools, "sessions": sessions}
