"""按文件身份缓存会话解析结果。

浏览一次大会话树要把主文件加所有子代理文件全部重新解析,而子代理文件
几乎从不变化——解析是纯函数,产物却在树装配时被 append 式修改
(children/agent_edges/loss 累积)。缓存因此存「解析基线」:命中时先把
可变派生字段恢复到刚解析完的状态,再交给装配逻辑重跑,保证与全新解析
完全等价。

并发约束:同一棵树的装配会修改共享的 Session 对象,调用方必须对同一
根会话的读取做互斥(见 codex reader 的 per-root 锁)。
"""
from __future__ import annotations

import os
import threading
from collections import OrderedDict

_MAX_ENTRIES = 512
# 内存上限按源文件字节数近似(Python 对象实际占用更大,但同数量级)。
_MAX_TOTAL_BYTES = 256 * 1024 * 1024
# 巨型活跃会话(上百 MB)缓存命中率低、常驻内存代价高,直接旁路。
_MAX_FILE_BYTES = 32 * 1024 * 1024


def _identity(stat: os.stat_result) -> tuple:
    return (stat.st_dev, stat.st_ino, stat.st_mtime_ns, stat.st_size)


class SessionParseCache:
    def __init__(
        self,
        snapshot,
        restore,
        *,
        max_entries: int = _MAX_ENTRIES,
        max_total_bytes: int = _MAX_TOTAL_BYTES,
        max_file_bytes: int = _MAX_FILE_BYTES,
    ):
        self._snapshot = snapshot
        self._restore = restore
        self._max_entries = max_entries
        self._max_total = max_total_bytes
        self._max_file = max_file_bytes
        self._lock = threading.Lock()
        self._entries: OrderedDict[str, tuple[tuple, object, object, int]] = (
            OrderedDict()
        )
        self._total = 0

    def get_or_parse(self, path: str, parser):
        try:
            before = os.stat(path)
        except OSError:
            return parser(path)
        identity = _identity(before)
        with self._lock:
            hit = self._entries.get(path)
            if hit is not None and hit[0] == identity:
                self._entries.move_to_end(path)
                _, value, baseline, _size = hit
                self._restore(value, baseline)
                return value
        value = parser(path)
        if before.st_size > self._max_file:
            return value
        try:
            after = os.stat(path)
        except OSError:
            return value
        if _identity(after) != identity:
            # 解析期间文件变了:结果仍可用,但不能以旧身份入缓存。
            return value
        baseline = self._snapshot(value)
        with self._lock:
            stale = self._entries.pop(path, None)
            if stale is not None:
                self._total -= stale[3]
            self._entries[path] = (identity, value, baseline, before.st_size)
            self._total += before.st_size
            while self._entries and (
                len(self._entries) > self._max_entries
                or self._total > self._max_total
            ):
                _key, (_id, _value, _baseline, size) = self._entries.popitem(
                    last=False,
                )
                self._total -= size
        return value

    def clear(self) -> None:
        with self._lock:
            self._entries.clear()
            self._total = 0
