"""会话元数据的纯函数原语：键编码、行解码与补丁合并。

从 operations/metadata_store.py 抽出，好让 organization 不必反向依赖
operations 的内部模块（storage/ 被结构守护限定为只放数据库组合根）。
"""

import json
import sqlite3


def metadata_key(tool: str, session_id: str) -> str:
    return f"{tool}\0{session_id}"


def metadata_entry(row: sqlite3.Row | None) -> dict:
    return json.loads(row["value_json"]) if row is not None else {}


def merge_metadata(current: dict, patch: dict) -> dict:
    merged = {**current, **patch}
    return {
        key: value
        for key, value in merged.items()
        if value not in (None, False, "", [])
    }
