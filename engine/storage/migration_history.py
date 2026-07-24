"""Ferry 迁移历史的 SQLite 存储。"""

import json
import sqlite3
import threading
from collections.abc import Callable


class MigrationHistoryStore:
    def __init__(
        self,
        connect: Callable[[], sqlite3.Connection],
        lock: threading.RLock,
    ):
        self._connect = connect
        self._lock = lock

    def append(self, history_id: str, entry: dict) -> None:
        with self._lock, self._connect() as connection:
            connection.execute(
                """
                INSERT INTO migration_history(history_id, entry_json)
                VALUES (?, ?)
                """,
                (
                    history_id,
                    json.dumps(
                        entry,
                        ensure_ascii=False,
                        sort_keys=True,
                        separators=(",", ":"),
                    ),
                ),
            )

    def list_all(self) -> list[dict]:
        with self._lock, self._connect() as connection:
            rows = connection.execute(
                """
                SELECT history_id, entry_json
                FROM migration_history
                ORDER BY sequence DESC
                """
            ).fetchall()
        return [
            {**json.loads(row["entry_json"]), "id": row["history_id"]}
            for row in rows
        ]

    def delete(self, history_id: str) -> dict:
        with self._lock, self._connect() as connection:
            connection.execute("BEGIN IMMEDIATE")
            deleted = connection.execute(
                "DELETE FROM migration_history WHERE history_id = ?",
                (history_id,),
            ).rowcount == 1
            remaining = connection.execute(
                "SELECT COUNT(*) FROM migration_history"
            ).fetchone()[0]
            connection.commit()
        return {
            "deleted": deleted,
            "id": history_id,
            "remaining": remaining,
        }
