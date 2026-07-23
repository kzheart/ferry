"""Explicit composition of Ferry's built-in session adapters."""
from __future__ import annotations

from collections.abc import Iterable

from ..contracts.agents import AGENT_IDS
from ..domain.errors import ToolUnknownError
from .base.plugin import ToolPlugin
from .claude.plugin import build as build_claude
from .codex.plugin import build as build_codex
from .opencode.plugin import build as build_opencode


class AdapterRegistry:
    """Immutable adapter lookup owned by the application composition root."""

    def __init__(self, plugins: Iterable[ToolPlugin]):
        items: dict[str, ToolPlugin] = {}
        for plugin in plugins:
            if plugin.id in items:
                raise ValueError(f"重复的 adapter id: {plugin.id}")
            items[plugin.id] = plugin
        self._items = items

    def get(self, tool: str) -> ToolPlugin:
        try:
            return self._items[tool]
        except KeyError as error:
            raise ToolUnknownError(tool) from error

    def ids(self) -> tuple[str, ...]:
        return tuple(self._items)


def create_registry() -> AdapterRegistry:
    builders = {
        "claude": build_claude,
        "codex": build_codex,
        "opencode": build_opencode,
    }
    return AdapterRegistry(builders[agent_id]() for agent_id in AGENT_IDS)
