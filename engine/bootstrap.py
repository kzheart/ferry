"""CLI、RPC 与测试共用的 Engine 组装入口。"""

from . import __version__
from .adapters.registry import create_registry
from .sessions.catalog import AgentSessionIndex
from .app import EngineService
from .context import EngineContext
from .system.resources import resource_path
from .storage.scan_cache import ScanCache
from .storage.snapshots import backup_dir
from .operations.service import OperationService


def create_context() -> EngineContext:
    registry = create_registry()
    return EngineContext(
        adapter=registry.get,
        adapters=registry.ids,
        cache_factory=ScanCache,
        resource_path=resource_path,
        snapshot_dir=backup_dir,
        version=__version__,
    )


def build_engine(ports: EngineContext | None = None) -> EngineService:
    ports = ports or create_context()
    index = AgentSessionIndex(ports)
    operations = OperationService(ports, index)
    return EngineService(ports, index, operations)
