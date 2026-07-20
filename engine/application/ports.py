"""Application-facing ports and process-local dependency wiring."""

from dataclasses import dataclass
from typing import Callable


@dataclass
class ApplicationPorts:
    adapter: Callable
    adapters: Callable
    cache_factory: Callable
    resource_path: Callable
    version: str


_ports: ApplicationPorts | None = None


def configure(ports: ApplicationPorts) -> None:
    global _ports
    _ports = ports


def current() -> ApplicationPorts:
    if _ports is None:
        from ..composition import configure_application
        configure_application()
    assert _ports is not None
    return _ports
