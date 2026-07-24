"""会话查询与操作返回值的输入边界、脱敏和体积限制。"""

import hashlib
import json
import math
import re
from datetime import datetime, timezone
from pathlib import Path

from ..errors import AgentRequestError


MAX_AGENT_DTO_BYTES = 64 * 1024

_SECRET_PATTERNS = (
    re.compile(r"(?i)\b(bearer)\s+[A-Za-z0-9._~+/=-]{8,}"),
    re.compile(
        r"(?i)\b[A-Z0-9_]*(?:API_KEY|TOKEN|SECRET|PASSWORD|PRIVATE_KEY)"
        r"[A-Z0-9_]*\s*[:=]\s*[^\s,;]+"
    ),
    re.compile(
        r"(?i)\b(api[_-]?key|access[_-]?token|refresh[_-]?token|token|password|secret)"
        r"\s*[:=]\s*[^\s,;]+"
    ),
    re.compile(r"\bsk-[A-Za-z0-9_-]{12,}\b"),
    re.compile(r"\b(?:gh[opusr]|github_pat)_[A-Za-z0-9_]{16,}\b"),
    re.compile(r"\bAKIA[A-Z0-9]{16}\b"),
    re.compile(r"\bxox[baprs]-[A-Za-z0-9-]{12,}\b"),
    re.compile(
        r"-----BEGIN (?:[A-Z0-9]+ )*PRIVATE KEY-----.*?"
        r"-----END (?:[A-Z0-9]+ )*PRIVATE KEY-----",
        re.DOTALL,
    ),
)
_FILE_URI = re.compile(r"(?i)\bfile://(?:/|\\)[^\s\]\[\)\(\}\{\"']+")
_HOME_PATH = re.compile(r"(?<!\w)~[/\\][^\s\]\[\)\(\}\{\"']+")
_POSIX_PATH = re.compile(r"(?<![:\w])/(?:[^/\s]+/)*[^\s\]\[\)\(\}\{\"']+")
_WINDOWS_PATH = re.compile(r"(?i)\b[A-Z]:[\\/][^\s\]\[\)\(\}\{\"']+")
_UNC_PATH = re.compile(r"\\\\[^\\\s]+\\[^\s\]\[\)\(\}\{\"']+")
_CONTROL_CHARS = re.compile(r"[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]")


def redact(value: str, limit: int | None = None) -> str:
    text = _CONTROL_CHARS.sub("", value)
    for pattern in _SECRET_PATTERNS:
        text = pattern.sub("[REDACTED]", text)
    for pattern in (_FILE_URI, _HOME_PATH, _POSIX_PATH, _WINDOWS_PATH, _UNC_PATH):
        text = pattern.sub("[ABSOLUTE_PATH]", text)
    if limit is not None and len(text) > limit:
        return text[:limit] + "…"
    return text


def safe_project(value: object) -> str:
    if not isinstance(value, str) or not value:
        return ""
    return redact(Path(value).name, 120)


def record_session_id(record, session=None) -> str:
    row = getattr(record, "row", None)
    value = row.get("id") if isinstance(row, dict) else None
    value = value or getattr(session, "source_id", None)
    return redact(str(value or ""), 512)


def timestamp(value) -> int | None:
    if value is None:
        return None
    if isinstance(value, bool):
        raise AgentRequestError("时间必须为毫秒时间戳或 ISO8601")
    if isinstance(value, (int, float)):
        if not math.isfinite(value):
            raise AgentRequestError("时间必须是有限数值")
        return int(value)
    try:
        parsed = datetime.fromisoformat(str(value).replace("Z", "+00:00"))
        if parsed.tzinfo is None:
            parsed = parsed.replace(tzinfo=timezone.utc)
        return int(parsed.timestamp() * 1000)
    except (TypeError, ValueError):
        raise AgentRequestError("时间必须为毫秒时间戳或 ISO8601")


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


def safe_json(value, depth: int = 0):
    if depth > 6:
        return "[truncated]"
    if isinstance(value, str):
        return redact(value, 1000)
    if isinstance(value, (int, float, bool)) or value is None:
        return value
    if isinstance(value, list):
        return [safe_json(item, depth + 1) for item in value[:100]]
    if isinstance(value, dict):
        return {
            redact(str(key), 100): safe_json(item, depth + 1)
            for key, item in list(value.items())[:50]
        }
    return redact(str(value), 1000)


def bounded_json(value, max_bytes: int = 32 * 1024):
    safe = safe_json(value)
    encoded = json.dumps(safe, ensure_ascii=False, sort_keys=True).encode("utf-8")
    if len(encoded) <= max_bytes:
        return safe
    return {
        "truncated": True,
        "sha256": hashlib.sha256(encoded).hexdigest(),
        "preview": encoded[:4000].decode("utf-8", errors="ignore"),
    }


def finalize_dto(result: dict) -> dict:
    if len(json.dumps(result, ensure_ascii=False).encode("utf-8")) > MAX_AGENT_DTO_BYTES:
        raise AgentRequestError("Agent DTO 超过 64 KiB")
    return result
