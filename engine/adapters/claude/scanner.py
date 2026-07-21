"""Claude 文件存储扫描。"""

import glob
import json
import os
from pathlib import Path

from ...domain.topology import session_roots
from ...domain.usage import add_tokens, dominant_model, empty_tokens, has_tokens, iso_ms


def _clip(text, size=80):
    text = " ".join(text.split())
    return text[:size] + ("…" if len(text) > size else "")


def _usage_tokens(usage) -> dict:
    return {"input": usage.get("input_tokens") or 0,
            "output": usage.get("output_tokens") or 0,
            "cache_read": usage.get("cache_read_input_tokens") or 0,
            "cache_write": usage.get("cache_creation_input_tokens") or 0}


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
        by_model, created = {}, None
        try:
            for line in path.read_text().splitlines():
                if not line.strip():
                    continue
                record = json.loads(line)
                kind = record.get("type")
                if kind in ("user", "assistant"):
                    count += 1
                    cwd = cwd or record.get("cwd", "")
                    ts = iso_ms(record.get("timestamp"))
                    if ts and (created is None or ts < created):
                        created = ts
                    message = record.get("message") or {}
                    model = message.get("model")
                    if kind == "assistant" and model and model != "<synthetic>":
                        add_tokens(by_model.setdefault(model, empty_tokens()),
                                   _usage_tokens(message.get("usage") or {}))
                    content = message.get("content")
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
        tokens = empty_tokens()
        for model_tokens in by_model.values():
            add_tokens(tokens, model_tokens)
        meta = {} if not count else {"tool": "claude", "id": path.stem,
            "title": title, "dir": cwd, "updated": int(stat.st_mtime * 1000),
            "created": created, "count": count, "size": stat.st_size, "path": str(path),
            "tokens": tokens if has_tokens(tokens) else None,
            "model": dominant_model(by_model),
            "parent_id": root_id if child else None, "root_id": root_id}
        cache.put(path, stat, meta)
        if meta:
            rows.append(meta)
    return session_roots(rows)
