"""Filesystem snapshot store shared by native session implementations."""

import json
import os
import shutil
import time
from pathlib import Path

DEFAULT_BACKUP_DIR = Path.home() / ".resume-harness" / "backups"


def backup_dir() -> Path:
    """快照根目录；FERRY_BACKUP_DIR 可覆盖，测试据此隔离出 tmp 目录。"""
    override = os.environ.get("FERRY_BACKUP_DIR")
    return Path(override) if override else DEFAULT_BACKUP_DIR


def _new_dest(stem: str) -> Path:
    """纳秒级 ID 避免同一秒内「编辑前快照」和「还原前保护」互相覆盖。"""
    root = backup_dir()
    root.mkdir(parents=True, exist_ok=True)
    return root / f"{stem}-{time.time_ns()}.jsonl"


def _write_meta(dest: Path, reason_code: str, tool: str, source: str,
                extra: dict | None = None) -> None:
    dest.with_suffix(".meta.json").write_text(json.dumps(
        {"reason_code": reason_code, "tool": tool, "source": source,
         **(extra or {})},
        ensure_ascii=False))


def snapshot_file(path: Path, reason_code: str, tool: str,
                  extra: dict | None = None) -> Path:
    """复制一份文件快照，写同名 .meta.json 记录创建原因。"""
    dest = _new_dest(path.stem)
    shutil.copy(path, dest)
    _write_meta(dest, reason_code, tool, str(path), extra)
    return dest


def snapshot_payload(stem: str, payload: str, reason_code: str, tool: str,
                     source: str, extra: dict | None = None) -> Path:
    """把内存中的会话导出内容落成快照（数据库型工具使用）。"""
    dest = _new_dest(stem)
    dest.write_text(payload)
    _write_meta(dest, reason_code, tool, source, extra)
    return dest
