"""目标会话文件的落盘原语。"""
from __future__ import annotations

import json
import os
from pathlib import Path


def write_jsonl(path, rows) -> None:
    """原子写 JSONL：同目录临时文件写完并 fsync 后再 replace 到目标。

    目标 Agent 可能正在扫描该目录，半截文件会被当成损坏会话。
    """
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    with temporary.open("w", encoding="utf-8") as stream:
        for row in rows:
            stream.write(json.dumps(row, ensure_ascii=False) + "\n")
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temporary, path)
