"""扫描阶段的 token 用量归一化辅助。

三个工具的原始 token 字段口径不同,统一成:
    {"input", "output", "cache_read", "cache_write"}
其中 input 只计未命中缓存的输入(缓存读取单独放 cache_read),便于前端按
models.dev 单价分档估算成本。
"""

from datetime import datetime, timezone

from .index import AgentSessionIndex
from .safety import (
    finalize_dto,
    safe_project,
    string_set,
    validated_interval,
)


def empty_tokens() -> dict:
    return {"input": 0, "output": 0, "cache_read": 0, "cache_write": 0}


def add_tokens(acc: dict, other: dict) -> None:
    for key in ("input", "output", "cache_read", "cache_write"):
        acc[key] += int(other.get(key) or 0)


def has_tokens(tokens: dict) -> bool:
    return any(tokens.get(key) for key in ("input", "output", "cache_read", "cache_write"))


def dominant_model(by_model: dict) -> str:
    """出现 token 最多的模型作为该会话的代表模型。"""
    if not by_model:
        return ""
    return max(by_model.items(), key=lambda item: sum(item[1].values()))[0]


def iso_ms(value) -> int | None:
    """ISO8601(带 Z)转毫秒时间戳;已是数字则原样返回。"""
    if value is None:
        return None
    if isinstance(value, (int, float)):
        return int(value)
    try:
        parsed = datetime.fromisoformat(str(value).replace("Z", "+00:00"))
        if parsed.tzinfo is None:
            parsed = parsed.replace(tzinfo=timezone.utc)
        return int(parsed.timestamp() * 1000)
    except (ValueError, TypeError):
        return None


def get_usage(agents=None, projects=None, time_range=None, *,
              index: AgentSessionIndex) -> dict:
    allowed_agents = string_set(agents, "agents", 8, 32)
    allowed_projects = {
        item.casefold()
        for item in string_set(projects, "projects", 20, 256)
    }
    start, end = validated_interval(time_range)
    total = empty_tokens()
    by_agent: dict[str, dict] = {}
    sessions = 0
    for record in index.refresh():
        row = record.row
        updated = int(row.get("updated") or 0)
        project = safe_project(row.get("dir"))
        if allowed_agents and record.tool not in allowed_agents:
            continue
        if allowed_projects and project.casefold() not in allowed_projects:
            continue
        if start is not None and updated < start:
            continue
        if end is not None and updated > end:
            continue
        tokens = row.get("tokens")
        if not isinstance(tokens, dict):
            continue
        sessions += 1
        add_tokens(total, tokens)
        add_tokens(by_agent.setdefault(record.tool, empty_tokens()), tokens)
    return finalize_dto({
        "sessions": sessions,
        "tokens": total,
        "by_agent": by_agent,
        "cost": None,
        "currency": "USD",
        "filters": {
            "agents": sorted(allowed_agents) if allowed_agents else None,
            "projects": sorted(allowed_projects) if allowed_projects else None,
            "time_range": {"from": start, "to": end},
        },
    })
