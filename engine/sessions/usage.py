"""扫描阶段的 token 用量归一化辅助。

三个工具的原始 token 字段口径不同,统一成:
    {"input", "output", "cache_read", "cache_write"}
其中 input 只计未命中缓存的输入(缓存读取单独放 cache_read),便于前端按
models.dev 单价分档估算成本。
"""

from datetime import datetime, timezone

from ..system.pricing import pricing
from .index import AgentSessionIndex
from .safety import (
    finalize_dto,
    redact,
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


def _norm_model(model: str) -> str:
    return str(model or "").split("/")[-1].lower()


def _price_index(prices: dict) -> dict:
    index: dict[str, dict] = {}
    for key, value in prices.items():
        normalized = _norm_model(key)
        index.setdefault(normalized, value)
    return index


def _match_price(model: str, prices: dict, index: dict):
    """与总览页同一套匹配规则:归一后精确命中优先,否则取边界对齐的最近前缀。"""
    if not model or not prices:
        return None
    if model in prices:
        return prices[model]
    normalized = _norm_model(model)
    if normalized in index:
        return index[normalized]
    best, best_diff = None, None
    for key, value in index.items():
        short, long = sorted((normalized, key), key=len)
        if not long.startswith(short):
            continue
        boundary = long[len(short):len(short) + 1]
        if boundary and boundary not in "-_.:":
            continue
        diff = len(long) - len(short)
        if best_diff is None or diff < best_diff:
            best, best_diff = value, diff
    return best


def _cost_of(tokens: dict, price: dict | None) -> float:
    if not price:
        return 0.0
    return sum(
        int(tokens.get(key) or 0) * float(price.get(key) or 0)
        for key in ("input", "output", "cache_read", "cache_write")
    ) / 1_000_000


_MAX_USAGE_BUCKETS = 15


def _top_by_cost(bucket: dict) -> dict:
    """按花费取前若干项:上千个项目全量返回会撑爆 agent 的 DTO 预算。"""
    ordered = sorted(
        bucket.items(),
        key=lambda item: (item[1]["cost"], sum(item[1]["tokens"].values())),
        reverse=True,
    )
    return dict(ordered[:_MAX_USAGE_BUCKETS])


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
    by_model: dict[str, dict] = {}
    by_project: dict[str, dict] = {}
    sessions = 0
    prices = pricing(cached_only=True).get("prices") or {}
    price_index = _price_index(prices)
    cost_total = 0.0
    unpriced_models: set[str] = set()
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
        model = redact(str(row.get("model") or ""), 120)
        price = _match_price(model, prices, price_index)
        cost = _cost_of(tokens, price)
        cost_total += cost
        if model and price is None:
            unpriced_models.add(model)
        for bucket, key in ((by_model, model or "unknown"),
                            (by_project, project or "unknown")):
            entry = bucket.setdefault(
                key, {"tokens": empty_tokens(), "cost": 0.0},
            )
            add_tokens(entry["tokens"], tokens)
            entry["cost"] = round(entry["cost"] + cost, 6)
    return finalize_dto({
        "sessions": sessions,
        "tokens": total,
        "by_agent": by_agent,
        # 金额是按 models.dev 公开单价估算的:模型匹配不上单价时只计 token,
        # 这些模型名列在 unpriced_models 里,免得下游把估算当账单。
        "by_model": _top_by_cost(by_model),
        "by_project": _top_by_cost(by_project),
        "cost": round(cost_total, 4),
        "cost_basis": "estimated_from_public_prices",
        "unpriced_models": sorted(unpriced_models)[:20],
        "currency": "USD",
        # 模型手里没有时钟:不给基准时间,它会去 shell 里跑 date 换算相对区间
        # (在 macOS 上 `date -d` 还会直接失败),或者干脆猜一个窗口。
        "now": int(datetime.now(timezone.utc).timestamp() * 1000),
        "filters": {
            "agents": sorted(allowed_agents) if allowed_agents else None,
            "projects": sorted(allowed_projects) if allowed_projects else None,
            "time_range": {"from": start, "to": end},
        },
    })
