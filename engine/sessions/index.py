"""Opaque 会话引用、消息 locator 与 revision 索引。"""
from __future__ import annotations

import hashlib
import json
import logging
import os
import secrets
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from pathlib import Path
from typing import NamedTuple

from ..context import EngineContext
from ..contracts.session_ref import is_opaque_session_ref
from ..errors import AgentReferenceError, LocatorStaleError
from ..system.paths import is_within
from .scan_progress import TRACKER

log = logging.getLogger(__name__)


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


class _IdentityStat(NamedTuple):
    """把 identity 四元组当 stat 用,避免写回摘要时再 stat 一次引入竞态。"""

    st_dev: int
    st_ino: int
    st_mtime_ns: int
    st_size: int


@dataclass(frozen=True)
class IndexedSession:
    opaque_ref: str
    tool: str
    canonical_ref: str
    root: str | None
    storage_kind: str
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


# ref 是稳定的会话句柄,但钉内容的读取/编辑路径在会话被写入后需要重扫。
# agent 只能看到结构化错误的 params,所以恢复办法必须以数据形式给出,
# 否则模型只会认为这个会话读不了。
_REF_RECOVERY_HINT = (
    "the session changed since the last scan; call session_search again to "
    "re-index it, then retry with the ref from the results"
)
_DIGEST_CACHE_LIMIT = 50_000
_PARALLEL_CANONICALIZE_THRESHOLD = 64
_CANONICALIZE_WORKERS = min(8, (os.cpu_count() or 4))


def _path_identity(
    path: Path,
    digest_cache: dict | None = None,
    digest_store=None,
) -> tuple:
    """会话文件身份 = stat 四元组 + 内容摘要。

    `digest_cache` 只在全量扫描时传入:上千个会话每次搜索都重算一遍内容摘要
    要读掉整个会话库,固定给每次工具调用加上秒级延迟。stat 四元组没变时先
    复用摘要;真正读取或编辑某个会话时 `resolve()` 一定不带缓存重算,并在发现
    「stat 没变但内容变了」时删掉该缓存项,下一次扫描就会重新哈希并换发 ref。

    `digest_store`(ScanCache)是同一份摘要的跨进程缓存:进程内 dict 冷启动时
    永远是空的,没有它每次开机都要把整个会话库读一遍算 SHA-256。校验四元组
    与进程内缓存完全一致,所以命中的安全语义相同。
    """
    if digest_cache is not None:
        probe = path.stat()
        key = (probe.st_dev, probe.st_ino, probe.st_mtime_ns, probe.st_size)
        cached = digest_cache.get(key)
        if cached is None and digest_store is not None:
            cached = digest_store.get_digest(path, probe)
            if cached is not None:
                if len(digest_cache) >= _DIGEST_CACHE_LIMIT:
                    digest_cache.clear()
                digest_cache[key] = cached
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
        raise AgentReferenceError(
            "会话在计算 revision 时发生变化",
            {"reason": "session_changed", "recovery": _REF_RECOVERY_HINT},
        )
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
        if digest_store is not None:
            digest_store.put_digest(path, after, identity[4])
    return identity


def _agent_fingerprint(browser, ref: str):
    marker = getattr(browser, "agent_fingerprint", None)
    return (marker or browser.fingerprint)(ref)


def _scan_fingerprint(browser, ref: str):
    """扫描路径的指纹:adapter 可提供容忍旧快照的变体,避免活跃存储的
    每次写入都把全量扫描拖进同步重建;resolve 的钉内容校验仍走严格版。"""
    marker = getattr(browser, "scan_fingerprint", None)
    if marker is not None:
        return marker(ref)
    return _agent_fingerprint(browser, ref)


def _directory_identity(
    path: Path,
    browser,
    digest_cache: dict | None = None,
    digest_store=None,
) -> tuple:
    member_provider = getattr(browser, "authoritative_members", None)
    if member_provider is None:
        raise AgentReferenceError("目录会话未声明权威成员")
    members = member_provider(str(path))
    if not isinstance(members, (list, tuple)) or not members:
        raise AgentReferenceError("目录会话缺少权威成员")
    identities = []
    for raw_member in members:
        member = Path(raw_member)
        if not member.is_absolute():
            member = path / member
        resolved = member.resolve(strict=True)
        if not resolved.is_file() or not resolved.is_relative_to(path):
            raise AgentReferenceError("目录会话权威成员超出 bundle")
        identities.append((
            str(resolved.relative_to(path)),
            _path_identity(resolved, digest_cache, digest_store),
        ))
    return tuple(sorted(identities))


class _RefreshFlight:
    """一次进行中的全量刷新:后到者在 done 上等待并复用结果。"""

    def __init__(self):
        self.done = threading.Event()
        self.result: tuple[dict, list[IndexedSession]] | None = None
        self.error: BaseException | None = None


class AgentSessionIndex:
    def __init__(self, ports: EngineContext):
        self._ports = ports
        self._by_opaque: dict[str, IndexedSession] = {}
        self._opaque_by_key: dict[tuple[str, str], str] = {}
        self._messages_by_opaque: dict[str, IndexedMessage] = {}
        self._opaque_by_message_key: dict[tuple[str, str, str], str] = {}
        self._digest_cache: dict[tuple, str] = {}
        self._lock = threading.RLock()
        self._refresh_lock = threading.Lock()
        self._refresh_inflight: _RefreshFlight | None = None

    @property
    def ports(self) -> EngineContext:
        return self._ports

    def refresh(self) -> list[IndexedSession]:
        return self.refresh_with_status()[1]

    def refresh_with_status(self) -> tuple[dict, list[IndexedSession]]:
        """全量扫库并重建索引;并发调用单飞合并。

        启动预热与 UI 首扫、agent 搜索与 usage 都会全量刷新,工作完全
        相同却各跑一遍。后到者等待先行者并复用其结果:至多陈旧一轮扫描,
        内容一致性由 pin_content 与 locator 的 revision 校验兜底。
        """
        with self._refresh_lock:
            flight = self._refresh_inflight
            leader = flight is None
            if leader:
                flight = self._refresh_inflight = _RefreshFlight()
        if not leader:
            flight.done.wait()
            if flight.error is not None:
                raise flight.error
            return flight.result
        try:
            flight.result = self._scan_all()
            return flight.result
        except BaseException as error:
            flight.error = error
            raise
        finally:
            with self._refresh_lock:
                self._refresh_inflight = None
            flight.done.set()

    def _scan_all(self) -> tuple[dict, list[IndexedSession]]:
        tools: dict[str, dict] = {}
        scanned = []
        cache = self._ports.cache_factory()
        names = list(self._ports.adapters())
        TRACKER.begin(names)
        started = time.monotonic()
        try:
            for name in names:
                tool = self._ports.adapter(name)
                source_path = getattr(
                    getattr(tool, "manifest", None), "source_path", None,
                )
                TRACKER.start_tool(name)
                tool_started = time.monotonic()
                try:
                    rows = tool.browser.scan(cache)
                    scanned.extend((name, tool, row) for row in rows)
                    tools[name] = {
                        "ok": True, "count": len(rows), "path": source_path,
                    }
                    log.info("扫描 %s: %d 条会话 耗时=%.1fs",
                             name, len(rows),
                             time.monotonic() - tool_started)
                except Exception as error:  # noqa: BLE001 - 单工具失败不拖垮全量
                    tools[name] = {
                        "ok": False, "error": str(error)[:200],
                        "path": source_path,
                    }
                    log.warning("扫描 %s 失败 耗时=%.1fs: %s",
                                name, time.monotonic() - tool_started, error)
                finally:
                    TRACKER.finish_tool(name)
            TRACKER.finalize()
            cache.flush()
            index_started = time.monotonic()
            records = self.index_rows(scanned)
            log.info("索引 %d 条会话 耗时=%.1fs 全程=%.1fs", len(records),
                     time.monotonic() - index_started,
                     time.monotonic() - started)
            # 扫描主体完成后才做各 adapter 的维护(如 opencode 指纹后台
            # 重建),避免维护任务与扫描本身争抢 CPU/GIL。
            for name in names:
                maintenance = getattr(
                    self._ports.adapter(name).browser,
                    "post_scan_maintenance", None,
                )
                if maintenance is None:
                    continue
                try:
                    maintenance()
                except Exception:  # noqa: BLE001 - 维护失败不影响扫描结果
                    log.exception("扫描收尾维护失败: %s", name)
            return tools, records
        finally:
            TRACKER.end()

    def _digest_store(self):
        """摘要的跨进程缓存。测试与旧上下文的 cache 不一定支持,拿不到就算了。"""
        try:
            cache = self._ports.cache_factory()
        except Exception:
            return None
        return cache if hasattr(cache, "get_digest") else None

    def _store_identity_digests(self, path: Path, storage_kind, identity):
        store = self._digest_store()
        if store is None:
            return
        if storage_kind == "file":
            entries = [(path, identity[0])]
        else:
            entries = [(path / relative, member) for relative, member in identity]
        for target, member in entries:
            store.put_digest(target, _IdentityStat(*member[:4]), member[4])
        store.flush()

    def index_rows(self, scanned) -> list[IndexedSession]:
        records: list[IndexedSession] = []
        active: set[str] = set()
        # 规范化要给每个会话文件算一遍内容摘要,几千个会话串行做会让每次
        # 搜索/用量调用固定多花两秒。摘要之间互不依赖,且 hashlib 与文件读取
        # 都会释放 GIL,所以先并行算完,再在锁内串行做签发与淘汰。
        digest_store = self._digest_store()
        canonical_rows = self._canonicalize_all(scanned, digest_store)
        if digest_store is not None:
            digest_store.flush()
        with self._lock:
            for (tool_name, _adapter, row), resolved in zip(
                scanned, canonical_rows,
            ):
                canonical, root, storage_kind, identity = resolved
                if canonical is None:
                    continue
                revision = _revision(tool_name, canonical, row, identity)
                # ref 按 (tool, canonical) 签发:它是会话的稳定句柄,内容变化
                # 只更新记录的 revision/identity,不换发 ref。UI 拿着上一轮
                # 扫描的 ref 也永远能解析;内容一致性由 pin_content 与
                # locator 的 revision 校验兜住,而不是靠 ref 轮换。
                key = (tool_name, canonical)
                opaque = self._opaque_by_key.get(key)
                if opaque is None:
                    opaque = "fsr_" + secrets.token_urlsafe(18)
                    self._opaque_by_key[key] = opaque
                record = IndexedSession(
                    opaque,
                    tool_name,
                    canonical,
                    root,
                    storage_kind,
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
                    (stale.tool, stale.canonical_ref),
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

    def resolve(
        self,
        tool: str,
        opaque_ref: str,
        *,
        pin_content: bool = True,
    ) -> IndexedSession:
        """把 opaque ref 换回索引记录。

        `pin_content=True`（Agent 读取与编辑路径）要求会话内容与签发时
        一字未变,否则报 session_changed 逼 agent 重新搜索——编辑安全依赖它。
        `pin_content=False`（UI 只读浏览）只做路径归属与存在性校验:活跃会话
        随时在被 CLI 追加写入,按最新内容展示正是期望行为,而且省掉了每次
        点开都要对整个会话文件做两遍 sha256 的开销。
        """
        if not is_opaque_session_ref(opaque_ref):
            raise AgentReferenceError("ref 不是 Engine 签发的 opaque ref")
        with self._lock:
            record = self._by_opaque.get(opaque_ref)
        if record is not None and record.tool != tool:
            # ref 已能唯一定位会话,tool 配错是 agent 高频笔误;报
            # unknown_ref 会误导它重新搜索,这里直接给出正确配对。
            raise AgentReferenceError(
                f"ref 属于 {record.tool} 会话，不属于 {tool}",
                {"expected_tool": record.tool, "given_tool": tool,
                 "reason": "tool_mismatch",
                 "recovery": f"retry the same ref with tool={record.tool}"},
            )
        if record is None:
            # 恢复办法要放进 params:agent 只看得到结构化错误,看不到这句中文。
            raise AgentReferenceError(
                "ref 不在当前扫描索引中",
                {"tool": tool, "reason": "unknown_ref",
                 "recovery": _REF_RECOVERY_HINT},
            )
        if record.storage_kind in {"file", "directory"}:
            try:
                resolved = Path(record.canonical_ref).resolve(strict=True)
                root = Path(record.root or "").resolve(strict=True)
            except OSError as error:
                raise AgentReferenceError(
                    "ref 指向的会话已失效",
                    {"tool": tool, "reason": "session_missing",
                     "recovery": _REF_RECOVERY_HINT},
                ) from error
            expected_type = (
                resolved.is_file()
                if record.storage_kind == "file"
                else resolved.is_dir()
            )
            if not resolved.is_relative_to(root) or not expected_type:
                raise AgentReferenceError("ref 超出 Agent 会话根目录")
            browser = self._ports.adapter(tool).browser
            if pin_content:
                try:
                    identity = (
                        (
                            _path_identity(resolved),
                            _agent_fingerprint(browser, str(resolved)),
                        )
                        if record.storage_kind == "file"
                        else _directory_identity(resolved, browser)
                    )
                except (OSError, ValueError) as error:
                    raise AgentReferenceError(
                        "ref 指向的会话已失效",
                    ) from error
                if (
                    record.storage_kind == "file"
                    and identity[1] is None
                ) or record.source_identity != identity:
                    # 摘要缓存可能命中了「stat 没变但内容变了」的旧值:踢掉它,
                    # 下一次扫描才会重新哈希并换发新的 ref。
                    with self._lock:
                        if record.storage_kind == "file":
                            self._digest_cache.pop(identity[0][:4], None)
                        else:
                            self._digest_cache.clear()
                    # 同一条陈旧摘要还躺在跨进程缓存里,不覆盖的话重启后会被
                    # 再次命中。手上正好有刚算出的真实摘要,直接写回。
                    self._store_identity_digests(
                        resolved, record.storage_kind, identity,
                    )
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
            # 钉内容才需要严格指纹;只查存在性时用扫描口径的宽松指纹,
            # 否则 UI 浏览/内容索引会在活跃库上触发同步整库重建。
            probe = _agent_fingerprint if pin_content else _scan_fingerprint
            fingerprint = probe(browser, record.canonical_ref)
            if fingerprint is None:
                raise AgentReferenceError(
                    "ref 指向的会话已失效",
                    {"tool": tool, "reason": "session_missing",
                     "recovery": _REF_RECOVERY_HINT},
                )
            if pin_content and fingerprint != record.source_identity:
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

    def _canonicalize_all(self, scanned, digest_store=None) -> list[tuple]:
        rows = list(scanned)
        if len(rows) < _PARALLEL_CANONICALIZE_THRESHOLD:
            return [
                self._canonicalize(adapter, row, digest_store)
                for _, adapter, row in rows
            ]
        with ThreadPoolExecutor(max_workers=_CANONICALIZE_WORKERS) as pool:
            return list(
                pool.map(
                    lambda item: self._canonicalize(
                        item[1], item[2], digest_store,
                    ),
                    rows,
                )
            )

    def _canonicalize(self, adapter, row: dict, digest_store=None) -> tuple[
        str | None,
        str | None,
        str | None,
        tuple | str | None,
    ]:
        native = adapter.browser.canonicalize(row)
        if native is None:
            return None, None, None, None
        if native.storage_kind in {"file", "directory"}:
            # 走 os.path 而不是 pathlib:语义(realpath + 前缀归属)完全一致,
            # 但少掉数万次 Path 对象构造,全量扫描省下的时间是可观的。
            try:
                root = os.path.realpath(native.root or "", strict=True)
                path = os.path.realpath(native.canonical_ref, strict=True)
            except OSError:
                return None, None, native.storage_kind, None
            if not is_within(path, root):
                return None, None, native.storage_kind, None
            if (
                native.storage_kind == "file"
                and not os.path.isfile(path)
            ) or (
                native.storage_kind == "directory"
                and not os.path.isdir(path)
            ):
                return None, None, native.storage_kind, None
            try:
                identity = (
                    (
                        _path_identity(
                            Path(path), self._digest_cache, digest_store,
                        ),
                        _scan_fingerprint(adapter.browser, path),
                    )
                    if native.storage_kind == "file"
                    else _directory_identity(
                        Path(path), adapter.browser, self._digest_cache,
                        digest_store,
                    )
                )
                if native.storage_kind == "file" and identity[1] is None:
                    return None, None, native.storage_kind, None
            except (OSError, ValueError, AgentReferenceError):
                return None, None, native.storage_kind, None
            return path, root, native.storage_kind, identity
        if adapter.browser.resolve_ref(native.canonical_ref) != native.canonical_ref:
            return None, None, "id", None
        fingerprint = _scan_fingerprint(
            adapter.browser,
            native.canonical_ref,
        )
        if fingerprint is None:
            return None, None, "id", None
        return native.canonical_ref, None, "id", fingerprint
