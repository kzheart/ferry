"""Claude 文件存储扫描。"""

import glob
import json
import os
from pathlib import Path

from ...domain.topology import session_roots


def _clip(text, size=80):
    text = " ".join(text.split())
    return text[:size] + ("…" if len(text) > size else "")


def scan(cache):
    rows = []
    base = Path(os.path.expanduser("~/.claude/projects"))
    for filename in glob.glob(str(base / "**/*.jsonl"), recursive=True):
        path, stat = Path(filename), Path(filename).stat()
        cached = cache.get(path, stat)
        if cached is not None:
            if cached:
                rows.append(cached)
            continue
        cwd, title, count = "", "", 0
        try:
            for line in path.read_text().splitlines():
                if not line.strip():
                    continue
                record = json.loads(line)
                kind = record.get("type")
                if kind in ("user", "assistant"):
                    count += 1
                    cwd = cwd or record.get("cwd", "")
                    content = (record.get("message") or {}).get("content")
                    if (not title and kind == "user" and isinstance(content, str)
                            and content.strip() and not content.strip().startswith("<")):
                        title = _clip(content)
                elif kind == "ai-title":
                    title = record.get("title", "") or title
        except (json.JSONDecodeError, OSError):
            continue
        try:
            relative = path.relative_to(base)
        except ValueError:
            relative = path
        child = len(relative.parts) > 2
        root_id = relative.parts[1] if child else path.stem
        meta = {} if not count else {"tool": "claude", "id": path.stem,
            "title": title, "dir": cwd, "updated": int(stat.st_mtime * 1000),
            "count": count, "size": stat.st_size, "path": str(path),
            "parent_id": root_id if child else None, "root_id": root_id}
        cache.put(path, stat, meta)
        if meta:
            rows.append(meta)
    return session_roots(rows)
