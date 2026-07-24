#!/usr/bin/env python3
"""Keep complete native records out of the Canonical Session model."""
from __future__ import annotations

import ast
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
MODEL = ROOT / "engine" / "sessions" / "model.py"


def violations() -> list[str]:
    tree = ast.parse(MODEL.read_text(), filename=str(MODEL))
    found = []
    for node in ast.walk(tree):
        if isinstance(node, ast.ClassDef) and node.name == "RawRecord":
            found.append("RawRecord")
        if (
            isinstance(node, ast.ClassDef)
            and node.name in {"AgentEdge", "Message", "Session"}
        ):
            for statement in node.body:
                if not isinstance(statement, ast.AnnAssign):
                    continue
                target = statement.target
                if not isinstance(target, ast.Name):
                    continue
                if node.name == "Message" and target.id == "raw":
                    found.append("Message.raw")
                if node.name == "Session" and target.id in {"meta", "raw_records"}:
                    found.append(f"Session.{target.id}")
                if node.name == "AgentEdge" and target.id == "meta":
                    found.append("AgentEdge.meta")
    for path in (ROOT / "engine").rglob("*.py"):
        text = path.read_text(errors="replace")
        for line_number, line in enumerate(text.splitlines(), 1):
            if "raw_records" in line:
                found.append(
                    f"{path.relative_to(ROOT)}:{line_number}: raw_records"
                )
    return found


def main() -> int:
    found = violations()
    if found:
        print("Canonical sessions must not retain complete native records:")
        print("\n".join(found))
        return 1
    print("Canonical sessions contain semantic data only.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
