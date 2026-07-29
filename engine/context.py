"""Engine 能力共享的显式运行上下文。"""
from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol, TYPE_CHECKING

if TYPE_CHECKING:
    from .adapters.contracts import AgentAdapter


class ScanCachePort(Protocol):
    def get(self, path, stat): ...

    def put(self, path, stat, meta) -> None: ...

    def flush(self) -> None: ...


class ResourcePathResolver(Protocol):
    def __call__(self, *parts: str) -> Path: ...


AdapterResolver = Callable[[str], "AgentAdapter"]
AdapterIds = Callable[[], tuple[str, ...]]
CacheFactory = Callable[[], ScanCachePort]
SnapshotDir = Callable[[], Path]


@dataclass
class EngineContext:
    adapter: AdapterResolver
    adapters: AdapterIds
    cache_factory: CacheFactory
    resource_path: ResourcePathResolver
    snapshot_dir: SnapshotDir
    version: str
    # 状态库位置。缺省回落到 snapshot_dir，让直接构造 context 的测试无需改动；
    # 生产经 bootstrap 显式指向 ~/.ferry，与备份快照目录分开。
    data_dir: SnapshotDir | None = None

    def state_dir(self) -> Path:
        """状态库（ferry-state.sqlite3）所在目录。"""
        return (self.data_dir or self.snapshot_dir)()
