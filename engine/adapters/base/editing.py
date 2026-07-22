"""格式无关的会话编辑契约与通用事务工具。

通用层只编排 preview/apply/save-as；每个 Agent 包内实现自己的
``EditBackend``，公共模块不 import 任何具体 Agent 实现。
"""
from __future__ import annotations

import hashlib
import json
import os
from abc import ABC, abstractmethod
from dataclasses import dataclass
from pathlib import Path


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
    inplace = True
    save_as = True
    probe = True

    def capabilities(self) -> dict:
        operations = ["delete-turn", "rewrite"]
        return {"tool": self.name, "operations": operations,
            "inplace": self.inplace, "save_as": self.save_as,
            "probe": self.probe,
            "operation_roles": {"rewrite": ["user", "assistant"],
                                "delete-turn": ["turn"]},
            "operation_modes": {op: (["inplace"] if self.inplace else []) +
                                (["saveas"] if self.save_as else [])
                                for op in operations}}

    def supports_mode(self, ops: list[dict], save_as: bool) -> bool:
        mode = "saveas" if save_as else "inplace"
        modes = self.capabilities().get("operation_modes", {})
        return all(mode in modes.get(op.get("op"), []) for op in ops)

    @abstractmethod
    def load(self, ref: str) -> EditDocument: ...

    @abstractmethod
    def apply_ops(self, doc: EditDocument, ops: list[dict]) -> list[str]: ...

    @abstractmethod
    def validate(self, doc: EditDocument) -> None: ...

    @abstractmethod
    def stats(self, doc: EditDocument) -> dict: ...

    @abstractmethod
    def commit(self, doc: EditDocument) -> dict: ...

    @abstractmethod
    def save_copy(self, doc: EditDocument) -> dict: ...

    def snapshot(self, doc: EditDocument, reason_code="snapshot.before_edit",
                 extra: dict | None = None) -> Path | None:
        return None

    def restore_snapshot(self, snapshot: Path, doc: EditDocument) -> None:
        raise NotImplementedError

    def discard(self, result: dict) -> None:
        pass

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
