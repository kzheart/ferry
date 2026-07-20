"""Filesystem snapshot store shared by native session implementations."""

import json
import shutil
import time
from pathlib import Path

BACKUP_DIR = Path.home() / ".resume-harness" / "backups"


def snapshot_file(path: Path, reason_code: str, tool: str,
                  extra: dict | None = None) -> Path:
    """复制一份文件快照，写同名 .meta.json 记录创建原因。

    纳秒级 ID 避免同一秒内「编辑前快照」和「还原前保护」互相覆盖。
    """
    BACKUP_DIR.mkdir(parents=True, exist_ok=True)
    dest = BACKUP_DIR / f"{path.stem}-{time.time_ns()}.jsonl"
    shutil.copy(path, dest)
    dest.with_suffix(".meta.json").write_text(json.dumps(
        {"reason_code": reason_code, "tool": tool, "source": str(path),
         **(extra or {})},
        ensure_ascii=False))
    return dest


def snapshot_payload(stem: str, payload: str, reason_code: str, tool: str,
                     source: str) -> Path:
    """把内存中的会话导出内容落成快照（数据库型工具使用）。"""
    BACKUP_DIR.mkdir(parents=True, exist_ok=True)
    dest = BACKUP_DIR / f"{stem}-{time.time_ns()}.jsonl"
    dest.write_text(payload)
    dest.with_suffix(".meta.json").write_text(json.dumps(
        {"reason_code": reason_code, "tool": tool, "source": source},
        ensure_ascii=False))
    return dest
