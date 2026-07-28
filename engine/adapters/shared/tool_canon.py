"""工具调用规范化的兼容入口：实现已收敛到各 adapter 的 dialect 声明。

映射的唯一事实源是 `engine/adapters/<name>/dialect.py`；本模块保留原有
函数签名，供仍按旧接口调用的 reader 与测试使用。canonical_tool_input
的历史签名不带 adapter 参数（claude 与 opencode 工具名大小写天然不冲突），
这里按 claude → opencode 顺序探测。
"""
from __future__ import annotations

import re

from .dialect import get_dialect


def canonical_tool_op(adapter: str, tool_name: str) -> str | None:
    dialect = get_dialect(adapter)
    return dialect.op_for(tool_name) if dialect else None


def patch_operations(patch: str) -> list[dict]:
    return [
        {"operation": operation.lower(), "path": path.strip()}
        for operation, path in re.findall(
            r"^\*\*\* (Add|Update|Delete) File: ([^\r\n]+)$",
            patch,
            re.MULTILINE,
        )
    ]


def canonical_tool_input(tool_name: str, raw):
    for adapter in ("claude", "opencode"):
        dialect = get_dialect(adapter)
        parsed = dialect.parse(tool_name, raw) if dialect else None
        if parsed is not None:
            return parsed[1]
    return raw
