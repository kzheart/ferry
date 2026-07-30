"""常驻活索引:文件系统轮询探测 + 增量重扫 + 周期全量对账。

引擎不再等 UI 拉取才知道世界变了:后台线程每个周期对各 adapter 的会话
存储做一轮廉价的 stat 扫描(数千文件约 20ms),源头一变就只重扫那个工具,
delta 经 AgentSessionIndex.on_delta 推给前端。周期性的全量对账兜住
轮询窗口内可能漏掉的变化(睡眠恢复、外置卷、探测失败)。

adapter 可选提供 browser.watch_stamp() 返回廉价变更令牌(如 opencode
只需 stat 一下 sqlite 的 db/-wal);未提供时默认对 manifest.source_path
做全树 stat 扫描。新增 agent 零成本获得实时能力。
"""
from __future__ import annotations

import hashlib
import logging
import os
import threading
import time

log = logging.getLogger(__name__)

_POLL_INTERVAL = 2.5
_RECONCILE_INTERVAL = 300.0
# 手动刷新(nudge)触发的全量对账最小间隔:防止 UI 连点造成扫描风暴。
_NUDGE_MIN_GAP = 5.0


def _tree_stamp(root: str) -> str | None:
    """目录树的变更令牌:全部文件的 (路径, mtime, size) 排序后哈希。"""
    try:
        base = os.path.realpath(os.path.expanduser(root), strict=True)
    except OSError:
        return None
    entries: list[str] = []
    stack = [base]
    while stack:
        directory = stack.pop()
        try:
            iterator = os.scandir(directory)
        except OSError:
            continue
        with iterator:
            for entry in iterator:
                try:
                    if entry.is_dir(follow_symlinks=False):
                        stack.append(entry.path)
                        continue
                    info = entry.stat(follow_symlinks=False)
                except OSError:
                    continue
                entries.append(
                    f"{entry.path}\x00{info.st_mtime_ns}\x00{info.st_size}"
                )
    entries.sort()
    digest = hashlib.sha256()
    for line in entries:
        digest.update(line.encode())
        digest.update(b"\n")
    return digest.hexdigest()


class LiveIndexService:
    def __init__(
        self,
        index,
        *,
        poll_interval: float = _POLL_INTERVAL,
        reconcile_interval: float = _RECONCILE_INTERVAL,
        nudge_min_gap: float = _NUDGE_MIN_GAP,
    ):
        self._index = index
        self._poll_interval = poll_interval
        self._reconcile_interval = reconcile_interval
        self._nudge_min_gap = nudge_min_gap
        self._wake = threading.Event()
        self._stop = threading.Event()
        self._nudge_lock = threading.Lock()
        self._nudged = False
        self._tokens: dict[str, object] = {}
        self._thread: threading.Thread | None = None

    def start(self) -> None:
        if self._thread is not None:
            return
        self._thread = threading.Thread(
            target=self._run, daemon=True, name="live-index",
        )
        self._thread.start()

    def stop(self) -> None:
        self._stop.set()
        self._wake.set()

    def nudge(self) -> None:
        """UI 手动刷新的逃生口:立即轮询一轮并(限频地)全量对账。"""
        with self._nudge_lock:
            self._nudged = True
        self._wake.set()

    def _take_nudge(self) -> bool:
        with self._nudge_lock:
            nudged, self._nudged = self._nudged, False
        return nudged

    def _run(self) -> None:
        last_reconcile = time.monotonic()
        while not self._stop.is_set():
            self._wake.wait(self._poll_interval)
            self._wake.clear()
            if self._stop.is_set():
                return
            nudged = self._take_nudge()
            # 首次全量扫描(启动预热)完成前没有可增量的基线。
            if self._index.snapshot_with_status() is None:
                continue
            for name in self._changed_tools():
                if self._stop.is_set():
                    return
                try:
                    self._index.refresh_tool(name)
                except Exception:  # noqa: BLE001 - 单工具失败不影响轮询
                    log.exception("增量重扫失败: %s", name)
            now = time.monotonic()
            due = now - last_reconcile >= self._reconcile_interval
            if due or (nudged and now - last_reconcile >= self._nudge_min_gap):
                try:
                    self._index.refresh_with_status()
                except Exception:  # noqa: BLE001 - 对账失败等下一轮
                    log.exception("周期对账失败")
                last_reconcile = time.monotonic()

    def _changed_tools(self) -> list[str]:
        changed: list[str] = []
        for name in self._index.ports.adapters():
            token = self._probe(name)
            if token is None:
                continue
            known = self._tokens.get(name)
            self._tokens[name] = token
            # 首次观测只记基线:快照刚由全量扫描建立,无需重扫。
            if known is not None and known != token:
                changed.append(name)
        return changed

    def _probe(self, name: str):
        adapter = self._index.ports.adapter(name)
        browser = getattr(adapter, "browser", None)
        if browser is None:
            return None
        stamp = getattr(browser, "watch_stamp", None)
        if stamp is not None:
            try:
                return ("adapter", stamp())
            except Exception:  # noqa: BLE001 - 探测失败按不可知处理
                log.exception("watch_stamp 探测失败: %s", name)
                return None
        source = getattr(
            getattr(adapter, "manifest", None), "source_path", None,
        )
        if not source:
            return None
        return _tree_stamp(source)
