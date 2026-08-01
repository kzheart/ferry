"""Agent 会话清理的盘点与裁决账本。"""
from __future__ import annotations

import base64
import binascii
import hashlib
import json
import re
import threading
from dataclasses import dataclass

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


@dataclass(frozen=True)
class _UniverseRow:
    key: tuple[str, str]
    record: IndexedSession
    summary: dict
    sort_key: tuple[int, str, str]


@dataclass
class _TriageLedger:
    scope_id: str
    generation: int
    scope: dict
    rows: dict[tuple[str, str], _UniverseRow]
    ordered: tuple[_UniverseRow, ...]
    verdicts: dict[tuple[str, str], dict]


def _scope_id(scope: dict, generation: int) -> str:
    canonical = json.dumps(
        scope,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    )
    return hashlib.sha256(f"{canonical}{generation}".encode()).hexdigest()[:16]


def _encode_cursor(sort_key: tuple[int, str, str]) -> str:
    payload = json.dumps(list(sort_key), ensure_ascii=False, separators=(",", ":"))
    return base64.urlsafe_b64encode(payload.encode()).decode().rstrip("=")


def _decode_cursor(cursor: str) -> tuple[int, str, str]:
    if not isinstance(cursor, str) or not cursor:
        raise AgentRequestError("cleanup cursor 无效", {"field": "cursor"})
    try:
        padded = cursor + "=" * (-len(cursor) % 4)
        value = json.loads(base64.urlsafe_b64decode(padded).decode())
    except (binascii.Error, ValueError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise AgentRequestError("cleanup cursor 无效", {"field": "cursor"}) from error
    if (
        not isinstance(value, list)
        or len(value) != 3
        or isinstance(value[0], bool)
        or not isinstance(value[0], int)
        or not all(isinstance(item, str) and item for item in value[1:])
    ):
        raise AgentRequestError("cleanup cursor 无效", {"field": "cursor"})
    return value[0], value[1], value[2]


def _updated(row: dict) -> int:
    raw = row.get("updated")
    try:
        return int(timestamp(raw) or 0)
    except AgentRequestError:
        return 0


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
            {field: summary[field] for field in _SUMMARY_FIELDS},
            (-updated, record.tool, session_id),
        )

    @staticmethod
    def _matches(row: _UniverseRow, scope: dict) -> bool:
        if scope.get("agents") and row.record.tool not in scope["agents"]:
            return False
        if scope.get("projects"):
            project = str(row.record.row.get("dir") or "").casefold()
            if project not in scope["projects"]:
                return False
        if "updated_before" in scope and row.summary["updated"] > scope["updated_before"]:
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

    def inventory(
        self,
        scope: dict | None,
        cursor: str | None = None,
        page_size: int = _MAX_PAGE_SIZE,
    ) -> dict:
        normalized = self._normalize_scope(scope)
        if (
            isinstance(page_size, bool)
            or not isinstance(page_size, int)
            or not 1 <= page_size <= _MAX_PAGE_SIZE
        ):
            raise AgentRequestError(
                f"page_size 必须是 1 到 {_MAX_PAGE_SIZE} 的整数",
                {"field": "page_size"},
            )
        _tools, records, generation = self._snapshot()
        scope_id = _scope_id(normalized, generation)
        with self._lock:
            ledger = self._ledgers.get(scope_id)
            if ledger is None:
                ledger = self._new_ledger(normalized, records, generation)
                self._ledgers[scope_id] = ledger
            start = 0
            if cursor is not None:
                cursor_key = _decode_cursor(cursor)
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
                _encode_cursor(page_rows[-1].sort_key)
                if end < len(ledger.ordered) and page_rows
                else None
            )
            return {
                "scope_id": ledger.scope_id,
                "generation": ledger.generation,
                "total": len(ledger.ordered),
                "page": [dict(row.summary) for row in page_rows],
                "next_cursor": next_cursor,
                "covered": len(ledger.verdicts),
            }

    def _ledger(self, scope_id: str) -> _TriageLedger:
        if not isinstance(scope_id, str) or not _SCOPE_ID.fullmatch(scope_id):
            raise AgentRequestError(
                "cleanup scope_id 无效",
                {"field": "scope_id", "recovery": "inventory"},
            )
        with self._lock:
            ledger = self._ledgers.get(scope_id)
        if ledger is None:
            raise AgentRequestError(
                "cleanup scope 已失效，请重新 inventory",
                {"scope_id": scope_id, "reason": "unknown_scope", "recovery": "inventory"},
            )
        return ledger

    def stale(self, scope_id: str) -> bool:
        try:
            ledger = self._ledger(scope_id)
        except AgentRequestError:
            return True
        return self._index.generation != ledger.generation

    def triage(self, scope_id: str, verdicts: list[dict]) -> dict:
        ledger = self._ledger(scope_id)
        if self.stale(scope_id):
            raise AgentRequestError(
                "cleanup scope 在 inventory 后已变化，请重新 inventory",
                {"scope_id": scope_id, "reason": "stale_generation", "recovery": "inventory"},
            )
        if not isinstance(verdicts, list) or len(verdicts) > _MAX_VERDICTS_PER_CALL:
            raise AgentRequestError(
                f"verdicts 必须是至多 {_MAX_VERDICTS_PER_CALL} 项的数组",
                {"field": "verdicts"},
            )
        for item in verdicts:
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
            prior = ledger.verdicts.get(key, {})
            ledger.verdicts[key] = {
                "verdict": verdict,
                "reason": reason if "reason" in item else prior.get("reason"),
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
        if self.stale(scope_id):
            raise AgentRequestError(
                "cleanup scope 在 inventory 后已变化，请重新 inventory",
                {"scope_id": scope_id, "reason": "stale_generation", "recovery": "inventory"},
            )
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
        if not isinstance(resolved, list):
            raise AgentRequestError("cleanup targets 解析结果非法")
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
