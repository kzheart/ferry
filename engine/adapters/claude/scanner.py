"""Claude 文件存储扫描。"""

import json
import os
from pathlib import Path

from ...domain.usage import add_tokens, dominant_model, empty_tokens, has_tokens, iso_ms
from ..base.scanner import clip_text, scan_jsonl


def _usage_tokens(usage) -> dict:
    return {"input": usage.get("input_tokens") or 0,
            "output": usage.get("output_tokens") or 0,
            "cache_read": usage.get("cache_read_input_tokens") or 0,
            "cache_write": usage.get("cache_creation_input_tokens") or 0}


def _meta(path: Path, stat, base: Path) -> dict:
    cwd, title, count = "", "", 0
    by_model, created = {}, None
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
                title = clip_text(content)
        elif kind == "ai-title":
            title = record.get("title", "") or title
    try:
        relative = path.relative_to(base)
    except ValueError:
        relative = path
    child = len(relative.parts) > 2
    root_id = relative.parts[1] if child else path.stem
    tokens = empty_tokens()
    for model_tokens in by_model.values():
        add_tokens(tokens, model_tokens)
    return {} if not count else {
        "tool": "claude", "id": path.stem, "title": title, "dir": cwd,
        "updated": int(stat.st_mtime * 1000), "created": created,
        "count": count, "size": stat.st_size, "path": str(path),
        "tokens": tokens if has_tokens(tokens) else None,
        "model": dominant_model(by_model), "parent_id": root_id if child else None,
        "root_id": root_id,
    }


def scan(cache):
    base = Path(os.path.expanduser("~/.claude/projects"))
    return scan_jsonl(str(base / "**/*.jsonl"), cache,
                      lambda path, stat: _meta(path, stat, base))
