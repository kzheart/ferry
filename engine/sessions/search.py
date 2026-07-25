"""跨 Agent 会话索引搜索。"""
from __future__ import annotations

import json
import math
from datetime import datetime, timezone

from ..errors import AgentRequestError
from .content_index import ContentIndex, parse_query_terms
from .index import AgentSessionIndex
from .safety import (
    MAX_AGENT_DTO_BYTES,
    bounded_int,
    finalize_dto,
    record_session_id,
    string_set,
    truncate_text,
    validated_interval,
)

MAX_SEARCH_RESULTS = 50
_SCOPES = ("any", "metadata", "content")
_SNIPPET_CHARS = 320


def _validated_scope(scope, needle: str) -> str:
    if scope not in _SCOPES:
        raise AgentRequestError(
            "scope 仅允许 any/metadata/content",
            {"field": "scope", "accepts": list(_SCOPES)},
        )
    if scope == "content" and not needle:
        raise AgentRequestError(
            "scope=content 需要非空 query", {"field": "query"},
        )
    return scope


def search_sessions(query: str = "", agents=None, projects=None,
                    time_range=None, limit: int = 20,
                    scope: str = "any",
                    include_tool_outputs: bool = False, *,
                    index: AgentSessionIndex,
                    content_index: ContentIndex | None = None) -> dict:
    limit = bounded_int(limit, 20, 1, MAX_SEARCH_RESULTS, "limit")
    if not isinstance(query, str) or len(query) > 500:
        raise AgentRequestError("query 必须是不超过 500 字符的字符串")
    if not isinstance(include_tool_outputs, bool):
        raise AgentRequestError("include_tool_outputs 必须是 boolean")
    allowed_agents = string_set(agents, "agents", 8, 32)
    allowed_projects = {
        item.casefold()
        for item in string_set(projects, "projects", 20, 256)
    }
    start, end = validated_interval(time_range)
    needle = query.strip().casefold()
    scope = _validated_scope(scope, needle)
    # 与内容检索同口径:空白拆词,同一目标里全部命中才算数。
    needle_terms = (
        [term.casefold() for term in parse_query_terms(needle)]
        if needle else []
    )

    records = index.refresh()
    # 内容命中与索引覆盖度。索引不可用时特性显式降级为纯元数据搜索。
    content_hits: dict = {}
    content_status: dict | None = None
    content_active = bool(needle) and scope != "metadata"
    if content_active:
        if content_index is None:
            content_status = {
                "ready": False, "reason": "content_index_unavailable",
            }
        else:
            content_status = content_index.sync(index, records)
            hits, meta = content_index.search(
                query.strip(), include_tool_outputs=include_tool_outputs,
            )
            content_hits = hits
            content_status.update(meta)

    matches = []
    ranking = {}
    hit_by_item = {}
    for record in records:
        row = record.row
        project, project_truncated = truncate_text(
            str(row.get("dir") or ""), 1024,
        )
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
        if start is not None and updated < start:
            continue
        if end is not None and updated > end:
            continue
        metadata_hit = bool(needle_terms) and all(
            term in haystack for term in needle_terms
        )
        content_hit = content_hits.get((record.tool, record.canonical_ref))
        if needle:
            if scope == "metadata" and not metadata_hit:
                continue
            if scope == "content" and content_hit is None:
                continue
            if scope == "any" and not metadata_hit and content_hit is None:
                continue
        title, title_truncated = truncate_text(
            str(row.get("title") or ""), 200,
        )
        model, model_truncated = truncate_text(
            str(row.get("model") or ""), 120,
        )
        item = {
            "tool": record.tool,
            "ref": record.opaque_ref,
            "session_id": record_session_id(record),
            "title": title,
            "project": project,
            "title_truncated": title_truncated,
            "project_truncated": project_truncated,
            "updated": updated,
            # 与 session_read 的 message_count 不是一回事:这里是原始转录条数,
            # 读取视图会把同一轮的多个片段合并,数字明显更小。同名会让模型把
            # 两个数当成同一个口径,先后报给用户互相矛盾的结果。
            "record_count": int(row.get("count") or 0),
            "model": model,
            "model_truncated": model_truncated,
            "revision": record.revision,
        }
        if needle and content_active:
            matched_in = []
            if metadata_hit:
                matched_in.append("metadata")
            if content_hit is not None:
                matched_in.append("content")
                item["content_match_count"] = content_hit["count"]
            item["matched_in"] = matched_in
        # 有内容命中时按相关性分组排序:双命中 > 内容命中(bm25) > 纯元数据。
        rank = (
            content_hit["best_rank"] if content_hit is not None else None
        )
        group = 2
        if content_hit is not None:
            group = 0 if metadata_hit else 1
            hit_by_item[id(item)] = content_hit
        ranking[id(item)] = (
            group,
            rank if rank is not None else math.inf,
            -updated,
        )
        matches.append(item)
    if content_active and content_hits:
        matches.sort(key=lambda item: ranking[id(item)])
    else:
        matches.sort(key=lambda item: item["updated"], reverse=True)

    # 摘要只为最终返回页生成，避免为未返回结果做额外读取。
    if content_active and content_index is not None:
        for item in matches[:limit]:
            hit = hit_by_item.get(id(item))
            if hit is None:
                continue
            item["content_matches"] = [
                {
                    "message": row["message"],
                    "turn": row["turn"],
                    "role": row["role"],
                    "snippet": truncate_text(
                        content_index.snippet(
                            row["id"], query.strip(), include_tool_outputs,
                        ),
                        _SNIPPET_CHARS,
                    )[0],
                }
                for row in hit["rows"]
            ]

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
    result = {
        "sessions": selected,
        "returned": len(selected),
        # 只给 has_more 的话,模型没法判断自己看到的是全部还是九牛一毛,
        # 于是把 20 条样本当成"你的全部会话"讲给用户。
        "total_matches": len(matches),
        "has_more": len(matches) > len(selected),
        # 相对时间窗要靠这个基准算,否则模型只能去 shell 里问 date。
        "now": int(datetime.now(timezone.utc).timestamp() * 1000),
        "truncation": {
            "truncated": byte_limited,
            "reason": "byte_budget" if byte_limited else None,
            "budget_bytes": MAX_AGENT_DTO_BYTES,
        },
    }
    if content_active:
        result["content_index"] = content_status
    return finalize_dto(result)
