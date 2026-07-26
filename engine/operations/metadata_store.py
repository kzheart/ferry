"""Ferry 会话元数据的 SQLite 存储。

键编码/行解码/补丁合并三个纯函数已抽到 contracts.metadata，此处保留
re-export 一个版本周期。
"""

import json
import sqlite3
import threading
from collections.abc import Callable

from ..contracts.metadata import (  # noqa: F401
    merge_metadata,
    metadata_entry,
    metadata_key,
)


class SessionMetadataStore:
    def __init__(
        self,
        connect: Callable[[], sqlite3.Connection],
        lock: threading.RLock,
    ):
        self._connect = connect
        self._lock = lock

    def list_all(self) -> dict[str, dict]:
        with self._lock, self._connect() as connection:
            rows = connection.execute(
                "SELECT tool, session_id, value_json FROM session_metadata"
            ).fetchall()
        return {
            metadata_key(row["tool"], row["session_id"]): json.loads(
                row["value_json"]
            )
            for row in rows
        }

    def set(
        self,
        tool: str,
        session_id: str,
        patch: dict,
        now: int,
    ) -> dict:
        return self.compare_and_set(
            [(tool, session_id, None, patch)],
            now,
        )[metadata_key(tool, session_id)]

    def compare_and_set(
        self,
        changes: list[tuple[str, str, dict | None, dict]],
        now: int,
    ) -> dict[str, dict] | None:
        with self._lock, self._connect() as connection:
            connection.execute("BEGIN IMMEDIATE")
            current: dict[str, dict] = {}
            for tool, session_id, expected, _patch in changes:
                key = metadata_key(tool, session_id)
                row = connection.execute(
                    """
                    SELECT value_json FROM session_metadata
                    WHERE tool = ? AND session_id = ?
                    """,
                    (tool, session_id),
                ).fetchone()
                value = metadata_entry(row)
                if expected is not None and value != expected:
                    connection.rollback()
                    return None
                current[key] = value

            result: dict[str, dict] = {}
            for tool, session_id, _expected, patch in changes:
                key = metadata_key(tool, session_id)
                entry = merge_metadata(current[key], patch)
                if entry:
                    connection.execute(
                        """
                        INSERT INTO session_metadata(
                            tool, session_id, value_json, updated_at
                        ) VALUES (?, ?, ?, ?)
                        ON CONFLICT(tool, session_id) DO UPDATE SET
                            value_json = excluded.value_json,
                            updated_at = excluded.updated_at
                        """,
                        (
                            tool,
                            session_id,
                            json.dumps(
                                entry,
                                ensure_ascii=False,
                                sort_keys=True,
                                separators=(",", ":"),
                            ),
                            now,
                        ),
                    )
                else:
                    connection.execute(
                        """
                        DELETE FROM session_metadata
                        WHERE tool = ? AND session_id = ?
                        """,
                        (tool, session_id),
                    )
                result[key] = entry
            connection.commit()
            return result
