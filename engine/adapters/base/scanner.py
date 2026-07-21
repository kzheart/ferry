"""Shared scanner mechanics for line-delimited session stores."""
from __future__ import annotations

import glob
import json
from collections.abc import Callable
from pathlib import Path

from ...domain.topology import session_roots


def clip_text(text: str, size: int = 80) -> str:
    text = " ".join(text.split())
    return text[:size] + ("…" if len(text) > size else "")


def scan_jsonl(pattern: str, cache, parse: Callable[[Path, object], dict]) -> list[dict]:
    """Scan cached JSONL files; adapters only implement their record schema."""
    rows = []
    for filename in glob.glob(pattern, recursive=True):
        path = Path(filename)
        try:
            stat = path.stat()
        except OSError:
            continue
        cached = cache.get(path, stat)
        if cached is not None:
            if cached:
                rows.append(cached)
            continue
        try:
            meta = parse(path, stat)
        except (json.JSONDecodeError, OSError):
            continue
        cache.put(path, stat, meta)
        if meta:
            rows.append(meta)
    return session_roots(rows)
