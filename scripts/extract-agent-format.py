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

from engine.adapters.claude.native_schema import extract_templates as extract_claude  # noqa: E402
from engine.adapters.codex.native_schema import extract_templates as extract_codex  # noqa: E402
from engine.adapters.opencode.native_schema import extract_templates as extract_opencode  # noqa: E402


def _jsonl(path: Path) -> list[dict]:
    return [
        json.loads(line)
        for line in path.read_text().splitlines()
        if line.strip()
    ]


def _json(path: Path):
    return json.loads(path.read_text())


EXTRACTORS = {
    "claude": (extract_claude, _jsonl),
    "codex": (extract_codex, _jsonl),
    "opencode": (extract_opencode, _json),
}


def extract(agent: str, path: Path) -> dict:
    try:
        extractor, load = EXTRACTORS[agent]
    except KeyError as error:
        raise ValueError(f"unsupported agent: {agent}") from error
    return extractor(load(path))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("agent", choices=tuple(EXTRACTORS))
    parser.add_argument("capture", type=Path)
    args = parser.parse_args()
    if not args.capture.is_file():
        parser.error(f"capture does not exist: {args.capture}")
    print(json.dumps(extract(args.agent, args.capture), ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
