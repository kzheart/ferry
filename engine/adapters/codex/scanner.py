"""Codex rollout 文件扫描。"""

import glob
import json
import os
from pathlib import Path

from ...domain.topology import session_roots
from .reader import session_id


def _clip(text, size=80):
    text = " ".join(text.split())
    return text[:size] + ("…" if len(text) > size else "")


def scan(cache):
    rows = []
    pattern = os.path.expanduser("~/.codex/sessions/*/*/*/rollout-*.jsonl")
    for filename in glob.glob(pattern):
        path, stat = Path(filename), Path(filename).stat()
        cached = cache.get(path, stat)
        if cached is not None:
            if cached:
                rows.append(cached)
            continue
        sid, cwd, title, count, parent_id = path.stem, "", "", 0, None
        root_id = agent_id = agent_path = agent_type = None
        has_meta = False
        try:
            for line in path.read_text().splitlines():
                if not line.strip():
                    continue
                record = json.loads(line)
                if record.get("type") == "session_meta" and not has_meta:
                    payload = record["payload"]
                    sid, cwd = session_id(payload, sid), payload.get("cwd", "")
                    source = payload.get("source") or payload.get("thread_source") or {}
                    source = source if isinstance(source, dict) else {}
                    subagent = source.get("subagent") or {}
                    subagent = subagent if isinstance(subagent, dict) else {}
                    spawn = subagent.get("thread_spawn") or {}
                    spawn = spawn if isinstance(spawn, dict) else {}
                    root_id = payload.get("session_id") or sid
                    parent_id = payload.get("parent_thread_id") or spawn.get("parent_thread_id") or subagent.get("parent_thread_id")
                    if not parent_id and root_id != sid:
                        parent_id = root_id
                    agent_id = subagent.get("agent_id") or spawn.get("agent_id") or payload.get("agent_id")
                    agent_path = subagent.get("agent_path") or spawn.get("agent_path") or payload.get("agent_path")
                    agent_type = subagent.get("agent_type") or spawn.get("agent_type") or payload.get("agent_type")
                    has_meta = True
                elif record.get("type") == "response_item":
                    payload = record["payload"]
                    if payload.get("type") == "message":
                        count += 1
                        text = "\n".join(block.get("text", "") for block in payload.get("content", []))
                        if not title and payload.get("role") == "user" and text.strip() and text.strip()[0] not in "<[":
                            title = _clip(text)
        except (json.JSONDecodeError, OSError):
            continue
        meta = {} if not count else {"tool": "codex", "id": sid, "title": title,
            "dir": cwd, "updated": int(stat.st_mtime * 1000), "count": count,
            "size": stat.st_size, "path": str(path), "parent_id": parent_id,
            "root_id": root_id or sid, "agent_id": agent_id,
            "agent_path": agent_path, "agent_type": agent_type}
        cache.put(path, stat, meta)
        if meta:
            rows.append(meta)
    return session_roots(rows)
