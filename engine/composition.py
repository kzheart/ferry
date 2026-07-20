"""Default composition root for CLI, RPC, and import consumers."""

from . import __version__
from .adapters.registry import adapter, adapters
from .application.ports import ApplicationPorts, configure
from .infrastructure.resources import resource_path
from .infrastructure.scan_cache import ScanCache
from .infrastructure.snapshots import BACKUP_DIR


def configure_application() -> None:
    configure(ApplicationPorts(
        adapter=adapter,
        adapters=adapters,
        cache_factory=ScanCache,
        resource_path=resource_path,
        snapshot_dir=BACKUP_DIR,
        version=__version__,
    ))
