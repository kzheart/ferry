"""OpenCode SQLite 存储扫描。"""

import sqlite3
from pathlib import Path

from ...domain.topology import session_roots

OPENCODE_DB = Path.home() / ".local/share/opencode/opencode.db"


def scan(_cache):
    if not OPENCODE_DB.exists():
        return []
    uri = f"file:{OPENCODE_DB}?mode=ro"
    with sqlite3.connect(uri, uri=True, timeout=5) as database:
        counts = dict(database.execute("SELECT session_id, COUNT(*) FROM message GROUP BY session_id"))
        records = database.execute("SELECT id, title, directory, time_updated, parent_id FROM session").fetchall()
    rows = [{"tool": "opencode", "id": sid, "title": title or "",
        "dir": directory or "", "updated": updated or 0,
        "count": counts.get(sid, 0), "size": 0, "path": "", "parent_id": parent}
        for sid, title, directory, updated, parent in records]
    return [root for root in session_roots(rows) if root["count"]]
