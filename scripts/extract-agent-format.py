#!/usr/bin/env python3
"""Extract a candidate declarative format template from a native fixture."""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from engine.adapters.claude.formats import extract_templates as extract_claude
from engine.adapters.codex.formats import extract_templates as extract_codex
from engine.adapters.opencode.formats import extract_templates as extract_opencode


def _jsonl(path: Path) -> list[dict]:
    return [
        json.loads(line)
        for line in path.read_text().splitlines()
        if line.strip()
    ]


def extract(agent: str, path: Path) -> dict:
    if agent == "claude":
        return extract_claude(_jsonl(path))
    if agent == "codex":
        return extract_codex(_jsonl(path))
    if agent == "opencode":
        return extract_opencode(json.loads(path.read_text()))
    raise ValueError(f"unsupported agent: {agent}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("agent", choices=("claude", "codex", "opencode"))
    parser.add_argument("capture", type=Path)
    args = parser.parse_args()
    if not args.capture.is_file():
        parser.error(f"capture does not exist: {args.capture}")
    print(json.dumps(extract(args.agent, args.capture), ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
