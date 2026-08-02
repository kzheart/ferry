"""CLI、RPC 与测试共用的 Engine 组装入口。"""

from . import __version__
from .adapters.registry import create_registry
from .sessions.content_index import ContentIndex
from .sessions.index import AgentSessionIndex
from .sessions.cleanup import CleanupService
from .app import EngineService
from .context import EngineContext
from .system.resources import resource_path
from .sessions.scan_cache import shared_cache
from .system.snapshots import backup_dir, data_dir
from .operations.service import OperationService


def create_context() -> EngineContext:
    registry = create_registry()
    context = EngineContext(
        adapter=registry.get,
        adapters=registry.ids,
        cache_factory=shared_cache,
        resource_path=resource_path,
        snapshot_dir=backup_dir,
        data_dir=data_dir,
        version=__version__,
    )
    return context


def build_engine(ports: EngineContext | None = None) -> EngineService:
    ports = ports or create_context()
    index = AgentSessionIndex(ports)
    cleanup = CleanupService(index, ports)
    operations = OperationService(ports, index, cleanup)
    return EngineService(
        ports, index, operations, ContentIndex(), cleanup=cleanup,
    )
