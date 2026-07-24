"""Opaque 会话引用、消息 locator 与 revision 索引。"""
from __future__ import annotations

import hashlib
import json
import secrets
import threading
from dataclasses import dataclass
from pathlib import Path

from ..context import EngineContext
from ..contracts.session_ref import is_opaque_session_ref
from ..errors import AgentReferenceError, LocatorStaleError


def _revision(
    tool: str,
    canonical_ref: str,
    row: dict,
    identity: tuple | str | None = None,
) -> str:
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
    if (
        (before.st_dev, before.st_ino, before.st_mtime_ns, before.st_size)
        != (after.st_dev, after.st_ino, after.st_mtime_ns, after.st_size)
    ):
        raise AgentReferenceError("会话在计算 revision 时发生变化")
    return (
        after.st_dev,
        after.st_ino,
        after.st_mtime_ns,
        after.st_size,
        digest.hexdigest(),
    )


def _agent_fingerprint(browser, ref: str):
    marker = getattr(browser, "agent_fingerprint", None)
    return (marker or browser.fingerprint)(ref)


class AgentSessionIndex:
    def __init__(self, ports: EngineContext):
        self._ports = ports
        self._by_opaque: dict[str, IndexedSession] = {}
        self._opaque_by_key: dict[tuple[str, str, str], str] = {}
        self._messages_by_opaque: dict[str, IndexedMessage] = {}
        self._opaque_by_message_key: dict[tuple[str, str, str], str] = {}
        self._lock = threading.RLock()

    @property
    def ports(self) -> EngineContext:
        return self._ports

    def refresh(self) -> list[IndexedSession]:
        cache = self._ports.cache_factory()
        scanned = []
        for tool_name in self._ports.adapters():
            adapter = self._ports.adapter(tool_name)
            scanned.extend(
                (tool_name, adapter, row)
                for row in adapter.browser.scan(cache)
            )
        cache.flush()
        return self.index_rows(scanned)

    def index_rows(self, scanned) -> list[IndexedSession]:
        records: list[IndexedSession] = []
        active: set[str] = set()
        with self._lock:
            for tool_name, adapter, row in scanned:
                canonical, root, path_backed, identity = self._canonicalize(
                    adapter, row,
                )
                if canonical is None:
                    continue
                revision = _revision(tool_name, canonical, row, identity)
                key = (tool_name, canonical, revision)
                opaque = self._opaque_by_key.get(key)
                if opaque is None:
                    opaque = "fsr_" + secrets.token_urlsafe(18)
                    self._opaque_by_key[key] = opaque
                record = IndexedSession(
                    opaque,
                    tool_name,
                    canonical,
                    root,
                    path_backed,
                    dict(row),
                    revision,
                    identity,
                )
                self._by_opaque[opaque] = record
                active.add(opaque)
                records.append(record)
            for opaque in set(self._by_opaque) - active:
                stale = self._by_opaque.pop(opaque)
                self._opaque_by_key.pop(
                    (stale.tool, stale.canonical_ref, stale.revision),
                    None,
                )
                stale_messages = [
                    locator
                    for locator, message in self._messages_by_opaque.items()
                    if message.session_ref == opaque
                ]
                for locator in stale_messages:
                    message = self._messages_by_opaque.pop(locator)
                    self._opaque_by_message_key.pop(
                        (
                            message.session_ref,
                            message.native_locator,
                            message.role,
                        ),
                        None,
                    )
        return records

    def resolve(self, tool: str, opaque_ref: str) -> IndexedSession:
        if not is_opaque_session_ref(opaque_ref):
            raise AgentReferenceError("ref 不是 Engine 签发的 opaque ref")
        with self._lock:
            record = self._by_opaque.get(opaque_ref)
        if record is None or record.tool != tool:
            raise AgentReferenceError(
                "ref 不在当前扫描索引中",
                {"tool": tool},
            )
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
            adapter_ref = browser.resolve_ref(str(resolved))
            if Path(adapter_ref).resolve(strict=True) != resolved:
                raise AgentReferenceError("adapter 未能规范解析 ref")
        else:
            browser = self._ports.adapter(tool).browser
            fingerprint = _agent_fingerprint(browser, record.canonical_ref)
            if fingerprint is None or fingerprint != record.source_identity:
                raise AgentReferenceError("ref 在扫描后已变化，请重新搜索")
        return record

    def issue_message_locator(
        self,
        record: IndexedSession,
        native_locator: str,
        role: str,
        editable: bool,
    ) -> str:
        if not native_locator or len(native_locator) > 512:
            raise AgentReferenceError("消息缺少可编辑定位信息")
        key = (record.opaque_ref, native_locator, role)
        with self._lock:
            opaque = self._opaque_by_message_key.get(key)
            if opaque is None:
                opaque = "fml_" + secrets.token_urlsafe(18)
                self._opaque_by_message_key[key] = opaque
            self._messages_by_opaque[opaque] = IndexedMessage(
                opaque,
                record.opaque_ref,
                record.tool,
                record.revision,
                native_locator,
                role,
                editable,
            )
        return opaque

    def resolve_message_locator(
        self,
        record: IndexedSession,
        opaque_locator: str,
    ) -> IndexedMessage:
        hint = (
            "重新调用 ferry_get_session_context，并原样使用 messages[].locator"
        )
        if (
            not isinstance(opaque_locator, str)
            or not opaque_locator.startswith("fml_")
        ):
            raise AgentReferenceError(
                "locator 不是 Engine 签发的消息引用",
                {"field": "locator", "hint": hint},
            )
        with self._lock:
            message = self._messages_by_opaque.get(opaque_locator)
        if (
            message is None
            or message.session_ref != record.opaque_ref
            or message.tool != record.tool
            or message.revision != record.revision
        ):
            raise LocatorStaleError(
                "消息引用已失效或不属于当前会话",
                {"field": "locator", "hint": hint},
            )
        return message

    @staticmethod
    def _canonicalize(adapter, row: dict) -> tuple[
        str | None,
        str | None,
        bool,
        tuple | str | None,
    ]:
        native = adapter.browser.canonicalize(row)
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
                fingerprint = _agent_fingerprint(adapter.browser, str(path))
                if fingerprint is None:
                    return None, None, True, None
                identity = (_path_identity(path), fingerprint)
            except (OSError, ValueError, AgentReferenceError):
                return None, None, True, None
            return str(path), str(root), True, identity
        if adapter.browser.resolve_ref(native.canonical_ref) != native.canonical_ref:
            return None, None, False, None
        fingerprint = _agent_fingerprint(
            adapter.browser,
            native.canonical_ref,
        )
        if fingerprint is None:
            return None, None, False, None
        return native.canonical_ref, None, False, fingerprint
