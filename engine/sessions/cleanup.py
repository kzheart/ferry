"""Agent 会话清理的盘点与裁决账本。"""
from __future__ import annotations

import base64
import binascii
import hashlib
import json
import re
import threading
import time
from dataclasses import dataclass, field

from ..context import EngineContext
from ..errors import AgentReferenceError, AgentRequestError
from ..contracts.session_ref import is_opaque_session_ref
from .index import AgentSessionIndex, IndexedSession
from .safety import string_set, timestamp


_MAX_PAGE_SIZE = 100
_MAX_VERDICTS_PER_CALL = 100
_SCOPE_ID = re.compile(r"^[0-9a-f]{16}$")
_SCOPE_KEYS = {"agents", "projects", "updated_before"}
_VERDICTS = {"delete", "keep", "ask_user"}
_SUMMARY_FIELDS = (
    "tool", "id", "ref", "title", "dir", "updated", "created",
    "count", "size", "revision",
)
# 账本是纯内存态。TTL 对齐 operations 的 PLAN_TTL_MS：计划一旦过期，
# 支撑它的账本也不再有人引用。上限兜住 generation 频繁自增时的堆积。
_LEDGER_TTL_SECONDS = 600
_MAX_LEDGERS = 8


def is_cleanup_scope_id(value: object) -> bool:
    """scope_id 的唯一格式定义；operations 侧校验直接复用，不要另抄一份正则。"""
    return isinstance(value, str) and _SCOPE_ID.fullmatch(value) is not None


@dataclass(frozen=True)
class _UniverseRow:
    key: tuple[str, str]
    record: IndexedSession
    summary: dict
    sort_key: tuple[int, str, str]
    updated: int | None


@dataclass
class _TriageLedger:
    scope_id: str
    generation: int
    scope: dict
    rows: dict[tuple[str, str], _UniverseRow]
    ordered: tuple[_UniverseRow, ...]
    verdicts: dict[tuple[str, str], dict]
    touched_at: float = field(default_factory=time.monotonic)


def _scope_id(scope: dict, generation: int) -> str:
    canonical = json.dumps(
        scope,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    )
    return hashlib.sha256(f"{canonical}{generation}".encode()).hexdigest()[:16]


def _encode_cursor(scope_id: str, sort_key: tuple[int, str, str]) -> str:
    payload = json.dumps(
        [scope_id, *sort_key], ensure_ascii=False, separators=(",", ":"),
    )
    return base64.urlsafe_b64encode(payload.encode()).decode().rstrip("=")


def _decode_cursor(cursor: str) -> tuple[str, tuple[int, str, str]]:
    """cursor 自带 scope_id：续页只认账本，绝不重新规范化 scope。

    scope 里的 updated_before 允许写 "now-7d" 这类相对时间，每次调用都会
    解析成不同的绝对毫秒;若续页时重算 scope_id,翻页就会掉进另一本账本,
    裁决覆盖率永远凑不齐。
    """
    if not isinstance(cursor, str) or not cursor:
        raise AgentRequestError("cleanup cursor 无效", {"field": "cursor"})
    try:
        padded = cursor + "=" * (-len(cursor) % 4)
        value = json.loads(base64.urlsafe_b64decode(padded).decode())
    except (binascii.Error, ValueError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise AgentRequestError("cleanup cursor 无效", {"field": "cursor"}) from error
    if (
        not isinstance(value, list)
        or len(value) != 4
        or not is_cleanup_scope_id(value[0])
        or isinstance(value[1], bool)
        or not isinstance(value[1], int)
        or not all(isinstance(item, str) and item for item in value[2:])
    ):
        raise AgentRequestError("cleanup cursor 无效", {"field": "cursor"})
    return value[0], (value[1], value[2], value[3])


def _updated(row: dict) -> int | None:
    """会话的更新时间；解析不出来时返回 None，而不是当成 epoch 0。

    返回 0 会让时间戳损坏的会话永远落进 `updated_before` 区间,并排在列表
    末尾——既最容易被批量裁决成 delete,又最不容易被人看见。None 让这类
    会话直接不参与 updated_before 过滤。
    """
    raw = row.get("updated")
    if raw is None:
        return None
    try:
        return timestamp(raw)
    except AgentRequestError:
        return None


class CleanupService:
    """冻结一次 inventory 的候选全集，并记录其后续 triage 裁决。"""

    def __init__(self, index: AgentSessionIndex, ports: EngineContext):
        self._index = index
        self._ports = ports
        self._lock = threading.RLock()
        self._ledgers: dict[str, _TriageLedger] = {}

    def _normalize_scope(self, scope: dict | None) -> dict:
        if scope is None:
            return {}
        if not isinstance(scope, dict) or not set(scope) <= _SCOPE_KEYS:
            raise AgentRequestError(
                "cleanup scope 仅允许 agents/projects/updated_before",
                {"field": "scope"},
            )
        agents = string_set(scope.get("agents"), "scope.agents", 32, 64)
        known_agents = set(self._ports.adapters())
        unknown = sorted(agents - known_agents)
        if unknown:
            raise AgentRequestError(
                "cleanup scope 包含未知 Agent",
                {"field": "scope.agents", "unknown": unknown},
            )
        projects = {
            project.casefold()
            for project in string_set(scope.get("projects"), "scope.projects", 20, 256)
        }
        updated_before = timestamp(scope.get("updated_before"))
        normalized = {}
        if agents:
            normalized["agents"] = sorted(agents)
        if projects:
            normalized["projects"] = sorted(projects)
        if updated_before is not None:
            normalized["updated_before"] = updated_before
        return normalized

    def _snapshot(self) -> tuple[dict, list[IndexedSession], int]:
        snapshot = self._index.snapshot_with_status()
        if snapshot is None:
            self._index.refresh()
            snapshot = self._index.snapshot_with_status()
        if snapshot is None:
            raise AgentRequestError(
                "会话索引尚未就绪，请稍后重试",
                {"recovery": "retry"},
            )
        return snapshot

    @staticmethod
    def _summary(record: IndexedSession) -> _UniverseRow | None:
        session_id = record.row.get("id")
        if not isinstance(session_id, str) or not session_id:
            return None
        updated = _updated(record.row)
        summary = {
            "tool": record.tool,
            "id": session_id,
            "ref": record.opaque_ref,
            "title": str(record.row.get("title") or ""),
            "dir": str(record.row.get("dir") or ""),
            "updated": updated,
            "created": record.row.get("created"),
            "count": int(record.row.get("count") or 0),
            "size": int(record.row.get("size") or 0),
            "revision": record.revision,
        }
        return _UniverseRow(
            (record.tool, session_id),
            record,
            {field_name: summary[field_name] for field_name in _SUMMARY_FIELDS},
            (-(updated or 0), record.tool, session_id),
            updated,
        )

    @staticmethod
    def _matches(row: _UniverseRow, scope: dict) -> bool:
        if scope.get("agents") and row.record.tool not in scope["agents"]:
            return False
        if scope.get("projects"):
            project = str(row.record.row.get("dir") or "").casefold()
            if project not in scope["projects"]:
                return False
        if "updated_before" in scope:
            if row.updated is None or row.updated > scope["updated_before"]:
                return False
        return True

    def _new_ledger(
        self,
        scope: dict,
        records: list[IndexedSession],
        generation: int,
    ) -> _TriageLedger:
        candidates = [
            row for record in records
            if (row := self._summary(record)) is not None
            and self._matches(row, scope)
        ]
        ordered = tuple(sorted(candidates, key=lambda item: item.sort_key))
        rows = {row.key: row for row in ordered}
        scope_id = _scope_id(scope, generation)
        return _TriageLedger(
            scope_id, generation, scope, rows, ordered, {},
        )

    def _evict(self, keep: str | None = None) -> None:
        """必须在 self._lock 内调用。"""
        now = time.monotonic()
        for scope_id, ledger in list(self._ledgers.items()):
            if scope_id != keep and now - ledger.touched_at > _LEDGER_TTL_SECONDS:
                del self._ledgers[scope_id]
        if len(self._ledgers) <= _MAX_LEDGERS:
            return
        expendable = sorted(
            (ledger.touched_at, scope_id)
            for scope_id, ledger in self._ledgers.items()
            if scope_id != keep
        )
        for _touched, scope_id in expendable[:len(self._ledgers) - _MAX_LEDGERS]:
            del self._ledgers[scope_id]

    def _page(
        self, ledger: _TriageLedger, cursor_key: tuple[int, str, str] | None,
        page_size: int,
    ) -> dict:
        """必须在 self._lock 内调用。"""
        start = 0
        if cursor_key is not None:
            matches = [
                index for index, row in enumerate(ledger.ordered)
                if row.sort_key == cursor_key
            ]
            if not matches:
                raise AgentRequestError(
                    "cleanup cursor 不属于当前 inventory",
                    {"field": "cursor", "recovery": "inventory"},
                )
            start = matches[0] + 1
        page_rows = ledger.ordered[start:start + page_size]
        end = start + len(page_rows)
        next_cursor = (
            _encode_cursor(ledger.scope_id, page_rows[-1].sort_key)
            if end < len(ledger.ordered) and page_rows
            else None
        )
        ledger.touched_at = time.monotonic()
        return {
            "scope_id": ledger.scope_id,
            "scope": dict(ledger.scope),
            "generation": ledger.generation,
            "total": len(ledger.ordered),
            "page": [dict(row.summary) for row in page_rows],
            "next_cursor": next_cursor,
            "covered": len(ledger.verdicts),
        }

    def inventory(
        self,
        scope: dict | None = None,
        cursor: str | None = None,
        page_size: int = _MAX_PAGE_SIZE,
        scope_id: str | None = None,
    ) -> dict:
        """盘点候选会话。

        首次调用传 scope，返回体回吐规范化后的绝对 scope 与 scope_id；续页
        传 cursor（或显式 scope_id）直查已冻结的账本，不再重新规范化 scope。
        """
        if (
            isinstance(page_size, bool)
            or not isinstance(page_size, int)
            or not 1 <= page_size <= _MAX_PAGE_SIZE
        ):
            raise AgentRequestError(
                f"page_size 必须是 1 到 {_MAX_PAGE_SIZE} 的整数",
                {"field": "page_size"},
            )
        if scope_id is not None and not is_cleanup_scope_id(scope_id):
            raise AgentRequestError(
                "cleanup scope_id 无效",
                {"field": "scope_id", "recovery": "inventory"},
            )
        cursor_key = None
        if cursor is not None:
            cursor_scope_id, cursor_key = _decode_cursor(cursor)
            if scope_id is not None and scope_id != cursor_scope_id:
                raise AgentRequestError(
                    "cleanup cursor 与 scope_id 不匹配",
                    {"field": "cursor", "recovery": "inventory"},
                )
            scope_id = cursor_scope_id

        if scope_id is not None:
            ledger = self._ledger(scope_id)
            self._require_fresh(scope_id, ledger)
            with self._lock:
                return self._page(ledger, cursor_key, page_size)

        normalized = self._normalize_scope(scope)
        _tools, records, generation = self._snapshot()
        fresh_id = _scope_id(normalized, generation)
        with self._lock:
            ledger = self._ledgers.get(fresh_id)
            if ledger is None:
                ledger = self._new_ledger(normalized, records, generation)
                self._ledgers[fresh_id] = ledger
            self._evict(keep=fresh_id)
            return self._page(ledger, None, page_size)

    def _ledger(self, scope_id: str) -> _TriageLedger:
        if not is_cleanup_scope_id(scope_id):
            raise AgentRequestError(
                "cleanup scope_id 无效",
                {"field": "scope_id", "recovery": "inventory"},
            )
        with self._lock:
            ledger = self._ledgers.get(scope_id)
            if ledger is not None:
                ledger.touched_at = time.monotonic()
        if ledger is None:
            raise AgentRequestError(
                "cleanup scope 已失效，请重新 inventory",
                {"scope_id": scope_id, "reason": "unknown_scope", "recovery": "inventory"},
            )
        return ledger

    def _require_fresh(self, scope_id: str, ledger: _TriageLedger) -> None:
        if self._index.generation != ledger.generation:
            raise AgentRequestError(
                "cleanup scope 在 inventory 后已变化，请重新 inventory",
                {"scope_id": scope_id, "reason": "stale_generation", "recovery": "inventory"},
            )

    def stale(self, scope_id: str) -> bool:
        try:
            ledger = self._ledger(scope_id)
        except AgentRequestError:
            return True
        return self._index.generation != ledger.generation

    def _validated_verdict(self, item: object, ledger: _TriageLedger) -> tuple:
        if not isinstance(item, dict) or not set(item) <= {"tool", "ref", "verdict", "reason"}:
            raise AgentRequestError("triage verdict 字段无效", {"field": "verdicts"})
        if set(item) - {"tool", "ref", "verdict"} and "reason" not in item:
            raise AgentRequestError("triage verdict 字段无效", {"field": "verdicts"})
        tool = item.get("tool")
        ref = item.get("ref")
        verdict = item.get("verdict")
        if not isinstance(tool, str) or tool not in set(self._ports.adapters()):
            raise AgentRequestError("triage verdict 的 tool 无效", {"field": "verdicts.tool"})
        if not is_opaque_session_ref(ref):
            raise AgentRequestError("triage verdict 的 ref 无效", {"field": "verdicts.ref"})
        if verdict not in _VERDICTS:
            raise AgentRequestError(
                "triage verdict 仅允许 delete/keep/ask_user",
                {"field": "verdicts.verdict"},
            )
        reason = item.get("reason")
        if "reason" in item and (
            not isinstance(reason, str) or len(reason) > 300
        ):
            raise AgentRequestError(
                "triage reason 必须是不超过 300 字符的字符串",
                {"field": "verdicts.reason"},
            )
        try:
            record = self._index.resolve(tool, ref, pin_content=False)
        except AgentReferenceError as error:
            raise AgentRequestError(
                "triage ref 不可解析，请重新 inventory",
                {"tool": tool, "ref": ref, "reason": "unknown_ref", "recovery": "inventory"},
            ) from error
        key = (tool, record.row.get("id"))
        if key not in ledger.rows:
            raise AgentRequestError(
                "triage ref 不在当前 inventory scope",
                {"tool": tool, "ref": ref, "reason": "out_of_scope"},
            )
        return key, verdict, "reason" in item, reason

    def triage(self, scope_id: str, verdicts: list[dict]) -> dict:
        ledger = self._ledger(scope_id)
        self._require_fresh(scope_id, ledger)
        if not isinstance(verdicts, list) or len(verdicts) > _MAX_VERDICTS_PER_CALL:
            raise AgentRequestError(
                f"verdicts 必须是至多 {_MAX_VERDICTS_PER_CALL} 项的数组",
                {"field": "verdicts"},
            )
        # 先全量校验再统一写入：逐条边校验边写会在中途报错时留下半批裁决，
        # 而调用方看到的是一个失败响应,会以为整批都没生效。
        validated = [self._validated_verdict(item, ledger) for item in verdicts]
        with self._lock:
            for key, verdict, has_reason, reason in validated:
                prior = ledger.verdicts.get(key, {})
                ledger.verdicts[key] = {
                    "verdict": verdict,
                    "reason": reason if has_reason else prior.get("reason"),
                }
            remaining = [
                {
                    "tool": row.summary["tool"],
                    "id": row.summary["id"],
                    "title": row.summary["title"],
                }
                for row in ledger.ordered
                if row.key not in ledger.verdicts
            ][:10]
            return {
                "covered": len(ledger.verdicts),
                "total": len(ledger.ordered),
                "remaining_sample": remaining,
            }

    def check_nomination(self, scope_id: str, resolved: list[tuple]) -> None:
        ledger = self._ledger(scope_id)
        self._require_fresh(scope_id, ledger)
        if not isinstance(resolved, list):
            raise AgentRequestError("cleanup targets 解析结果非法")
        with self._lock:
            missing = len(ledger.rows) - len(ledger.verdicts)
            if missing:
                raise AgentRequestError(
                    f"cleanup triage 尚有 {missing} 条会话未裁决",
                    {
                        "scope_id": scope_id,
                        "covered": len(ledger.verdicts),
                        "total": len(ledger.rows),
                        "missing": missing,
                        "recovery": "triage_remaining",
                    },
                )
            seen = set()
            for item in resolved:
                if (
                    not isinstance(item, tuple)
                    or len(item) != 2
                    or not all(isinstance(value, str) and value for value in item)
                ):
                    raise AgentRequestError("cleanup target 解析结果非法")
                if item in seen:
                    raise AgentRequestError("cleanup targets 不允许重复")
                seen.add(item)
                if item not in ledger.rows:
                    raise AgentRequestError(
                        "cleanup target 不在当前 inventory scope",
                        {"tool": item[0], "session_id": item[1], "reason": "out_of_scope"},
                    )
                verdict = ledger.verdicts.get(item, {}).get("verdict")
                if verdict != "delete":
                    raise AgentRequestError(
                        "cleanup target 没有 delete 裁决",
                        {"tool": item[0], "session_id": item[1], "verdict": verdict},
                    )

    def coverage(self, scope_id: str) -> dict:
        ledger = self._ledger(scope_id)
        with self._lock:
            return {
                "covered": len(ledger.verdicts),
                "total": len(ledger.rows),
                "scope": scope_id,
            }

    def verdict(self, scope_id: str, key: tuple[str, str]) -> dict | None:
        ledger = self._ledger(scope_id)
        with self._lock:
            value = ledger.verdicts.get(key)
            return dict(value) if value is not None else None
