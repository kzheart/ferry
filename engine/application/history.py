"""Persistent migration history."""

import hashlib
import json
import os
import tempfile
from pathlib import Path


HISTORY = Path.home() / ".resume-harness" / "history.jsonl"


def append(entry: dict) -> None:
    HISTORY.parent.mkdir(parents=True, exist_ok=True)
    with HISTORY.open("a") as stream:
        stream.write(json.dumps(entry, ensure_ascii=False) + "\n")


def _raw_lines() -> list[str]:
    if not HISTORY.exists():
        return []
    return [line for line in HISTORY.read_text().splitlines() if line.strip()]


def _ids(lines: list[str]) -> list[str]:
    """行内容的哈希做稳定 id：历史文件本身没有主键，用序号会在删除后错位。

    完全相同的两行(理论上要求毫秒时间戳也撞上)追加序号区分，
    否则删一条会把重复行一起删掉。
    """
    seen: dict[str, int] = {}
    ids = []
    for line in lines:
        digest = hashlib.sha1(line.encode("utf-8")).hexdigest()[:16]
        nth = seen.get(digest, 0)
        seen[digest] = nth + 1
        ids.append(digest if nth == 0 else f"{digest}-{nth}")
    return ids


def list_entries() -> list[dict]:
    lines = _raw_lines()
    rows = [{**json.loads(line), "id": entry_id}
            for line, entry_id in zip(lines, _ids(lines))]
    return rows[::-1]


def _rewrite(lines: list[str]) -> None:
    """整文件原子替换：写临时文件再 rename，中途崩溃不会留下半截历史。"""
    HISTORY.parent.mkdir(parents=True, exist_ok=True)
    handle, temp = tempfile.mkstemp(dir=str(HISTORY.parent), suffix=".tmp")
    try:
        with os.fdopen(handle, "w") as stream:
            for line in lines:
                stream.write(line + "\n")
        os.replace(temp, HISTORY)
    except BaseException:
        Path(temp).unlink(missing_ok=True)
        raise


def delete(entry_id: str) -> dict:
    lines = _raw_lines()
    ids = _ids(lines)
    if entry_id not in ids:
        return {"deleted": False, "id": entry_id, "remaining": len(lines)}
    kept = [line for line, item in zip(lines, ids) if item != entry_id]
    _rewrite(kept)
    return {"deleted": True, "id": entry_id, "remaining": len(kept)}
