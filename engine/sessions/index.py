"""Opaque 会话引用、消息 locator 与 revision 索引。"""
from __future__ import annotations

import hashlib
import json
import os
import secrets
import threading
from concurrent.futures import ThreadPoolExecutor
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


# 会话被写入过就会换发 ref。agent 只能看到结构化错误的 params,
# 所以恢复办法必须以数据形式给出,否则模型只会认为这个会话读不了。
_REF_RECOVERY_HINT = (
    "the session changed or was re-indexed; call session_search again and use "
    "the fresh fsr_ ref from the results"
)
_DIGEST_CACHE_LIMIT = 50_000
_PARALLEL_CANONICALIZE_THRESHOLD = 64
_CANONICALIZE_WORKERS = min(8, (os.cpu_count() or 4))


def _path_identity(path: Path, digest_cache: dict | None = None) -> tuple:
    """会话文件身份 = stat 四元组 + 内容摘要。

    `digest_cache` 只在全量扫描时传入:上千个会话每次搜索都重算一遍内容摘要
    要读掉整个会话库,固定给每次工具调用加上秒级延迟。stat 四元组没变时先
    复用摘要;真正读取或编辑某个会话时 `resolve()` 一定不带缓存重算,并在发现
    「stat 没变但内容变了」时删掉该缓存项,下一次扫描就会重新哈希并换发 ref。
    """
    if digest_cache is not None:
        probe = path.stat()
        key = (probe.st_dev, probe.st_ino, probe.st_mtime_ns, probe.st_size)
        cached = digest_cache.get(key)
        if cached is not None:
            return (*key, cached)
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
    identity = (
        after.st_dev,
        after.st_ino,
        after.st_mtime_ns,
        after.st_size,
        digest.hexdigest(),
    )
    if digest_cache is not None:
        if len(digest_cache) >= _DIGEST_CACHE_LIMIT:
            digest_cache.clear()
        digest_cache[identity[:4]] = identity[4]
    return identity


def _is_within(path: str, root: str) -> bool:
    """等价于 Path(path).is_relative_to(root),按已规范化的路径串比较。"""
    if path == root:
        return True
    prefix = root if root.endswith(os.sep) else root + os.sep
    return path.startswith(prefix)


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
        self._digest_cache: dict[tuple, str] = {}
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
        # 规范化要给每个会话文件算一遍内容摘要,几千个会话串行做会让每次
        # 搜索/用量调用固定多花两秒。摘要之间互不依赖,且 hashlib 与文件读取
        # 都会释放 GIL,所以先并行算完,再在锁内串行做签发与淘汰。
        canonical_rows = self._canonicalize_all(scanned)
        with self._lock:
            for (tool_name, _adapter, row), resolved in zip(
                scanned, canonical_rows,
            ):
                canonical, root, path_backed, identity = resolved
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
            # 恢复办法要放进 params:agent 只看得到结构化错误,看不到这句中文。
            raise AgentReferenceError(
                "ref 不在当前扫描索引中",
                {"tool": tool, "reason": "unknown_ref",
                 "recovery": _REF_RECOVERY_HINT},
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
                # 摘要缓存可能命中了「stat 没变但内容变了」的旧值:踢掉它,
                # 下一次扫描才会重新哈希并换发新的 ref。
                with self._lock:
                    self._digest_cache.pop(identity[0][:4], None)
                raise AgentReferenceError(
                    "ref 在扫描后已变化，请重新搜索",
                    {"tool": tool, "reason": "session_changed",
                     "recovery": _REF_RECOVERY_HINT},
                )
            adapter_ref = browser.resolve_ref(str(resolved))
            if Path(adapter_ref).resolve(strict=True) != resolved:
                raise AgentReferenceError("adapter 未能规范解析 ref")
        else:
            browser = self._ports.adapter(tool).browser
            fingerprint = _agent_fingerprint(browser, record.canonical_ref)
            if fingerprint is None or fingerprint != record.source_identity:
                raise AgentReferenceError(
                    "ref 在扫描后已变化，请重新搜索",
                    {"tool": tool, "reason": "session_changed",
                     "recovery": _REF_RECOVERY_HINT},
                )
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

    def _canonicalize_all(self, scanned) -> list[tuple]:
        rows = list(scanned)
        if len(rows) < _PARALLEL_CANONICALIZE_THRESHOLD:
            return [self._canonicalize(adapter, row) for _, adapter, row in rows]
        with ThreadPoolExecutor(max_workers=_CANONICALIZE_WORKERS) as pool:
            return list(
                pool.map(
                    lambda item: self._canonicalize(item[1], item[2]),
                    rows,
                )
            )

    def _canonicalize(self, adapter, row: dict) -> tuple[
        str | None,
        str | None,
        bool,
        tuple | str | None,
    ]:
        native = adapter.browser.canonicalize(row)
        if native is None:
            return None, None, False, None
        if native.path_backed:
            # 走 os.path 而不是 pathlib:语义(realpath + 前缀归属)完全一致,
            # 但少掉数万次 Path 对象构造,全量扫描省下的时间是可观的。
            try:
                root = os.path.realpath(native.root or "", strict=True)
                path = os.path.realpath(native.canonical_ref, strict=True)
            except OSError:
                return None, None, True, None
            if not _is_within(path, root):
                return None, None, True, None
            try:
                fingerprint = _agent_fingerprint(adapter.browser, path)
                if fingerprint is None:
                    return None, None, True, None
                identity = (
                    _path_identity(Path(path), self._digest_cache),
                    fingerprint,
                )
            except (OSError, ValueError, AgentReferenceError):
                return None, None, True, None
            return path, root, True, identity
        if adapter.browser.resolve_ref(native.canonical_ref) != native.canonical_ref:
            return None, None, False, None
        fingerprint = _agent_fingerprint(
            adapter.browser,
            native.canonical_ref,
        )
        if fingerprint is None:
            return None, None, False, None
        return native.canonical_ref, None, False, fingerprint
