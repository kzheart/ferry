"""Agent 专用窄能力层；所有输出都经过限量、脱敏和引用收窄。"""
from __future__ import annotations

import hashlib
import json
import math
import re
import secrets
import threading
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

from ..domain.errors import AgentReferenceError, AgentRequestError, LocatorStaleError
from ..domain.model import tool_result_text
from ..domain.usage import add_tokens, empty_tokens
from .ports import ApplicationPorts, current

MAX_SEARCH_RESULTS = 50
MAX_CONTENT_SEARCH_RESULTS = 50
MAX_CONTEXT_MESSAGES = 50
MAX_CONTEXT_BYTES = 64 * 1024
DEFAULT_CONTEXT_BYTES = 24 * 1024
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


def _redact(value: str, limit: int | None = None) -> str:
    text = _CONTROL_CHARS.sub("", value)
    for pattern in _SECRET_PATTERNS:
        text = pattern.sub("[REDACTED]", text)
    for pattern in (_FILE_URI, _HOME_PATH, _POSIX_PATH, _WINDOWS_PATH, _UNC_PATH):
        text = pattern.sub("[ABSOLUTE_PATH]", text)
    if limit is not None and len(text) > limit:
        return text[:limit] + "…"
    return text


def _safe_project(value: object) -> str:
    if not isinstance(value, str) or not value:
        return ""
    return _redact(Path(value).name, 120)


def _record_session_id(record, session=None) -> str:
    row = getattr(record, "row", None)
    value = row.get("id") if isinstance(row, dict) else None
    value = value or getattr(session, "source_id", None)
    return _redact(str(value or ""), 512)


def _timestamp(value) -> int | None:
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


def _bounded_int(value, default: int, minimum: int, maximum: int, name: str) -> int:
    if value is None:
        return default
    if isinstance(value, bool) or not isinstance(value, int):
        raise AgentRequestError(f"{name} 必须是整数", {"field": name})
    if value < minimum or value > maximum:
        raise AgentRequestError(
            f"{name} 超出范围", {"field": name, "minimum": minimum, "maximum": maximum}
        )
    return value


def _revision(tool: str, canonical_ref: str, row: dict,
              identity: tuple | str | None = None) -> str:
    stable = json.dumps(
        {
            "tool": tool,
            "ref": canonical_ref,
            "updated": row.get("updated"),
            "size": row.get("size"),
            "file_identity": identity,
        },
        sort_keys=True,
        separators=(",", ":"),
    )
    return hashlib.sha256(stable.encode()).hexdigest()


@dataclass(frozen=True)
class IndexedSession:
    opaque_ref: str
    tool: str
    canonical_ref: str
    root: str | None
    path_backed: bool
    row: dict
    revision: str
    source_identity: tuple | str | None


@dataclass(frozen=True)
class IndexedMessage:
    opaque_locator: str
    session_ref: str
    tool: str
    revision: str
    native_locator: str
    role: str
    editable: bool


def _path_identity(path: Path) -> tuple:
    before = path.stat()
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    after = path.stat()
    if ((before.st_dev, before.st_ino, before.st_mtime_ns, before.st_size) !=
            (after.st_dev, after.st_ino, after.st_mtime_ns, after.st_size)):
        raise AgentReferenceError("会话在计算 revision 时发生变化")
    return (after.st_dev, after.st_ino, after.st_mtime_ns, after.st_size,
            digest.hexdigest())


def _agent_fingerprint(browser, ref: str):
    marker = getattr(browser, "agent_fingerprint", None)
    return (marker or browser.fingerprint)(ref)


class AgentSessionIndex:
    def __init__(self, ports: ApplicationPorts):
        self._ports = ports
        self._by_opaque: dict[str, IndexedSession] = {}
        self._opaque_by_key: dict[tuple[str, str, str], str] = {}
        self._messages_by_opaque: dict[str, IndexedMessage] = {}
        self._opaque_by_message_key: dict[tuple[str, str, str], str] = {}
        self._lock = threading.RLock()

    @property
    def ports(self) -> ApplicationPorts:
        return self._ports

    def refresh(self) -> list[IndexedSession]:
        cache = self._ports.cache_factory()
        scanned = []
        for tool_name in self._ports.adapters():
            plugin = self._ports.adapter(tool_name)
            scanned.extend(
                (tool_name, plugin, row)
                for row in plugin.browser.scan(cache)
            )
        cache.flush()
        return self.index_rows(scanned)

    def index_rows(self, scanned) -> list[IndexedSession]:
        records: list[IndexedSession] = []
        active: set[str] = set()
        with self._lock:
            for tool_name, plugin, row in scanned:
                canonical, root, path_backed, identity = self._canonicalize(plugin, row)
                if canonical is None:
                    continue
                revision = _revision(tool_name, canonical, row, identity)
                key = (tool_name, canonical, revision)
                opaque = self._opaque_by_key.get(key)
                if opaque is None:
                    opaque = "fsr_" + secrets.token_urlsafe(18)
                    self._opaque_by_key[key] = opaque
                record = IndexedSession(
                    opaque, tool_name, canonical, root, path_backed,
                    dict(row), revision, identity,
                )
                self._by_opaque[opaque] = record
                active.add(opaque)
                records.append(record)
            for opaque in set(self._by_opaque) - active:
                stale = self._by_opaque.pop(opaque)
                self._opaque_by_key.pop(
                    (stale.tool, stale.canonical_ref, stale.revision), None)
                stale_messages = [locator for locator, message
                                  in self._messages_by_opaque.items()
                                  if message.session_ref == opaque]
                for locator in stale_messages:
                    message = self._messages_by_opaque.pop(locator)
                    self._opaque_by_message_key.pop(
                        (message.session_ref, message.native_locator, message.role), None)
        return records

    def resolve(self, tool: str, opaque_ref: str) -> IndexedSession:
        if not isinstance(opaque_ref, str) or not opaque_ref.startswith("fsr_"):
            raise AgentReferenceError("ref 不是 Engine 签发的 opaque ref")
        with self._lock:
            record = self._by_opaque.get(opaque_ref)
        if record is None or record.tool != tool:
            raise AgentReferenceError("ref 不在当前扫描索引中", {"tool": tool})
        if record.path_backed:
            try:
                resolved = Path(record.canonical_ref).resolve(strict=True)
                root = Path(record.root or "").resolve(strict=True)
            except OSError as error:
                raise AgentReferenceError("ref 指向的会话已失效") from error
            if not resolved.is_relative_to(root) or not resolved.is_file():
                raise AgentReferenceError("ref 超出 Agent 会话根目录")
            browser = self._ports.adapter(tool).browser
            fingerprint = _agent_fingerprint(browser, str(resolved))
            identity = (_path_identity(resolved), fingerprint)
            if fingerprint is None or record.source_identity != identity:
                raise AgentReferenceError("ref 在扫描后已变化，请重新搜索")
            plugin_ref = browser.resolve_ref(str(resolved))
            if Path(plugin_ref).resolve(strict=True) != resolved:
                raise AgentReferenceError("adapter 未能规范解析 ref")
        else:
            browser = self._ports.adapter(tool).browser
            fingerprint = _agent_fingerprint(browser, record.canonical_ref)
            if fingerprint is None or fingerprint != record.source_identity:
                raise AgentReferenceError("ref 在扫描后已变化，请重新搜索")
        return record

    def issue_message_locator(self, record: IndexedSession,
                              native_locator: str, role: str,
                              editable: bool) -> str:
        if not native_locator or len(native_locator) > 512:
            raise AgentReferenceError("消息缺少可编辑定位信息")
        key = (record.opaque_ref, native_locator, role)
        with self._lock:
            opaque = self._opaque_by_message_key.get(key)
            if opaque is None:
                opaque = "fml_" + secrets.token_urlsafe(18)
                self._opaque_by_message_key[key] = opaque
            self._messages_by_opaque[opaque] = IndexedMessage(
                opaque, record.opaque_ref, record.tool, record.revision,
                native_locator, role, editable)
        return opaque

    def resolve_message_locator(self, record: IndexedSession,
                                opaque_locator: str) -> IndexedMessage:
        hint = "重新调用 ferry_get_session_context，并原样使用 messages[].locator"
        if not isinstance(opaque_locator, str) or not opaque_locator.startswith("fml_"):
            raise AgentReferenceError(
                "locator 不是 Engine 签发的消息引用",
                {"field": "locator", "hint": hint})
        with self._lock:
            message = self._messages_by_opaque.get(opaque_locator)
        if (message is None or message.session_ref != record.opaque_ref
                or message.tool != record.tool or message.revision != record.revision):
            raise LocatorStaleError(
                "消息引用已失效或不属于当前会话",
                {"field": "locator", "hint": hint})
        return message

    @staticmethod
    def _canonicalize(plugin, row: dict) -> tuple[
        str | None, str | None, bool, tuple | str | None
    ]:
        native = plugin.browser.canonicalize(row)
        if native is None:
            return None, None, False, None
        if native.path_backed:
            try:
                root = Path(native.root or "").resolve(strict=True)
                path = Path(native.canonical_ref).resolve(strict=True)
            except OSError:
                return None, None, True, None
            if not path.is_relative_to(root):
                return None, None, True, None
            try:
                fingerprint = _agent_fingerprint(plugin.browser, str(path))
                if fingerprint is None:
                    return None, None, True, None
                identity = (_path_identity(path), fingerprint)
            except (OSError, ValueError, AgentReferenceError):
                return None, None, True, None
            return str(path), str(root), True, identity
        if plugin.browser.resolve_ref(native.canonical_ref) != native.canonical_ref:
            return None, None, False, None
        fingerprint = _agent_fingerprint(plugin.browser, native.canonical_ref)
        if fingerprint is None:
            return None, None, False, None
        return native.canonical_ref, None, False, fingerprint


_INDEX = AgentSessionIndex(current())


def reset_index(ports: ApplicationPorts | None = None) -> None:
    """仅供 composition 切换和测试隔离。"""
    global _INDEX
    _INDEX = AgentSessionIndex(ports or current())


def _string_set(value, name: str, maximum: int, item_size: int) -> set[str]:
    if value is None:
        return set()
    if not isinstance(value, list) or not all(isinstance(item, str) for item in value):
        raise AgentRequestError(f"{name} 必须是字符串数组", {"field": name})
    if len(value) > maximum:
        raise AgentRequestError(
            f"{name} 数量超出范围", {"field": name, "maximum": maximum})
    if any(not item or len(item) > item_size for item in value):
        raise AgentRequestError(
            f"{name} 项长度超出范围", {"field": name, "maximum": item_size})
    return set(value)


def _validate_json_shape(value, *, max_depth: int = 8,
                         max_nodes: int = 2000) -> None:
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


def _validate_ops(ops) -> None:
    if not isinstance(ops, list) or not ops or len(ops) > 50:
        raise AgentRequestError("ops 必须是 1 到 50 项的数组")
    _validate_json_shape(ops)
    rewrite_locators = []
    for op in ops:
        if not isinstance(op, dict):
            raise AgentRequestError("每个 edit op 必须是 object")
        if op.get("op") == "delete-turn":
            if set(op) != {"op", "turn"} or isinstance(op.get("turn"), bool) \
                    or not isinstance(op.get("turn"), int) or op["turn"] < 1:
                raise AgentRequestError("delete-turn 参数非法")
        elif op.get("op") == "rewrite":
            if set(op) != {"op", "locator", "text"}:
                raise AgentRequestError("rewrite 参数非法")
            locator, text = op.get("locator"), op.get("text")
            if (not isinstance(locator, str) or not 1 <= len(locator) <= 512
                    or not isinstance(text, str) or not 1 <= len(text) <= 20_000):
                raise AgentRequestError("rewrite locator/text 超出范围")
            rewrite_locators.append(locator)
        else:
            raise AgentRequestError("Agent edit 仅允许 delete-turn/rewrite")
    if len(rewrite_locators) != len(set(rewrite_locators)):
        raise AgentRequestError(
            "同一消息不能在一次编辑中重复改写", {"field": "ops.locator"})


def resolve_edit_ops(record: IndexedSession, ops: list[dict]) -> list[dict]:
    """把 Agent 可见的 fml_ 定位符换成适配器原生定位符。"""
    resolved = []
    for op in ops:
        item = dict(op)
        if item.get("op") == "rewrite":
            message = _INDEX.resolve_message_locator(record, item["locator"])
            if not message.editable:
                raise AgentRequestError(
                    "目标消息不支持文本改写",
                    {"field": "locator", "locator": item["locator"],
                     "hint": "仅使用 editable=true 的消息引用"})
            item["locator"] = message.native_locator
        resolved.append(item)
    return resolved


def _public_locator_error(ops: list[dict]) -> LocatorStaleError:
    locator = next((op.get("locator") for op in ops
                    if op.get("op") == "rewrite"), None)
    return LocatorStaleError(
        "消息定位信息与当前会话不匹配",
        {"field": "locator", "locator": locator,
         "hint": "重新调用 ferry_get_session_context，并原样使用 messages[].locator"})


def _validated_interval(value) -> tuple[int | None, int | None]:
    interval = value or {}
    if not isinstance(interval, dict) or not set(interval) <= {"from", "to"}:
        raise AgentRequestError("time_range 必须且只能包含 from/to")
    start, end = _timestamp(interval.get("from")), _timestamp(interval.get("to"))
    if start is not None and end is not None and start > end:
        raise AgentRequestError("time_range.from 不得晚于 to")
    return start, end


def _safe_json(value, depth: int = 0):
    if depth > 6:
        return "[truncated]"
    if isinstance(value, str):
        return _redact(value, 1000)
    if isinstance(value, (int, float, bool)) or value is None:
        return value
    if isinstance(value, list):
        return [_safe_json(item, depth + 1) for item in value[:100]]
    if isinstance(value, dict):
        return {
            _redact(str(key), 100): _safe_json(item, depth + 1)
            for key, item in list(value.items())[:50]
        }
    return _redact(str(value), 1000)


def _bounded_json(value, max_bytes: int = 32 * 1024):
    safe = _safe_json(value)
    encoded = json.dumps(safe, ensure_ascii=False, sort_keys=True).encode("utf-8")
    if len(encoded) <= max_bytes:
        return safe
    return {"truncated": True, "sha256": hashlib.sha256(encoded).hexdigest(),
            "preview": encoded[:4000].decode("utf-8", errors="ignore")}


def _finalize_dto(result: dict) -> dict:
    if len(json.dumps(result, ensure_ascii=False).encode("utf-8")) > MAX_AGENT_DTO_BYTES:
        raise AgentRequestError("Agent DTO 超过 64 KiB")
    return result


def search_sessions(query: str = "", agents=None, projects=None, time_range=None,
                    limit: int = 20) -> dict:
    limit = _bounded_int(limit, 20, 1, MAX_SEARCH_RESULTS, "limit")
    if not isinstance(query, str) or len(query) > 500:
        raise AgentRequestError("query 必须是不超过 500 字符的字符串")
    allowed_agents = _string_set(agents, "agents", 8, 32)
    allowed_projects = {
        item.casefold() for item in _string_set(projects, "projects", 20, 256)}
    start, end = _validated_interval(time_range)
    needle = query.strip().casefold()
    matches = []
    for record in _INDEX.refresh():
        row = record.row
        project = _safe_project(row.get("dir"))
        updated = int(row.get("updated") or 0)
        haystack = " ".join((
            str(row.get("title") or ""), project, record.tool,
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
            "session_id": _record_session_id(record),
            "title": _redact(str(row.get("title") or ""), 200),
            "project": project,
            "updated": updated,
            "message_count": int(row.get("count") or 0),
            "model": _redact(str(row.get("model") or ""), 120),
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
    result = {
        "sessions": selected,
        "returned": len(selected),
        "has_more": len(matches) > len(selected),
        "truncation": {
            "truncated": byte_limited,
            "reason": "byte_budget" if byte_limited else None,
            "budget_bytes": MAX_AGENT_DTO_BYTES,
        },
    }
    return _finalize_dto(result)


def _take(text: str, remaining: int) -> tuple[str, int, bool]:
    encoded = text.encode("utf-8")
    if len(encoded) <= remaining:
        return text, remaining - len(encoded), False
    clipped = encoded[:max(0, remaining)].decode("utf-8", errors="ignore")
    return clipped, 0, True


def _validate_read_scope(record: IndexedSession) -> None:
    if not record.path_backed:
        return
    path = Path(record.canonical_ref)
    root = Path(record.root or "").resolve(strict=True)
    if record.tool == "claude":
        child_root = path.with_suffix("") / "subagents"
        candidates = child_root.rglob("*.jsonl") if child_root.exists() else ()
    elif record.tool == "codex":
        candidates = root.rglob("rollout*.jsonl")
    else:
        candidates = ()
    for candidate in candidates:
        try:
            resolved = candidate.resolve(strict=True)
        except OSError as error:
            raise AgentReferenceError("会话子树包含失效文件") from error
        if not resolved.is_file() or not resolved.is_relative_to(root):
            raise AgentReferenceError("会话子树超出 Agent 会话根目录")


def _read_record(record: IndexedSession):
    _validate_read_scope(record)
    browser = _INDEX.ports.adapter(record.tool).browser
    session = getattr(browser, "read_agent", browser.read)(record.canonical_ref)
    _INDEX.resolve(record.tool, record.opaque_ref)
    _validate_read_scope(record)
    return session


def _fit_context_result(result: dict, budget: int) -> dict:
    truncation = result["truncation"]
    while len(json.dumps(result, ensure_ascii=False).encode("utf-8")) > budget:
        messages = result["messages"]
        if not messages:
            result["title"] = ""
            break
        removed = messages.pop()
        next_message = removed["message"]
        current_next = result.get("next_from_message")
        result["next_from_message"] = min(current_next, next_message) \
            if isinstance(current_next, int) else next_message
        truncation["omitted_blocks"] += len(removed["blocks"])
        truncation["truncated"] = True
    result["returned_message_count"] = len(result["messages"])
    result["message_range"]["to"] = (
        result["messages"][-1]["message"] if result["messages"] else None)
    return result


def _message_native_locator(message, index: int) -> str:
    if isinstance(message.source_id, str) and message.source_id:
        return message.source_id
    return f"index:{index}"


def _message_is_rewritable(_tool: str, message) -> bool:
    return any(block.kind == "text" for block in message.blocks)


def get_session_context(tool: str, opaque_ref: str, from_message: int = 1,
                        limit: int = 20,
                        include_tool_outputs: bool = False,
                        max_bytes: int = DEFAULT_CONTEXT_BYTES) -> dict:
    record = _INDEX.resolve(tool, opaque_ref)
    first = _bounded_int(from_message, 1, 1, 1_000_000, "from_message")
    count = _bounded_int(limit, 20, 1, MAX_CONTEXT_MESSAGES, "limit")
    budget = _bounded_int(max_bytes, DEFAULT_CONTEXT_BYTES, 1024,
                          MAX_CONTEXT_BYTES, "max_bytes")
    if not isinstance(include_tool_outputs, bool):
        raise AgentRequestError("include_tool_outputs 必须是 boolean")
    session = _read_record(record)
    total_turns = sum(message.role == "user" for message in session.messages)
    messages, current_turn, remaining = [], 0, budget
    omitted_blocks = omitted_bytes = 0
    exhausted = False
    selected_until = min(len(session.messages), first - 1 + count)
    for index, message in enumerate(session.messages):
        if message.role == "user":
            current_turn += 1
        message_number = index + 1
        if message_number < first or message_number > selected_until:
            continue
        blocks = []
        message_clipped = False
        for block in message.blocks:
            item = None
            if block.kind == "text":
                original = _redact(block.text)
                value, remaining, clipped = _take(original, remaining)
                item = {"kind": "text", "text": value}
                if clipped:
                    message_clipped = True
                    omitted_bytes += len(original.encode("utf-8")) - len(value.encode("utf-8"))
            elif block.kind == "tool" and block.tool:
                result = block.tool.result
                item = {"kind": "tool", "name": _redact(block.tool.name, 120),
                        "op": _redact(str(block.tool.op), 120) if block.tool.op else None,
                        "status": _redact(result.status, 80) if result else None,
                        "input": "[omitted]", "output": "[omitted]"}
                clipped = False
                if include_tool_outputs and remaining:
                    output = _redact(tool_result_text(result))
                    value, remaining, output_clipped = _take(output, remaining)
                    item["output"] = value
                    clipped = clipped or output_clipped
                if clipped:
                    message_clipped = True
                    omitted_blocks += 1
            elif block.kind == "image" and block.image:
                item = {"kind": "image", "id": _redact(block.image.id, 200),
                        "mime_type": _redact(block.image.mime_type, 120),
                        "filename": _redact(Path(block.image.filename).name, 255)
                        if block.image.filename else None,
                        "data": "[omitted]"}
            else:
                omitted_blocks += 1
            if item is not None:
                blocks.append(item)
            if remaining == 0:
                exhausted = True
                break
        editable = _message_is_rewritable(tool, message)
        item = {"message": message_number, "turn": current_turn,
                "role": message.role, "blocks": blocks, "editable": editable,
                "complete": not message_clipped}
        item["locator"] = _INDEX.issue_message_locator(
            record, _message_native_locator(message, index), message.role, editable)
        messages.append(item)
        if exhausted:
            break
    last_returned = messages[-1]["message"] if messages else first - 1
    has_more = last_returned < len(session.messages)
    result = {
        "tool": tool,
        "ref": opaque_ref,
        "session_id": _record_session_id(record, session),
        "title": _redact(session.title, 200),
        "project": _safe_project(session.cwd),
        "revision": record.revision,
        "message_count": len(session.messages),
        "turn_count": total_turns,
        "returned_message_count": len(messages),
        "message_range": {"from": first,
                          "to": last_returned if messages else None},
        "next_from_message": last_returned + 1 if has_more else None,
        "messages": messages,
        "truncation": {"truncated": exhausted or omitted_blocks > 0,
                       "omitted_blocks": omitted_blocks,
                       "omitted_bytes": omitted_bytes,
                       "budget_bytes": budget},
    }
    return _fit_context_result(result, budget)


def search_session_content(tool: str, opaque_ref: str, terms,
                           roles=None, limit: int = 20) -> dict:
    """在单个会话的可见文本中检索，返回可直接用于改写的消息引用。"""
    record = _INDEX.resolve(tool, opaque_ref)
    wanted = _string_set(terms, "terms", 20, 100)
    if not wanted:
        raise AgentRequestError("terms 至少包含一个检索词", {"field": "terms"})
    allowed_roles = _string_set(roles, "roles", 2, 16)
    if not allowed_roles <= {"user", "assistant"}:
        raise AgentRequestError(
            "roles 仅允许 user/assistant", {"field": "roles"})
    maximum = _bounded_int(
        limit, 20, 1, MAX_CONTENT_SEARCH_RESULTS, "limit")
    normalized = [(term, term.casefold()) for term in sorted(wanted)]
    session = _read_record(record)
    total_turns = sum(message.role == "user" for message in session.messages)
    matches = []
    current_turn = 0
    total_matches = 0
    byte_limited = False
    for index, message in enumerate(session.messages):
        if message.role == "user":
            current_turn += 1
        if allowed_roles and message.role not in allowed_roles:
            continue
        text = "\n".join(block.text for block in message.blocks
                         if block.kind == "text" and block.text)
        folded = text.casefold()
        hit_terms = [term for term, folded_term in normalized
                     if folded_term in folded]
        if not hit_terms:
            continue
        total_matches += 1
        if len(matches) >= maximum:
            continue
        first_hit = min(folded.find(term.casefold()) for term in hit_terms)
        start = max(0, first_hit - 240)
        end = min(len(text), first_hit + 560)
        snippet = ("…" if start else "") + text[start:end] + \
            ("…" if end < len(text) else "")
        editable = _message_is_rewritable(tool, message)
        item = {
            "message": index + 1,
            "turn": current_turn,
            "role": message.role,
            "editable": editable,
            "locator": _INDEX.issue_message_locator(
                record, _message_native_locator(message, index), message.role, editable),
            "matched_terms": hit_terms,
            "snippet": _redact(snippet, 900),
            "complete": start == 0 and end == len(text),
        }
        candidate = {"matches": [*matches, item], "message_count": len(session.messages),
                     "turn_count": total_turns, "total_matches": total_matches}
        if len(json.dumps(candidate, ensure_ascii=False).encode("utf-8")) \
                > MAX_AGENT_DTO_BYTES - 2048:
            byte_limited = True
            continue
        matches.append(item)
    has_more = total_matches > len(matches)
    return _finalize_dto({
        "tool": tool,
        "ref": opaque_ref,
        "session_id": _record_session_id(record, session),
        "revision": record.revision,
        "message_count": len(session.messages),
        "turn_count": total_turns,
        "matches": matches,
        "returned": len(matches),
        "total_matches": total_matches,
        "has_more": has_more,
        "truncation": {"truncated": has_more,
                       "reason": "byte_budget" if byte_limited
                       else "result_limit" if has_more else None,
                       "budget_bytes": MAX_AGENT_DTO_BYTES},
    })


def session_read(tool: str, ref: str | None = None, terms=None, roles=None,
                 from_message: int = 1, limit: int = 20,
                 include_tool_outputs: bool = False,
                 max_bytes: int = DEFAULT_CONTEXT_BYTES) -> dict:
    """读取 Engine 索引会话；只接受 scan/search 签发的 opaque ref。"""
    if not isinstance(ref, str) or not ref:
        raise AgentRequestError("必须提供 Engine 签发的 ref", {"field": "ref"})
    if terms is not None:
        result = search_session_content(tool, ref, terms, roles=roles, limit=limit)
        result["mode"] = "search"
    else:
        result = get_session_context(
            tool, ref, from_message=from_message, limit=limit,
            include_tool_outputs=include_tool_outputs, max_bytes=max_bytes)
        result["mode"] = "context"
    return result


def get_usage(agents=None, projects=None, time_range=None) -> dict:
    allowed_agents = _string_set(agents, "agents", 8, 32)
    allowed_projects = {
        item.casefold() for item in _string_set(projects, "projects", 20, 256)}
    start, end = _validated_interval(time_range)
    total = empty_tokens()
    by_agent: dict[str, dict] = {}
    sessions = 0
    for record in _INDEX.refresh():
        row, updated = record.row, int(record.row.get("updated") or 0)
        project = _safe_project(row.get("dir"))
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
    return _finalize_dto({
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


def preview_migration(source_tool: str, opaque_ref: str, target_tool: str,
                      max_turn: int | None = None) -> dict:
    record = _INDEX.resolve(source_tool, opaque_ref)
    if target_tool not in _INDEX.ports.adapters():
        raise AgentRequestError("未知目标 Agent", {"target_tool": target_tool})
    session = _read_record(record)
    if max_turn is not None:
        max_turn = _bounded_int(max_turn, 1, 1, 1_000_000, "max_turn")
        from .services import _truncate_rounds
        _truncate_rounds(session, max_turn)
    target = _INDEX.ports.adapter(target_tool).migration_target
    loss = target.plan(session)
    from .services import _migration_counts
    tree_count, message_count = _migration_counts(session)
    edge_count = sum(len(node.agent_edges) for node in session.walk())
    topology = {"nodes": tree_count, "edges": max(0, tree_count - 1),
                "agent_edges": edge_count, "preserved": True}
    return _finalize_dto({"source_tool": source_tool, "target_tool": target_tool,
            "ref": opaque_ref, "revision": record.revision,
            "source_session_id": _record_session_id(record, session),
            "message_count": message_count,
            "root_message_count": len(session.messages), "tree_count": tree_count,
            "child_count": tree_count - 1, "loss": _bounded_json(loss),
            "topology": topology, "max_turn": max_turn})


def preview_edit(tool: str, opaque_ref: str, *, ops) -> dict:
    record = _INDEX.resolve(tool, opaque_ref)
    plugin = _INDEX.ports.adapter(tool)
    _validate_ops(ops)
    if len(json.dumps(ops, ensure_ascii=False, default=str).encode()) > 64 * 1024:
        raise AgentRequestError("ops 超过 64 KiB")
    from .editing import preview
    editor = plugin.editor
    native_ops = resolve_edit_ops(record, ops)
    try:
        result = preview(editor, record.canonical_ref, native_ops,
                         loader=getattr(editor, "load_preview", None))
    except LocatorStaleError as error:
        raise _public_locator_error(ops) from error
    _INDEX.resolve(tool, opaque_ref)
    return _finalize_dto({"tool": tool, "ref": opaque_ref, "mode": "edit",
            "session_id": _record_session_id(record),
            "revision": _redact(str(result["revision"]), 256),
            "before": _bounded_json(result["before"], 12 * 1024),
            "after": _bounded_json(result["after"], 12 * 1024),
            "changes": _bounded_json(result["changes"], 12 * 1024),
            "capabilities": _bounded_json(
                result.get("capabilities", {}), 12 * 1024)})
