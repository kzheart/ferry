#!/usr/bin/env python3
"""守护 Engine 的分层方向：禁止 adapters 反向依赖 operations。

adapters 是被 operations 编排的下层，不该伸进 operations 内部。相对 import
（`from ...operations.x import y`）按文件在包里的深度还原成绝对模块名后再判定。
"""
from __future__ import annotations

import ast
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ENGINE = ROOT / "engine"

RULES = (
    (
        "engine/adapters",
        lambda module: module == "engine.operations"
        or module.startswith("engine.operations."),
        "adapters 不得依赖 engine.operations（类型走 contracts.operation_types，"
        "快照工具走 system.snapshots）",
    ),
)


def absolute_module(node: ast.ImportFrom, path: Path) -> str | None:
    """把 `from ..x import y` 还原为 `engine.x`；绝对 import 原样返回。"""
    if node.level == 0:
        return node.module
    package = path.relative_to(ROOT).parent.parts
    if node.level > len(package):
        return None
    base = package[: len(package) - node.level + 1]
    return ".".join([*base, *([node.module] if node.module else [])])


def imported_modules(path: Path) -> list[tuple[str, int]]:
    tree = ast.parse(path.read_text(), filename=str(path))
    modules = []
    for node in ast.walk(tree):
        if isinstance(node, ast.ImportFrom):
            module = absolute_module(node, path)
            if module:
                modules.append((module, node.lineno))
        elif isinstance(node, ast.Import):
            modules.extend((alias.name, node.lineno) for alias in node.names)
    return modules


def main() -> int:
    violations = []
    for area, forbidden, reason in RULES:
        for path in sorted((ROOT / area).rglob("*.py")):
            for module, line in imported_modules(path):
                if forbidden(module):
                    relative = path.relative_to(ROOT)
                    violations.append(f"{relative}:{line}: import {module} — {reason}")
    if violations:
        print("Engine 分层检查失败:", *violations, sep="\n")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
