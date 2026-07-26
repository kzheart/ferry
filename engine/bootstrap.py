"""CLI、RPC 与测试共用的 Engine 组装入口。"""

from . import __version__
from .adapters.registry import create_registry
from .sessions.content_index import ContentIndex
from .sessions.index import AgentSessionIndex
from .app import EngineService
from .context import EngineContext
from .system.resources import resource_path
from .sessions.scan_cache import ScanCache
from .system.snapshots import backup_dir
from .operations.service import OperationService
from .operations import metadata as _metadata


def create_context() -> EngineContext:
    registry = create_registry()
    context = EngineContext(
        adapter=registry.get,
        adapters=registry.ids,
        cache_factory=ScanCache,
        resource_path=resource_path,
        snapshot_dir=backup_dir,
        version=__version__,
    )
    context.metadata_list_all = lambda: _metadata.list_all(context)
    return context


def build_engine(ports: EngineContext | None = None) -> EngineService:
    ports = ports or create_context()
    index = AgentSessionIndex(ports)
    operations = OperationService(ports, index)
    return EngineService(ports, index, operations, ContentIndex())
