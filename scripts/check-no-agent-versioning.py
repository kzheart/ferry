#!/usr/bin/env python3
"""Reject external session-format version selection and compatibility code."""
from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SOURCE_ROOTS = (
    ROOT / "engine",
    ROOT / "app" / "src",
    ROOT / "app" / "src-tauri" / "src",
    ROOT / "ferry-runtime" / "src",
    ROOT / "tests",
)
SOURCE_SUFFIXES = {".py", ".js", ".jsx", ".ts", ".tsx", ".rs"}
FORBIDDEN = (
    "Format" + "Profile",
    "Format" + "Registry",
    "Version" + "Range",
    "tested_" + "versions",
    "supported_" + "range",
    "output_" + "version",
    "SUPPORTED_" + "VERSION",
    "format_" + "profile",
    "resolve_" + "profile",
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
        print("External agent versioning is forbidden:")
        print("\n".join(found))
        return 1
    print("No external agent versioning found.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
