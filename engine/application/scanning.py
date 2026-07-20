"""跨工具会话扫描用例。"""

from .ports import current


def scan() -> dict:
    tools, sessions = {}, []
    ports = current()
    cache = ports.cache_factory()
    for name in ports.adapters():
        tool = ports.adapter(name)
        source_path = tool.manifest.source_path
        try:
            rows = tool.browser.scan(cache)
            sessions.extend(rows)
            tools[name] = {"ok": True, "count": len(rows), "path": source_path}
        except Exception as error:
            tools[name] = {"ok": False, "error": str(error)[:200], "path": source_path}
    cache.flush()
    sessions.sort(key=lambda session: session["updated"], reverse=True)
    return {"tools": tools, "sessions": sessions}
