"""扫描阶段的 token 用量归一化辅助。

三个工具的原始 token 字段口径不同,统一成:
    {"input", "output", "cache_read", "cache_write"}
其中 input 只计未命中缓存的输入(缓存读取单独放 cache_read),便于前端按
models.dev 单价分档估算成本。
"""

from datetime import datetime, timezone


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
