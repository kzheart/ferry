"""Application-facing ports."""

from dataclasses import dataclass
from typing import Callable


@dataclass
class ApplicationPorts:
    adapter: Callable
    adapters: Callable
    cache_factory: Callable
    resource_path: Callable
    snapshot_dir: Callable
    version: str
