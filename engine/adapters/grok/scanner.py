"""Recursive Grok summary scanner."""
from __future__ import annotations

import hashlib
import json

from ...sessions.usage import iso_ms
from ...sessions.topology import session_roots
from ...system.paths import grok_home


def _meta(path):
    summary_path = path / "summary.json"
    summary = json.loads(summary_path.read_text())
    info = summary.get("info") or {}
    if summary.get("chat_format_version") != 1 or not info.get("id") \
            or not isinstance(info.get("cwd"), str):
        return {}
    stat = summary_path.stat()
    return {
        "tool": "grok", "id": info["id"],
        "title": summary.get("generated_title")
                 or summary.get("session_summary") or "",
        "dir": info["cwd"], "updated": iso_ms(summary.get("updated_at"))
               or int(stat.st_mtime * 1000),
        "created": iso_ms(summary.get("created_at")),
        "count": summary.get("num_chat_messages")
                 or summary.get("num_messages") or 0,
        "size": stat.st_size, "path": str(path),
        "parent_id": summary.get("parent_session_id"),
        "root_id": summary.get("root_session_id") or info["id"],
        "tokens": None, "model": summary.get("current_model_id") or "",
        "authoritative_members": [
            "summary.json",
            "updates.jsonl" if (path / "updates.jsonl").is_file()
            else "chat_history.jsonl",
        ],
    }


def scan(cache):
    root = grok_home() / "sessions"
    rows = []
    if not root.exists():
        return rows
    for summary in root.rglob("summary.json"):
        path, stat = summary.parent, summary.stat()
        cached = cache.get(summary, stat) if cache else None
        if cached is None:
            try:
                cached = _meta(path)
            except (OSError, json.JSONDecodeError):
                cached = {}
            if cache:
                cache.put(summary, stat, cached)
        if cached:
            rows.append(cached)
    return session_roots(rows)


def agent_fingerprint(ref):
    path = __import__("pathlib").Path(ref).resolve(strict=True)
    stat = (path / "summary.json").stat()
    marker = f"{path}:{stat.st_dev}:{stat.st_ino}:{stat.st_mtime_ns}:{stat.st_size}"
    return "stat:" + hashlib.sha256(marker.encode()).hexdigest()


def fingerprint(ref):
    from .store import fingerprint as bundle_fingerprint

    return bundle_fingerprint(__import__("pathlib").Path(ref))
