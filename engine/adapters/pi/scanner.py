"""Pi v3 JSONL session discovery."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path

from ...sessions.scan_progress import TRACKER
from ...sessions.usage import add_tokens, empty_tokens, has_tokens, iso_ms
from ...system.paths import pi_session_roots
from ..shared.scanner import clip_text, iter_lines, path_stat_fingerprint


def _text(content) -> str:
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return "\n".join(str(part.get("text") or "") for part in content
                         if isinstance(part, dict) and part.get("type") == "text")
    return ""


def _meta(path: Path, stat) -> dict:
    records = []
    # 只容忍「文件最后一行写了一半」:坏行后面还有任何一行都按整个文件不可解析
    # 处理。逐行读时读到下一行才知道坏行不是末行,所以先记下再判。
    broken = False
    for line in iter_lines(path):
        if broken:
            return {}
        if not line.strip():
            continue
        try:
            records.append(json.loads(line))
        except json.JSONDecodeError:
            broken = True
    if not records:
        return {}
    header = records[0]
    if (header.get("type") != "session" or header.get("version") != 3
            or not all(isinstance(header.get(key), str) and header[key]
                       for key in ("id", "timestamp", "cwd"))):
        return {}
    entries = records[1:]
    valid = [entry for entry in entries
             if isinstance(entry.get("id"), str) and "parentId" in entry]
    by_id = {entry["id"]: entry for entry in valid}
    branch, seen = [], set()
    current = valid[-1] if valid else None
    while current is not None and current["id"] not in seen:
        branch.append(current)
        seen.add(current["id"])
        current = by_id.get(current.get("parentId"))
    records = [header, *reversed(branch)]
    title, count, model = "", 0, ""
    tokens = empty_tokens()
    for record in records[1:]:
        if record.get("type") == "session_info" and record.get("name"):
            title = str(record["name"])
        if record.get("type") != "message":
            continue
        message = record.get("message") or {}
        role = message.get("role")
        if role in {"user", "assistant", "bashExecution"}:
            count += 1
        if role == "user" and not title:
            candidate = _text(message.get("content"))
            if candidate.strip():
                title = clip_text(candidate)
        if role == "assistant":
            model = str(message.get("model") or model)
            usage = message.get("usage") or {}
            add_tokens(tokens, {
                "input": usage.get("input") or 0,
                "output": usage.get("output") or 0,
                "cache_read": usage.get("cacheRead") or 0,
                "cache_write": usage.get("cacheWrite") or 0,
            })
    return {} if not count else {
        "tool": "pi", "id": header["id"], "title": title,
        "dir": header["cwd"], "updated": int(stat.st_mtime * 1000),
        "created": iso_ms(header["timestamp"]), "count": count,
        "size": stat.st_size, "path": str(path),
        "parent_id": None, "root_id": header["id"],
        "tokens": tokens if has_tokens(tokens) else None, "model": model,
    }


def scan(cache):
    rows = []
    seen = set()
    candidates = [
        path
        for root in pi_session_roots() if root.exists()
        for path in root.rglob("*.jsonl")
    ]
    TRACKER.set_total(len(candidates))
    for path in candidates:
        TRACKER.advance()
        try:
            resolved = path.resolve(strict=True)
            stat = resolved.stat()
        except OSError:
            continue
        if resolved in seen or not resolved.is_file():
            continue
        seen.add(resolved)
        cached = cache.get(resolved, stat) if cache is not None else None
        if cached is None:
            try:
                cached = _meta(resolved, stat)
            except OSError:
                cached = {}
            if cache is not None:
                cache.put(resolved, stat, cached)
        if cached:
            rows.append(cached)
    return rows


def fingerprint(ref: str) -> str:
    path = Path(ref).resolve(strict=True)
    return "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()


agent_fingerprint = path_stat_fingerprint
