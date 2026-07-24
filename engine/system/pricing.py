"""models.dev 模型单价:抓取 + 磁盘缓存,供前端估算成本。

返回扁平表 {model_id: {input, output, cache_read, cache_write}},单位为每百万
token 的美元价。抓不到时退回上次缓存,再退回内置兜底表——始终返回可用的表,
不因离线而报错(匹配不上的模型前端只记 token、不计价)。
"""

import json
import time
import urllib.request
from pathlib import Path

MODELS_DEV_URL = "https://models.dev/api.json"
CACHE = Path.home() / ".resume-harness" / "models-dev.json"
TTL_SECONDS = 7 * 24 * 3600

# 离线兜底:少量常见公开模型的近似单价(USD / 百万 token)。
_FALLBACK = {
    "claude-opus-4": {"input": 15, "output": 75, "cache_read": 1.5, "cache_write": 18.75},
    "claude-sonnet-4": {"input": 3, "output": 15, "cache_read": 0.3, "cache_write": 3.75},
    "claude-3-5-haiku": {"input": 0.8, "output": 4, "cache_read": 0.08, "cache_write": 1},
    "gpt-5": {"input": 1.25, "output": 10, "cache_read": 0.125, "cache_write": 0},
    "gpt-5-mini": {"input": 0.25, "output": 2, "cache_read": 0.025, "cache_write": 0},
    "gpt-4o": {"input": 2.5, "output": 10, "cache_read": 1.25, "cache_write": 0},
    "deepseek-chat": {"input": 0.27, "output": 1.1, "cache_read": 0.07, "cache_write": 0},
}


def _flatten(api: dict) -> dict:
    prices = {}
    for provider in api.values():
        if not isinstance(provider, dict):
            continue
        for model_id, model in (provider.get("models") or {}).items():
            cost = (model or {}).get("cost") or {}
            if not cost:
                continue
            prices[model_id] = {
                "input": cost.get("input") or 0,
                "output": cost.get("output") or 0,
                "cache_read": cost.get("cache_read") or 0,
                "cache_write": cost.get("cache_write") or 0}
    return prices


def _read_cache() -> dict | None:
    try:
        return json.loads(CACHE.read_text())
    except (OSError, json.JSONDecodeError):
        return None


def _fetch() -> dict:
    request = urllib.request.Request(
        MODELS_DEV_URL, headers={"User-Agent": "resume-harness/1.0"})
    with urllib.request.urlopen(request, timeout=8) as response:
        return json.loads(response.read().decode())


def pricing(force: bool = False) -> dict:
    """返回 {"prices": {...}, "fetched_at": ms, "source": "..."}。"""
    cached = _read_cache()
    fresh = (cached and not force
             and time.time() - (cached.get("fetched_at", 0) / 1000) < TTL_SECONDS)
    if fresh:
        return {"prices": cached["prices"], "fetched_at": cached["fetched_at"],
                "source": "cache"}
    try:
        prices = _flatten(_fetch())
        if prices:
            payload = {"prices": prices, "fetched_at": int(time.time() * 1000)}
            CACHE.parent.mkdir(parents=True, exist_ok=True)
            CACHE.write_text(json.dumps(payload))
            return {**payload, "source": "network"}
    except Exception:
        pass
    if cached and cached.get("prices"):
        return {"prices": cached["prices"], "fetched_at": cached.get("fetched_at", 0),
                "source": "stale"}
    return {"prices": _FALLBACK, "fetched_at": 0, "source": "fallback"}
