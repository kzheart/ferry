#!/usr/bin/env python3
"""Reject deleted runtime compatibility and duplicate-operation concepts."""
from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SOURCE_ROOTS = (
    ROOT / "engine",
    ROOT / "app" / "src",
    ROOT / "app" / "src-tauri" / "src",
    ROOT / "ferry-runtime" / "src",
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
    "Tool" + "Plugin",
    "ToolPlugin." + "require",
    "in" + "Tauri",
    "/api/" + "rpc",
    "engine" + "Bridge",
)
FORBIDDEN_PATHS = (
    ROOT / "app/scripts/check-tool-contract.mjs",
    ROOT / "engine/adapters/base/plugin.py",
    ROOT / "engine/adapters/claude/plugin.py",
    ROOT / "engine/adapters/codex/plugin.py",
    ROOT / "engine/adapters/opencode/plugin.py",
)


def violations() -> list[str]:
    found = [
        f"{path.relative_to(ROOT)}: deleted adapter plugin path"
        for path in FORBIDDEN_PATHS
        if path.exists()
    ]
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
