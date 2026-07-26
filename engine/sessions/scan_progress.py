"""扫描进度跟踪。

scan 在 serial 池中执行,同一时刻至多一个;scan_progress 走 parallel-read 池,
可在扫描期间并发查询。单例状态由锁保护;未处于扫描中的上报调用一律忽略,
因此内容索引预热等旁路扫描不会污染进度。
"""
from __future__ import annotations

import threading


class ScanProgressTracker:
    def __init__(self):
        self._lock = threading.Lock()
        self._running = False
        self._phase = "reading"
        self._tools: dict[str, dict] = {}
        self._current: str | None = None

    def begin(self, names: list[str]) -> None:
        with self._lock:
            self._running = True
            self._phase = "reading"
            self._current = None
            self._tools = {
                name: {"processed": 0, "total": None, "done": False}
                for name in names
            }

    def finalize(self) -> None:
        """文件读取结束,进入索引整理阶段(index_rows/落盘)。"""
        with self._lock:
            if self._running:
                self._phase = "finalizing"
                self._current = None

    def start_tool(self, name: str) -> None:
        with self._lock:
            if self._running and name in self._tools:
                self._current = name

    def set_total(self, total: int) -> None:
        with self._lock:
            tool = self._tools.get(self._current) if self._running else None
            if tool is not None:
                tool["total"] = total

    def advance(self, count: int = 1) -> None:
        with self._lock:
            tool = self._tools.get(self._current) if self._running else None
            if tool is not None:
                tool["processed"] += count

    def finish_tool(self, name: str) -> None:
        with self._lock:
            tool = self._tools.get(name) if self._running else None
            if tool is not None:
                tool["done"] = True
                if tool["total"] is None:
                    tool["total"] = tool["processed"]
                if name == self._current:
                    self._current = None

    def end(self) -> None:
        with self._lock:
            self._running = False
            self._current = None

    def snapshot(self) -> dict:
        with self._lock:
            tools = {
                name: {
                    "processed": tool["processed"],
                    "total": tool["total"],
                    "done": tool["done"],
                }
                for name, tool in self._tools.items()
            }
            processed = sum(tool["processed"] for tool in tools.values())
            total = sum(
                tool["total"] for tool in tools.values()
                if tool["total"] is not None
            )
            return {
                "state": "running" if self._running else "idle",
                "phase": self._phase,
                "processed": processed,
                "total": total,
                "tools": tools,
            }


TRACKER = ScanProgressTracker()
