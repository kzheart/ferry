"""Explicit composition of Ferry's built-in session adapters."""
from __future__ import annotations

from collections.abc import Iterable

from ..contracts.agents import AGENT_IDS
from ..errors import ToolUnknownError
from .contracts import AgentAdapter
from .claude.adapter import build as build_claude
from .codex.adapter import build as build_codex
from .opencode.adapter import build as build_opencode
from .pi.adapter import build as build_pi
from .grok.adapter import build as build_grok

ADAPTER_BUILDERS = {
    "claude": build_claude,
    "codex": build_codex,
    "opencode": build_opencode,
    "pi": build_pi,
    "grok": build_grok,
}

#: 只有 Rust 引擎实现的 Agent。
#:
#: contracts/agents.json 同时喂两个引擎，但 Python 引擎已进入维护期，只读型的
#: 新 Agent 不再补 Python 实现。这些 id 在 Python 侧整体跳过：不装配 adapter，
#: 也不参与 AGENT_IDS 一致性校验，否则 create_registry 会直接 ValueError。
RUST_ONLY_AGENTS = frozenset({"cursor"})

#: Python 引擎实际装配的 Agent id（保持 AGENT_IDS 的顺序）。
PYTHON_AGENT_IDS = tuple(
    agent_id for agent_id in AGENT_IDS if agent_id not in RUST_ONLY_AGENTS
)


class AdapterRegistry:
    """Immutable adapter lookup owned by the Engine composition root."""

    def __init__(self, adapters: Iterable[AgentAdapter]):
        items: dict[str, AgentAdapter] = {}
        for adapter in adapters:
            if adapter.id in items:
                raise ValueError(f"重复的 adapter id: {adapter.id}")
            items[adapter.id] = adapter
        self._items = items

    def get(self, tool: str) -> AgentAdapter:
        try:
            return self._items[tool]
        except KeyError as error:
            raise ToolUnknownError(tool) from error

    def ids(self) -> tuple[str, ...]:
        return tuple(self._items)


def create_registry() -> AdapterRegistry:
    expected = set(PYTHON_AGENT_IDS)
    actual = set(ADAPTER_BUILDERS)
    if expected != actual:
        missing = sorted(expected - actual)
        extra = sorted(actual - expected)
        raise ValueError(
            "Adapter builders 与 AGENT_IDS 不一致: "
            f"missing={missing}, extra={extra}"
        )
    return AdapterRegistry(
        ADAPTER_BUILDERS[agent_id]() for agent_id in PYTHON_AGENT_IDS
    )
