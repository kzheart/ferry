"""Engine 能力共享的显式运行上下文。"""

from dataclasses import dataclass
from typing import Callable


@dataclass
class EngineContext:
    adapter: Callable
    adapters: Callable
    cache_factory: Callable
    resource_path: Callable
    snapshot_dir: Callable
    version: str
