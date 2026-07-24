"""格式无关的会话编辑契约与通用事务工具。

通用层只编排 preview/apply；每个 Agent 包内实现自己的
``EditBackend``，公共模块不 import 任何具体 Agent 实现。
"""
from __future__ import annotations

import hashlib
import json
import os
from abc import ABC, abstractmethod
from dataclasses import dataclass
from pathlib import Path

from ...operations.types import AssistantReply, ToolItem
from ...errors import (
    OperationUnsupportedError,
    SubagentNotSupportedError,
)
from .codec import TurnSpan, positive_turn, select_span  # noqa: F401

SPAWN_TOOL_NAMES = {"agent", "spawn_agent", "task"}


@dataclass
class EditDocument:
    tool: str
    ref: str
    handle: object
    data: object
    revision: str
    context: object | None = None


class EditBackend(ABC):
    """Agent 原生编辑契约；API 与 UI 不得依赖具体存储格式。"""

    name: str
    operations = ("delete-turn", "rewrite")

    @abstractmethod
    def load(self, ref: str) -> EditDocument: ...

    @abstractmethod
    def apply_ops(self, doc: EditDocument, ops: list[dict]) -> list[str]: ...

    def replace_reply(
        self, doc: EditDocument, turn: int | str, reply: AssistantReply
    ) -> list[str]:
        raise OperationUnsupportedError(
            self.name, "replace-assistant-reply", "inplace"
        )

    @abstractmethod
    def validate(self, doc: EditDocument) -> None: ...

    @abstractmethod
    def stats(self, doc: EditDocument) -> dict: ...

    @abstractmethod
    def commit(self, doc: EditDocument) -> dict: ...

    def snapshot(self, doc: EditDocument, reason_code="snapshot.before_edit",
                 extra: dict | None = None) -> Path | None:
        return None

    def restore_snapshot(self, snapshot: Path, doc: EditDocument) -> None:
        raise NotImplementedError

    def saved_revision(self, result: dict, doc: EditDocument) -> str:
        path = Path(result.get("saved_as", ""))
        if path.is_file():
            return hash_bytes(path.read_bytes())
        raise RuntimeError(f"{self.name} 无法读取已保存会话 revision")


def hash_bytes(data: bytes) -> str:
    return "sha256:" + hashlib.sha256(data).hexdigest()


def json_size(value) -> int:
    return len(json.dumps(value, ensure_ascii=False).encode())


def write_jsonl(path: Path, records: list[dict]) -> None:
    tmp = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    tmp.write_text("\n".join(json.dumps(r, ensure_ascii=False) for r in records) + "\n")
    os.replace(tmp, path)


def reject_replacement_spawn(reply: AssistantReply) -> None:
    if any(
        isinstance(item, ToolItem) and is_spawn_name(item.name)
        for item in reply.items
    ):
        raise SubagentNotSupportedError(
            "子 Agent spawn/task 会改变会话树，编辑操作已拒绝"
        )


def reject_target_spawn(tool: str) -> None:
    raise SubagentNotSupportedError(
        "目标回复包含子 Agent spawn/task，编辑操作已拒绝",
        {"tool": tool},
    )


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
