#!/usr/bin/env python3
"""Reject reintroduction of mirrored canonical tool-result fields."""
from __future__ import annotations

import ast
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
MODEL = ROOT / "engine" / "domain" / "model.py"
FORBIDDEN_TOOL_CALL_FIELDS = {"output", "status", "tool_result"}
FORBIDDEN_METHODS = {
    "from_legacy",
    "legacy_output",
    "normalize_tool_result_status",
    "set_result",
}
FORBIDDEN_PRIVATE_FIELDS = (
    "canonical" + "ToolResult",
    "canonical_" + "blocks",
    "canonical_" + "metadata",
)


def violations() -> list[str]:
    tree = ast.parse(MODEL.read_text(), filename=str(MODEL))
    found = []
    for node in ast.walk(tree):
        if (
            isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
            and node.name == "normalize_tool_result_status"
        ):
            found.append(f"{node.name}()")
        if isinstance(node, ast.ClassDef) and node.name == "ToolCall":
            for statement in node.body:
                if isinstance(statement, ast.AnnAssign):
                    target = statement.target
                    if (
                        isinstance(target, ast.Name)
                        and target.id in FORBIDDEN_TOOL_CALL_FIELDS
                    ):
                        found.append(f"ToolCall.{target.id}")
                if (
                    isinstance(statement, (ast.FunctionDef, ast.AsyncFunctionDef))
                    and statement.name in FORBIDDEN_METHODS
                ):
                    found.append(f"ToolCall.{statement.name}()")
        if (
            isinstance(node, ast.ClassDef)
            and node.name == "ToolResult"
        ):
            for statement in node.body:
                if (
                    isinstance(statement, (ast.FunctionDef, ast.AsyncFunctionDef))
                    and statement.name in FORBIDDEN_METHODS
                ):
                    found.append(f"ToolResult.{statement.name}()")
    for path in (ROOT / "engine").rglob("*.py"):
        text = path.read_text(errors="replace")
        for line_number, line in enumerate(text.splitlines(), 1):
            for field in FORBIDDEN_PRIVATE_FIELDS:
                if field in line:
                    found.append(
                        f"{path.relative_to(ROOT)}:{line_number}: {field}"
                    )
    return found


def main() -> int:
    found = violations()
    if found:
        print("Canonical tool results must have one source of truth:")
        print("\n".join(found))
        return 1
    print("Canonical tool-result model has one source of truth.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
