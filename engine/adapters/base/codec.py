"""统一轮次解析契约：TurnIndex（读侧）与 NativeEditCodec（写侧）。

每个 Agent 只允许存在一份原生会话解析实现；reader、delete-turn、
rewrite、replace-reply 全部消费同一个 TurnIndex，避免语义漂移。
"""
from __future__ import annotations

from dataclasses import dataclass
from typing import Protocol, runtime_checkable

from ...domain.errors import LocatorStaleError, TurnOutOfRangeError


@dataclass(frozen=True)
class TurnSpan:
    """一轮对话在原生记录中的区间。

    ordinal 从 1 起；locator 是对 UI 稳定的定位符；
    [start, end) 是原生记录序列中的半开区间。
    """

    ordinal: int
    locator: str
    start: int
    end: int


@runtime_checkable
class TurnIndex(Protocol):
    """读侧契约：所有 Agent 必须提供。"""

    def visible_messages(self, document) -> list:
        """返回参与轮次判定的可见原生消息（含索引）。"""

    def turns(self, document) -> list[TurnSpan]:
        """返回按顺序排列的轮次区间。"""


@runtime_checkable
class NativeEditCodec(Protocol):
    """写侧契约：可选能力，只读 Agent 不实现。"""

    def replace_reply(self, document, span: TurnSpan, reply) -> list:
        """把 span 对应轮次的 AI 回复替换为 reply，返回结构化变更。"""

    def delete_turn(self, document, span: TurnSpan) -> list:
        """删除 span 对应的整轮，返回结构化变更。"""

    def rewrite_message(self, document, locator: str, text: str) -> list:
        """改写 locator 指向的用户消息文本，返回结构化变更。"""


def select_span(spans: list[TurnSpan], selector) -> TurnSpan:
    """按 ordinal（正整数）或 locator（字符串）选择轮次。"""
    if isinstance(selector, str):
        for span in spans:
            if span.locator == selector:
                return span
        raise LocatorStaleError(params={"locator": selector})
    ordinal = positive_turn(selector)
    if ordinal > len(spans):
        raise TurnOutOfRangeError(ordinal, len(spans))
    return spans[ordinal - 1]


def positive_turn(turn) -> int:
    if isinstance(turn, bool):
        raise TurnOutOfRangeError(turn)
    try:
        value = int(turn)
    except (TypeError, ValueError) as error:
        raise TurnOutOfRangeError(turn) from error
    if value < 1 or value != turn:
        raise TurnOutOfRangeError(turn)
    return value
