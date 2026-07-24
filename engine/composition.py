"""Default composition root for CLI, RPC, and import consumers."""

from . import __version__
from .adapters.registry import create_registry
from .application.agent_tools import AgentSessionIndex
from .application.engine import EngineApplication
from .application.ports import ApplicationPorts
from .infrastructure.resources import resource_path
from .infrastructure.scan_cache import ScanCache
from .infrastructure.snapshots import backup_dir
from .operations.service import OperationService


def create_ports() -> ApplicationPorts:
    registry = create_registry()
    return ApplicationPorts(
        adapter=registry.get,
        adapters=registry.ids,
        cache_factory=ScanCache,
        resource_path=resource_path,
        snapshot_dir=backup_dir,
        version=__version__,
    )


def build_application(ports: ApplicationPorts | None = None) -> EngineApplication:
    ports = ports or create_ports()
    index = AgentSessionIndex(ports)
    operations = OperationService(ports, index)
    return EngineApplication(ports, index, operations)
