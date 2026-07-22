"""会话元数据 sidecar:重命名 / 置顶 / 归档 / 标签,独立于会话文件存储。

按会话 id 记录,会话文件本身不做任何改写;条目全部字段清空时自动移除。
"""

import json
import os
import tempfile
import threading
from pathlib import Path

META = Path.home() / ".resume-harness" / "session-meta.json"
_LOCK = threading.RLock()


def _load() -> dict:
    try:
        return json.loads(META.read_text())
    except (OSError, json.JSONDecodeError):
        return {}


def list_all() -> dict:
    return _load()


def _write(data: dict) -> None:
    META.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix="session-meta-", suffix=".tmp",
                                     dir=META.parent)
    try:
        with os.fdopen(fd, "w") as stream:
            json.dump(data, stream, ensure_ascii=False, indent=1)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, META)
        os.chmod(META, 0o600)
    finally:
        try:
            os.unlink(temporary)
        except OSError:
            pass


def _merged(data: dict, sid: str, patch: dict) -> dict:
    entry = {**data.get(sid, {}), **patch}
    entry = {k: v for k, v in entry.items() if v not in (None, False, "", [])}
    if entry:
        data[sid] = entry
    else:
        data.pop(sid, None)
    return entry


def set_entry(sid: str, patch: dict) -> dict:
    with _LOCK:
        data = _load()
        entry = _merged(data, sid, patch)
        _write(data)
        return entry


def compare_and_set_entry(sid: str, expected: dict, patch: dict) -> dict:
    from ..domain.errors import ConcurrentModificationError

    with _LOCK:
        data = _load()
        if data.get(sid, {}) != expected:
            raise ConcurrentModificationError("会话元数据在审批后已变化")
        entry = _merged(data, sid, patch)
        _write(data)
        if _load().get(sid, {}) != entry:
            raise RuntimeError("元数据写入后校验失败")
        return entry
