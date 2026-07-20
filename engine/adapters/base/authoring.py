"""AssistantReply authoring 的公共契约与通用输入验证。

只保留格式无关的部分；各 Agent 的编译器实现位于对应 Agent 包内。
"""
from __future__ import annotations

from abc import ABC, abstractmethod

from ...domain.authoring import AssistantReply, ToolItem
from ...domain.errors import SubagentNotSupportedError
from .codec import TurnSpan, positive_turn, select_span  # noqa: F401 (re-export)

SPAWN_TOOL_NAMES = {"agent", "spawn_agent", "task"}


class AuthoringCompiler(ABC):
    name: str
    inplace = True
    save_as = True

    def capabilities(self) -> dict:
        modes = (["inplace"] if self.inplace else []) + (["saveas"] if self.save_as else [])
        return {"tool": self.name, "operation": "replace-assistant-reply",
                "item_kinds": ["text", "tool"], "ordered": True,
                "tool_fields": ["name", "input", "output"],
                "turn_selectors": ["ordinal", "locator"],
                "inplace": self.inplace, "save_as": self.save_as,
                "operation_modes": {"replace-assistant-reply": modes}}

    def supports_mode(self, save_as: bool) -> bool:
        return self.save_as if save_as else self.inplace

    @abstractmethod
    def replace(self, doc, turn: int, reply: AssistantReply) -> list[str]: ...


def select_turn(candidates: list[tuple[int, str]], selector) -> tuple[int, int]:
    """按 ordinal（正整数）或 locator（字符串）选择 (ordinal, index)。"""
    spans = [TurnSpan(ordinal, locator, index, index)
             for ordinal, (index, locator) in enumerate(candidates, 1)]
    span = select_span(spans, selector)
    return span.ordinal, span.start


def reject_authored_spawn(reply: AssistantReply) -> None:
    if any(isinstance(item, ToolItem) and is_spawn_name(item.name)
           for item in reply.items):
        raise SubagentNotSupportedError(
            "子 Agent spawn/task 会改变会话树，authoring 已拒绝")


def reject_target_spawn(tool: str) -> None:
    """目标回复区间内含子 Agent spawn 时拒绝 authoring。"""
    raise SubagentNotSupportedError(
        "目标回复包含子 Agent spawn/task，authoring 已拒绝", {"tool": tool})


def is_spawn_name(name) -> bool:
    return isinstance(name, str) and name.lower() in SPAWN_TOOL_NAMES


def replace_at_first(records, is_reply, compiled):
    result = []
    inserted = False
    for record in records:
        if is_reply(record):
            if not inserted:
                result.extend(compiled)
                inserted = True
        else:
            result.append(record)
    if not inserted:
        result.extend(compiled)
    return result
