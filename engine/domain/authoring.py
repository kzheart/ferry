"""声明式 AI 回复模型；不承载任何原生存储标识。"""
from __future__ import annotations

from dataclasses import dataclass
from typing import Any


@dataclass(frozen=True)
class TextItem:
    text: str
    kind: str = "text"

    def to_dict(self) -> dict:
        return {"kind": self.kind, "text": self.text}


@dataclass(frozen=True)
class ToolItem:
    name: str
    input: dict | str
    output: str
    kind: str = "tool"

    def to_dict(self) -> dict:
        return {"kind": self.kind, "name": self.name,
                "input": self.input, "output": self.output}


ReplyItem = TextItem | ToolItem


@dataclass(frozen=True)
class AssistantReply:
    items: tuple[ReplyItem, ...]

    @classmethod
    def from_dict(cls, value: Any) -> "AssistantReply":
        if not isinstance(value, dict) or set(value) != {"items"}:
            raise ValueError("reply 必须且只能包含 items")
        raw_items = value["items"]
        if not isinstance(raw_items, list) or not raw_items:
            raise ValueError("reply.items 必须是非空数组")
        items: list[ReplyItem] = []
        for index, raw in enumerate(raw_items):
            if not isinstance(raw, dict):
                raise ValueError(f"reply.items[{index}] 必须是对象")
            kind = raw.get("kind")
            if kind == "text":
                if set(raw) != {"kind", "text"} or not isinstance(raw.get("text"), str):
                    raise ValueError(f"reply.items[{index}] text 结构非法")
                if not raw["text"]:
                    raise ValueError(f"reply.items[{index}].text 不可为空")
                items.append(TextItem(raw["text"]))
            elif kind == "tool":
                if set(raw) != {"kind", "name", "input", "output"}:
                    raise ValueError(f"reply.items[{index}] tool 结构非法")
                if not isinstance(raw.get("name"), str) or not raw["name"]:
                    raise ValueError(f"reply.items[{index}].name 必须是非空字符串")
                if not isinstance(raw.get("input"), (dict, str)):
                    raise ValueError(f"reply.items[{index}].input 必须是对象或字符串")
                if not isinstance(raw.get("output"), str):
                    raise ValueError(f"reply.items[{index}].output 必须是字符串")
                items.append(ToolItem(raw["name"], raw["input"], raw["output"]))
            else:
                raise ValueError(f"reply.items[{index}].kind 仅支持 text/tool")
        return cls(tuple(items))

    def to_dict(self) -> dict:
        return {"items": [item.to_dict() for item in self.items]}
