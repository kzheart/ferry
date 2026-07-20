"""会话元数据 sidecar:重命名 / 置顶 / 归档 / 标签,独立于会话文件存储。

按会话 id 记录,会话文件本身不做任何改写;条目全部字段清空时自动移除。
"""

import json
from pathlib import Path

META = Path.home() / ".resume-harness" / "session-meta.json"


def _load() -> dict:
    try:
        return json.loads(META.read_text())
    except (OSError, json.JSONDecodeError):
        return {}


def list_all() -> dict:
    return _load()


def set_entry(sid: str, patch: dict) -> dict:
    data = _load()
    entry = {**data.get(sid, {}), **patch}
    entry = {k: v for k, v in entry.items() if v not in (None, False, "", [])}
    if entry:
        data[sid] = entry
    else:
        data.pop(sid, None)
    META.parent.mkdir(parents=True, exist_ok=True)
    META.write_text(json.dumps(data, ensure_ascii=False, indent=1))
    return entry
