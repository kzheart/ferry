"""Persistent migration history."""

import json
from pathlib import Path


HISTORY = Path.home() / ".resume-harness" / "history.jsonl"


def append(entry: dict) -> None:
    HISTORY.parent.mkdir(parents=True, exist_ok=True)
    with HISTORY.open("a") as stream:
        stream.write(json.dumps(entry, ensure_ascii=False) + "\n")


def list_entries() -> list[dict]:
    if not HISTORY.exists():
        return []
    rows = [json.loads(line) for line in HISTORY.read_text().splitlines()
            if line.strip()]
    return rows[::-1]
