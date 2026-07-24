"""跨 Agent 会话索引搜索。"""
from __future__ import annotations

import json

from ..errors import AgentRequestError
from .index import AgentSessionIndex
from .safety import (
    MAX_AGENT_DTO_BYTES,
    bounded_int,
    finalize_dto,
    record_session_id,
    redact,
    safe_project,
    string_set,
    validated_interval,
)

MAX_SEARCH_RESULTS = 50


def search_sessions(query: str = "", agents=None, projects=None,
                    time_range=None, limit: int = 20, *,
                    index: AgentSessionIndex) -> dict:
    limit = bounded_int(limit, 20, 1, MAX_SEARCH_RESULTS, "limit")
    if not isinstance(query, str) or len(query) > 500:
        raise AgentRequestError("query 必须是不超过 500 字符的字符串")
    allowed_agents = string_set(agents, "agents", 8, 32)
    allowed_projects = {
        item.casefold()
        for item in string_set(projects, "projects", 20, 256)
    }
    start, end = validated_interval(time_range)
    needle = query.strip().casefold()
    matches = []
    for record in index.refresh():
        row = record.row
        project = safe_project(row.get("dir"))
        updated = int(row.get("updated") or 0)
        haystack = " ".join((
            str(row.get("title") or ""),
            project,
            record.tool,
            str(row.get("model") or ""),
        )).casefold()
        if allowed_agents and record.tool not in allowed_agents:
            continue
        if allowed_projects and project.casefold() not in allowed_projects:
            continue
        if needle and needle not in haystack:
            continue
        if start is not None and updated < start:
            continue
        if end is not None and updated > end:
            continue
        matches.append({
            "tool": record.tool,
            "ref": record.opaque_ref,
            "session_id": record_session_id(record),
            "title": redact(str(row.get("title") or ""), 200),
            "project": project,
            "updated": updated,
            "message_count": int(row.get("count") or 0),
            "model": redact(str(row.get("model") or ""), 120),
            "revision": record.revision,
        })
    matches.sort(key=lambda item: item["updated"], reverse=True)
    selected = []
    byte_limited = False
    for item in matches[:limit]:
        candidate = {
            "sessions": [*selected, item],
            "returned": len(selected) + 1,
            "has_more": len(matches) > len(selected) + 1,
            "truncation": {
                "truncated": True,
                "reason": "byte_budget",
                "budget_bytes": MAX_AGENT_DTO_BYTES,
            },
        }
        if len(json.dumps(candidate, ensure_ascii=False).encode("utf-8")) \
                > MAX_AGENT_DTO_BYTES:
            byte_limited = True
            break
        selected.append(item)
    return finalize_dto({
        "sessions": selected,
        "returned": len(selected),
        "has_more": len(matches) > len(selected),
        "truncation": {
            "truncated": byte_limited,
            "reason": "byte_budget" if byte_limited else None,
            "budget_bytes": MAX_AGENT_DTO_BYTES,
        },
    })
