"""会话查询与操作返回值的输入边界和体积限制。"""

import hashlib
import json
import math
import re
from datetime import datetime, timezone

from ..errors import AgentRequestError


MAX_AGENT_DTO_BYTES = 64 * 1024

def truncate_text(value: str, limit: int) -> tuple[str, bool]:
    if len(value) <= limit:
        return value, False
    return value[:limit], True


def record_session_id(record, session=None) -> str:
    row = getattr(record, "row", None)
    value = row.get("id") if isinstance(row, dict) else None
    value = value or getattr(session, "source_id", None)
    return truncate_text(str(value or ""), 512)[0]


_TIME_FORMAT_HINT = (
    "epoch milliseconds, ISO8601, now, or a relative offset like now-7d / -7d "
    "(units s/m/h/d/w)"
)
_RELATIVE_TIME = re.compile(
    r"(?i)^\s*(?:now\s*)?(?:(?P<sign>[-+])\s*(?P<amount>\d{1,6})"
    r"\s*(?P<unit>[smhdw]))?\s*$"
)
_UNIT_MS = {"s": 1_000, "m": 60_000, "h": 3_600_000,
            "d": 86_400_000, "w": 604_800_000}


def _relative_timestamp(text: str) -> int | None:
    """支持 now / now-7d / -7d 这类写法。

    模型手里没有时钟,第一反应就是写相对时间;不认的话它只能去 shell 里跑 date
    (在 macOS 上还会失败),或者干脆猜一个时间戳,把整段统计答错。
    """
    match = _RELATIVE_TIME.match(text)
    if not match or (not text.strip().lower().startswith("now")
                     and not match.group("amount")):
        return None
    now = int(datetime.now(timezone.utc).timestamp() * 1000)
    if not match.group("amount"):
        return now
    delta = int(match.group("amount")) * _UNIT_MS[match.group("unit").lower()]
    return now - delta if match.group("sign") == "-" else now + delta


def timestamp(value) -> int | None:
    if value is None:
        return None
    if isinstance(value, bool):
        raise AgentRequestError("时间格式无效", {"accepts": _TIME_FORMAT_HINT})
    if isinstance(value, (int, float)):
        if not math.isfinite(value):
            raise AgentRequestError("时间必须是有限数值",
                                    {"accepts": _TIME_FORMAT_HINT})
        return int(value)
    text = str(value)
    relative = _relative_timestamp(text)
    if relative is not None:
        return relative
    try:
        parsed = datetime.fromisoformat(text.replace("Z", "+00:00"))
        if parsed.tzinfo is None:
            parsed = parsed.replace(tzinfo=timezone.utc)
        return int(parsed.timestamp() * 1000)
    except (TypeError, ValueError):
        raise AgentRequestError(
            "时间格式无效", {"accepts": _TIME_FORMAT_HINT, "received": text[:64]},
        )


def bounded_int(value, default: int, minimum: int, maximum: int, name: str) -> int:
    if value is None:
        return default
    if isinstance(value, bool) or not isinstance(value, int):
        raise AgentRequestError(f"{name} 必须是整数", {"field": name})
    if value < minimum or value > maximum:
        raise AgentRequestError(
            f"{name} 超出范围",
            {"field": name, "minimum": minimum, "maximum": maximum},
        )
    return value


def string_set(value, name: str, maximum: int, item_size: int) -> set[str]:
    if value is None:
        return set()
    if not isinstance(value, list) or not all(isinstance(item, str) for item in value):
        raise AgentRequestError(f"{name} 必须是字符串数组", {"field": name})
    if len(value) > maximum:
        raise AgentRequestError(
            f"{name} 数量超出范围", {"field": name, "maximum": maximum}
        )
    if any(not item or len(item) > item_size for item in value):
        raise AgentRequestError(
            f"{name} 项长度超出范围", {"field": name, "maximum": item_size}
        )
    return set(value)


def validate_json_shape(
    value,
    *,
    max_depth: int = 8,
    max_nodes: int = 2000,
) -> None:
    nodes = 0

    def visit(item, depth):
        nonlocal nodes
        nodes += 1
        if nodes > max_nodes or depth > max_depth:
            raise AgentRequestError("JSON 结构过深或项目过多")
        if isinstance(item, float) and not math.isfinite(item):
            raise AgentRequestError("JSON 不允许 NaN/Infinity")
        if isinstance(item, dict):
            if not all(isinstance(key, str) and len(key) <= 128 for key in item):
                raise AgentRequestError("JSON key 必须是不超过 128 字符的字符串")
            for child in item.values():
                visit(child, depth + 1)
        elif isinstance(item, list):
            for child in item:
                visit(child, depth + 1)
        elif not isinstance(item, (str, int, float, bool, type(None))):
            raise AgentRequestError("JSON 包含不支持的值")

    visit(value, 0)


def validate_agent_edit_ops(ops) -> None:
    if not isinstance(ops, list) or not ops or len(ops) > 50:
        raise AgentRequestError("ops 必须是 1 到 50 项的数组")
    validate_json_shape(ops)
    rewrite_locators = []
    for operation in ops:
        if not isinstance(operation, dict):
            raise AgentRequestError("每个 edit op 必须是 object")
        if operation.get("op") == "delete-turn":
            if (
                set(operation) != {"op", "turn"}
                or isinstance(operation.get("turn"), bool)
                or not isinstance(operation.get("turn"), int)
                or operation["turn"] < 1
            ):
                raise AgentRequestError("delete-turn 参数非法")
        elif operation.get("op") == "rewrite":
            if set(operation) != {"op", "locator", "text"}:
                raise AgentRequestError("rewrite 参数非法")
            locator, text = operation.get("locator"), operation.get("text")
            if (
                not isinstance(locator, str)
                or not 1 <= len(locator) <= 512
                or not isinstance(text, str)
                or not 1 <= len(text) <= 20_000
            ):
                raise AgentRequestError("rewrite locator/text 超出范围")
            rewrite_locators.append(locator)
        else:
            raise AgentRequestError("Agent edit 仅允许 delete-turn/rewrite")
    if len(rewrite_locators) != len(set(rewrite_locators)):
        raise AgentRequestError(
            "同一消息不能在一次编辑中重复改写", {"field": "ops.locator"}
        )


def validated_interval(value) -> tuple[int | None, int | None]:
    interval = value or {}
    if not isinstance(interval, dict) or not set(interval) <= {"from", "to"}:
        raise AgentRequestError("time_range 必须且只能包含 from/to")
    start, end = timestamp(interval.get("from")), timestamp(interval.get("to"))
    if start is not None and end is not None and start > end:
        raise AgentRequestError("time_range.from 不得晚于 to")
    return start, end


def _bounded_structure(
    value,
    *,
    depth: int = 0,
    budget: dict[str, int] | None = None,
):
    budget = budget or {"nodes": 2000}
    if budget["nodes"] <= 0 or depth > 8:
        return None, True
    budget["nodes"] -= 1
    if isinstance(value, (str, int, float, bool)) or value is None:
        return value, False
    if isinstance(value, list):
        result = []
        truncated = len(value) > 200
        for item in value[:200]:
            child, child_truncated = _bounded_structure(
                item, depth=depth + 1, budget=budget,
            )
            result.append(child)
            truncated = truncated or child_truncated
        return result, truncated
    if isinstance(value, dict):
        result = {}
        truncated = len(value) > 200
        for key, item in list(value.items())[:200]:
            child, child_truncated = _bounded_structure(
                item, depth=depth + 1, budget=budget,
            )
            result[str(key)] = child
            truncated = truncated or child_truncated
        return result, truncated
    return str(value), False


def bounded_json(value, max_bytes: int = 32 * 1024):
    bounded, structurally_truncated = _bounded_structure(value)
    if structurally_truncated:
        bounded = {"truncated": True, "value": bounded}
    encoded = json.dumps(
        bounded, ensure_ascii=False, sort_keys=True,
    ).encode("utf-8")
    if len(encoded) <= max_bytes:
        return bounded
    preview_limit = min(4000, max(0, max_bytes - 256))
    return {
        "truncated": True,
        "sha256": hashlib.sha256(encoded).hexdigest(),
        "preview": encoded[:preview_limit].decode("utf-8", errors="ignore"),
    }


def finalize_dto(result: dict) -> dict:
    if len(json.dumps(result, ensure_ascii=False).encode("utf-8")) > MAX_AGENT_DTO_BYTES:
        raise AgentRequestError("Agent DTO 超过 64 KiB")
    return result
