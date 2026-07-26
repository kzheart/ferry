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
MetadataListAll = Callable[[], dict]


def _metadata_port_missing() -> dict:
    raise RuntimeError(
        "EngineContext.metadata_list_all 未组装；请经 bootstrap.create_context 构造"
    )


@dataclass
class EngineContext:
    adapter: AdapterResolver
    adapters: AdapterIds
    cache_factory: CacheFactory
    resource_path: ResourcePathResolver
    snapshot_dir: SnapshotDir
    version: str
    # organization 不再直接 import operations 的元数据模块，改由此端口注入。
    metadata_list_all: MetadataListAll = _metadata_port_missing
