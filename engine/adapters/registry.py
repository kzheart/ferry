"""引擎内唯一的插件注册表：只负责装配 ToolPlugin。

adapter 全部随应用打包，静态 factory 列表即可；lifecycle、
resolve、cleanup 等策略都在各 Agent 包内，不在这里。
"""
from __future__ import annotations

from typing import Callable

from ..domain.errors import ToolUnknownError
from .base.plugin import ToolPlugin
from .claude.plugin import build as _build_claude
from .codex.plugin import build as _build_codex
from .opencode.plugin import build as _build_opencode

_FACTORIES: dict[str, Callable[[], ToolPlugin]] = {
    "claude": _build_claude,
    "codex": _build_codex,
    "opencode": _build_opencode,
}

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
