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

from ..domain.errors import AgentReferenceError, AgentRequestError
from ..domain.usage import add_tokens, empty_tokens
from .ports import current

MAX_SEARCH_RESULTS = 50
MAX_CONTEXT_TURNS = 20
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


class AgentSessionIndex:
    def __init__(self):
        self._by_opaque: dict[str, IndexedSession] = {}
        self._opaque_by_key: dict[tuple[str, str, str], str] = {}
        self._lock = threading.RLock()

    def refresh(self) -> list[IndexedSession]:
        ports = current()
        cache = ports.cache_factory()
        records: list[IndexedSession] = []
        active: set[str] = set()
        with self._lock:
            for tool_name in ports.adapters():
                plugin = ports.adapter(tool_name)
                for row in plugin.browser.scan(cache):
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
            cache.flush()
            for opaque in set(self._by_opaque) - active:
                stale = self._by_opaque.pop(opaque)
                self._opaque_by_key.pop(
                    (stale.tool, stale.canonical_ref, stale.revision), None)
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
            browser = current().adapter(tool).browser
            fingerprint = getattr(browser, "fingerprint", lambda _ref: None)(
                str(resolved))
            identity = (_path_identity(resolved), fingerprint)
            if fingerprint is None or record.source_identity != identity:
                raise AgentReferenceError("ref 在扫描后已变化，请重新搜索")
            plugin_ref = browser.resolve_ref(str(resolved))
            if Path(plugin_ref).resolve(strict=True) != resolved:
                raise AgentReferenceError("adapter 未能规范解析 ref")
        else:
            fingerprint = getattr(current().adapter(tool).browser,
                                  "fingerprint", lambda _ref: None)(record.canonical_ref)
            if fingerprint is None or fingerprint != record.source_identity:
                raise AgentReferenceError("ref 在扫描后已变化，请重新搜索")
        return record

    @staticmethod
    def _canonicalize(plugin, row: dict) -> tuple[
        str | None, str | None, bool, tuple | str | None
    ]:
        if plugin.manifest.reference_kind == "path":
            raw = row.get("path")
            if not isinstance(raw, str) or not raw:
                return None, None, True, None
            try:
                root = Path(plugin.manifest.source_path).expanduser().resolve(strict=True)
                path = Path(raw).expanduser().resolve(strict=True)
            except OSError:
                return None, None, True, None
            if (not path.is_file() or path.suffix != ".jsonl"
                    or not path.is_relative_to(root)):
                return None, None, True, None
            try:
                resolved = Path(plugin.browser.resolve_ref(str(path))).resolve(strict=True)
            except (OSError, ValueError):
                return None, None, True, None
            if resolved != path:
                return None, None, True, None
            try:
                fingerprint = getattr(
                    plugin.browser, "fingerprint", lambda _ref: None)(str(path))
                if fingerprint is None:
                    return None, None, True, None
                identity = (_path_identity(path), fingerprint)
            except (OSError, ValueError, AgentReferenceError):
                return None, None, True, None
            return str(path), str(root), True, identity
        raw = row.get("id")
        if not isinstance(raw, str) or not raw or len(raw) > 512 or "\x00" in raw:
            return None, None, False, None
        if plugin.browser.resolve_ref(raw) != raw:
            return None, None, False, None
        fingerprint = getattr(plugin.browser, "fingerprint", lambda _ref: None)(raw)
        if fingerprint is None:
            return None, None, False, None
        return raw, None, False, fingerprint


_INDEX = AgentSessionIndex()


def reset_index() -> None:
    """仅供 composition 切换和测试隔离。"""
    global _INDEX
    _INDEX = AgentSessionIndex()


def list_capabilities() -> dict:
    tools = []
    ports = current()
    for name in ports.adapters():
        plugin = ports.adapter(name)
        tools.append({
            "id": plugin.id,
            "display_name": _redact(plugin.manifest.display_name, 80),
            "capabilities": plugin.capabilities(),
            "reference_kind": "opaque",
        })
    return _finalize_dto({
        "tools": tools,
        "limits": {
            "search_results": MAX_SEARCH_RESULTS,
            "context_turns": MAX_CONTEXT_TURNS,
            "context_bytes": MAX_CONTEXT_BYTES,
        },
        "mutations_require_approval": True,
        "destructive_tools": False,
    })


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
        else:
            raise AgentRequestError("Agent edit 仅允许 delete-turn/rewrite")


def _validate_reply_payload(turn, reply) -> None:
    if isinstance(turn, bool) or not isinstance(turn, int) or turn < 1:
        raise AgentRequestError("turn 必须是正整数")
    _validate_json_shape(reply)
    if not isinstance(reply, dict) or set(reply) != {"items"}:
        raise AgentRequestError("reply 必须且只能包含 items")
    items = reply["items"]
    if not isinstance(items, list) or not 1 <= len(items) <= 100:
        raise AgentRequestError("reply.items 必须是 1 到 100 项的数组")
    for item in items:
        if not isinstance(item, dict):
            raise AgentRequestError("reply item 必须是 object")
        if item.get("kind") == "text":
            text = item.get("text")
            if set(item) != {"kind", "text"} or not isinstance(text, str) \
                    or not 1 <= len(text) <= 20_000:
                raise AgentRequestError("reply text item 参数非法")
        elif item.get("kind") == "tool":
            if set(item) != {"kind", "name", "input", "output"}:
                raise AgentRequestError("reply tool item 参数非法")
            if (not isinstance(item.get("name"), str)
                    or not 1 <= len(item["name"]) <= 120
                    or not isinstance(item.get("input"), (dict, str))
                    or not isinstance(item.get("output"), str)
                    or len(item["output"]) > 20_000):
                raise AgentRequestError("reply tool item 字段超出范围")
            if isinstance(item["input"], str) and len(item["input"]) > 20_000:
                raise AgentRequestError("reply tool input 超出范围")
        else:
            raise AgentRequestError("reply item kind 仅允许 text/tool")


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
    browser = current().adapter(record.tool).browser
    session = getattr(browser, "read_agent", browser.read)(record.canonical_ref)
    _INDEX.resolve(record.tool, record.opaque_ref)
    _validate_read_scope(record)
    return session


def _fit_context_result(result: dict, budget: int) -> dict:
    truncation = result["truncation"]
    while len(json.dumps(result, ensure_ascii=False).encode("utf-8")) > budget:
        turns = result["turns"]
        if not turns:
            result["title"] = ""
            break
        if turns[-1]["blocks"]:
            turns[-1]["blocks"].pop()
            truncation["omitted_blocks"] += 1
        else:
            turns.pop()
        truncation["truncated"] = True
    return result


def get_session_context(tool: str, opaque_ref: str, from_turn: int = 1,
                        to_turn: int | None = None,
                        include_tool_outputs: bool = False,
                        max_bytes: int = DEFAULT_CONTEXT_BYTES) -> dict:
    record = _INDEX.resolve(tool, opaque_ref)
    first = _bounded_int(from_turn, 1, 1, 1_000_000, "from_turn")
    last = first + MAX_CONTEXT_TURNS - 1 if to_turn is None else _bounded_int(
        to_turn, first, first, 1_000_000, "to_turn")
    if last - first + 1 > MAX_CONTEXT_TURNS:
        raise AgentRequestError("单次最多读取 20 轮")
    budget = _bounded_int(max_bytes, DEFAULT_CONTEXT_BYTES, 1024,
                          MAX_CONTEXT_BYTES, "max_bytes")
    if not isinstance(include_tool_outputs, bool):
        raise AgentRequestError("include_tool_outputs 必须是 boolean")
    session = _read_record(record)
    turns, current_turn, remaining = [], 0, budget
    omitted_blocks = omitted_bytes = 0
    exhausted = False
    for message in session.messages:
        if message.role == "user":
            current_turn += 1
        if current_turn < first or current_turn > last or current_turn == 0:
            continue
        blocks = []
        for block in message.blocks:
            item = None
            if block.kind == "text":
                original = _redact(block.text)
                value, remaining, clipped = _take(original, remaining)
                item = {"kind": "text", "text": value}
                if clipped:
                    omitted_bytes += len(original.encode("utf-8")) - len(value.encode("utf-8"))
            elif block.kind == "tool" and block.tool:
                item = {"kind": "tool", "name": _redact(block.tool.name, 120),
                        "op": _redact(str(block.tool.op), 120) if block.tool.op else None,
                        "status": _redact(str(block.tool.status), 80)
                        if block.tool.status else None,
                        "input": "[omitted]", "output": "[omitted]"}
                clipped = False
                if include_tool_outputs and remaining:
                    output = _redact(block.tool.output or "")
                    value, remaining, output_clipped = _take(output, remaining)
                    item["output"] = value
                    clipped = clipped or output_clipped
                if clipped:
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
        turns.append({"turn": current_turn, "role": message.role, "blocks": blocks})
        if exhausted:
            break
    result = {
        "tool": tool,
        "ref": opaque_ref,
        "title": _redact(session.title, 200),
        "project": _safe_project(session.cwd),
        "revision": record.revision,
        "turn_range": {"from": first, "to": last},
        "turns": turns,
        "truncation": {"truncated": exhausted or omitted_blocks > 0,
                       "omitted_blocks": omitted_blocks,
                       "omitted_bytes": omitted_bytes,
                       "budget_bytes": budget},
    }
    return _fit_context_result(result, budget)


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
    })


def preview_migration(source_tool: str, opaque_ref: str, target_tool: str,
                      max_turn: int | None = None) -> dict:
    record = _INDEX.resolve(source_tool, opaque_ref)
    if target_tool not in current().adapters():
        raise AgentRequestError("未知目标 Agent", {"target_tool": target_tool})
    session = _read_record(record)
    if max_turn is not None:
        max_turn = _bounded_int(max_turn, 1, 1, 1_000_000, "max_turn")
        from .services import _truncate_rounds
        _truncate_rounds(session, max_turn)
    target = current().adapter(target_tool).require("migration_target")
    loss = target.plan(session)
    tree_count = sum(1 for _ in session.walk())
    edge_count = sum(len(node.agent_edges) for node in session.walk())
    topology = {"nodes": tree_count, "edges": max(0, tree_count - 1),
                "agent_edges": edge_count, "preserved": True}
    return _finalize_dto({"source_tool": source_tool, "target_tool": target_tool,
            "ref": opaque_ref, "revision": record.revision,
            "message_count": len(session.messages), "tree_count": tree_count,
            "child_count": tree_count - 1, "loss": _bounded_json(loss),
            "topology": topology, "max_turn": max_turn})


def preview_edit(tool: str, opaque_ref: str, *, ops=None, turn=None,
                 reply=None) -> dict:
    record = _INDEX.resolve(tool, opaque_ref)
    plugin = current().adapter(tool)
    if ops is not None:
        _validate_ops(ops)
        if len(json.dumps(ops, ensure_ascii=False, default=str).encode()) > 64 * 1024:
            raise AgentRequestError("ops 超过 64 KiB")
        from .editing import preview
        editor = plugin.require("editor")
        result = preview(editor, record.canonical_ref, ops,
                         loader=getattr(editor, "load_preview", None))
        mode = "edit"
    elif turn is not None and reply is not None:
        _validate_reply_payload(turn, reply)
        if len(json.dumps(reply, ensure_ascii=False, default=str).encode()) > 64 * 1024:
            raise AgentRequestError("reply 超过 64 KiB")
        from .authoring import preview
        editor = plugin.require("editor")
        result = preview(editor, plugin.require("authoring"),
                         record.canonical_ref, turn, reply,
                         loader=getattr(editor, "load_preview", None))
        mode = "authoring"
    else:
        raise AgentRequestError("必须提供 ops，或同时提供 turn/reply")
    _INDEX.resolve(tool, opaque_ref)
    return _finalize_dto({"tool": tool, "ref": opaque_ref, "mode": mode,
            "revision": _redact(str(result["revision"]), 256),
            "before": _bounded_json(result["before"], 12 * 1024),
            "after": _bounded_json(result["after"], 12 * 1024),
            "changes": _bounded_json(result["changes"], 12 * 1024),
            "capabilities": _bounded_json(
                result.get("capabilities", {}), 12 * 1024)})
