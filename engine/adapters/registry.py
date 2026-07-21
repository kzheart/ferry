"""Adapter registry with package discovery and lazy ToolPlugin construction."""
from __future__ import annotations

import importlib
import pkgutil
from typing import Callable

from ..domain.errors import ToolUnknownError
from .base.plugin import ToolPlugin

def _discover_factories() -> dict[str, Callable[[], ToolPlugin]]:
    """Load every bundled adapter package exposing ``plugin.build``.

    Adding an adapter only requires creating ``adapters/<id>/plugin.py``. The
    registry deliberately contains no per-agent imports or identifiers.
    """
    factories = {}
    package = importlib.import_module(__package__)
    packages = sorted(pkgutil.iter_modules(package.__path__), key=lambda item: item.name)
    for _, name, is_package in packages:
        if not is_package or name == "base":
            continue
        module_name = f"{__package__}.{name}.plugin"
        if importlib.util.find_spec(module_name) is None:
            continue
        module = importlib.import_module(module_name)
        factory = getattr(module, "build", None)
        if callable(factory):
            manifest = getattr(module, "MANIFEST", None)
            tool_id = manifest.id if manifest is not None else name
            if tool_id in factories:
                raise ValueError(f"重复的 adapter id: {tool_id}")
            factories[tool_id] = factory
    return factories


_FACTORIES: dict[str, Callable[[], ToolPlugin]] = _discover_factories()

_PLUGINS: dict[str, ToolPlugin] = {}


def register(factory: Callable[[], ToolPlugin], tool_id: str | None = None) -> None:
    """注册额外插件（测试 fake、实验性 Agent）。"""
    plugin = factory()
    _FACTORIES[tool_id or plugin.id] = factory
    _PLUGINS[tool_id or plugin.id] = plugin


def unregister(tool_id: str) -> None:
    _FACTORIES.pop(tool_id, None)
    _PLUGINS.pop(tool_id, None)


def adapter(tool: str) -> ToolPlugin:
    try:
        plugin = _PLUGINS.get(tool)
        if plugin is None:
            plugin = _PLUGINS[tool] = _FACTORIES[tool]()
        return plugin
    except KeyError as error:
        raise ToolUnknownError(tool) from error


def adapters() -> tuple[str, ...]:
    return tuple(_FACTORIES)
