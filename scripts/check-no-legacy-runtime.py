#!/usr/bin/env python3
"""Reject deleted runtime compatibility and duplicate-operation concepts."""
from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SOURCE_ROOTS = (
    ROOT / "engine",
    ROOT / "app" / "src",
    ROOT / "app" / "src-tauri" / "src",
    ROOT / "agent-runtime" / "src",
)
SOURCE_SUFFIXES = {".py", ".js", ".jsx", ".ts", ".tsx", ".rs"}
FORBIDDEN = (
    "from_" + "legacy",
    "legacy_" + "output",
    "canonical" + "ToolResult",
    "canonical_" + "blocks",
    "authoring_" + "preview",
    "authoring_" + "apply",
    "save_" + "as",
    "migration_" + "handoff",
    "dry_" + "run",
    "pkgutil." + "iter_modules",
    "ToolPlugin." + "require",
    "in" + "Tauri",
    "/api/" + "rpc",
    "engine" + "Bridge",
)


def violations() -> list[str]:
    found = []
    for source_root in SOURCE_ROOTS:
        for path in source_root.rglob("*"):
            if not path.is_file() or path.suffix not in SOURCE_SUFFIXES:
                continue
            text = path.read_text(errors="replace")
            for line_number, line in enumerate(text.splitlines(), 1):
                for term in FORBIDDEN:
                    if term in line:
                        found.append(
                            f"{path.relative_to(ROOT)}:{line_number}: {term}"
                        )
    return found


def main() -> int:
    found = violations()
    if found:
        print("Deleted runtime compatibility paths were reintroduced:")
        print("\n".join(found))
        return 1
    print("No deleted runtime compatibility paths found.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
