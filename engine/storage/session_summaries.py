"""会话摘要底座的 SQLite 存储。"""

import json
import sqlite3
import threading
from collections.abc import Callable


class SessionSummaryStore:
    def __init__(
        self,
        connect: Callable[[], sqlite3.Connection],
        lock: threading.RLock,
    ):
        self._connect = connect
        self._lock = lock

    def get(self, tool: str, session_id: str) -> dict | None:
        with self._lock, self._connect() as connection:
            row = connection.execute(
                """
                SELECT record_json FROM session_summaries
                WHERE tool = ? AND session_id = ?
                """,
                (tool, session_id),
            ).fetchone()
        return json.loads(row["record_json"]) if row is not None else None

    def store(self, record: dict, now: int) -> None:
        with self._lock, self._connect() as connection:
            connection.execute(
                """
                INSERT INTO session_summaries(
                    tool, session_id, record_json, updated_at
                ) VALUES (?, ?, ?, ?)
                ON CONFLICT(tool, session_id) DO UPDATE SET
                    record_json = excluded.record_json,
                    updated_at = excluded.updated_at
                """,
                (
                    record["tool"],
                    record["id"],
                    json.dumps(
                        record,
                        ensure_ascii=False,
                        sort_keys=True,
                        separators=(",", ":"),
                    ),
                    now,
                ),
            )
