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
