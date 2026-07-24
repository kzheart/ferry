"""Ferry Runtime 会话、消息与事件的 SQLite 存储。"""

import json
import sqlite3
import threading
from collections.abc import Callable


class RuntimeSessionStore:
    def __init__(
        self,
        connect: Callable[[], sqlite3.Connection],
        lock: threading.RLock,
    ):
        self._connect = connect
        self._lock = lock

    def load_all(self) -> list[dict]:
        with self._lock, self._connect() as connection:
            sessions = connection.execute(
                "SELECT session_id, metadata_json FROM runtime_sessions"
            ).fetchall()
            result = []
            for row in sessions:
                session_id = row["session_id"]
                messages = connection.execute(
                    """
                    SELECT ordinal, message_json FROM runtime_messages
                    WHERE session_id = ? ORDER BY ordinal
                    """,
                    (session_id,),
                ).fetchall()
                events = connection.execute(
                    """
                    SELECT event_json FROM runtime_events
                    WHERE session_id = ? ORDER BY seq
                    """,
                    (session_id,),
                ).fetchall()
                result.append({
                    "state": {
                        **json.loads(row["metadata_json"]),
                        "messages": [
                            json.loads(item["message_json"])
                            for item in messages
                        ],
                    },
                    "events": [
                        json.loads(item["event_json"])
                        for item in events
                    ],
                })
        return result

    def commit(self, update: dict) -> None:
        metadata = update["metadata"]
        session_id = metadata["session_id"]
        timestamp = update["timestamp"]
        with self._lock, self._connect() as connection:
            connection.execute("BEGIN IMMEDIATE")
            existing = connection.execute(
                """
                SELECT metadata_json, created_at
                FROM runtime_sessions
                WHERE session_id = ?
                """,
                (session_id,),
            ).fetchone()
            connection.execute(
                """
                INSERT INTO runtime_sessions(
                    session_id, metadata_json, created_at, updated_at
                ) VALUES (?, ?, ?, ?)
                ON CONFLICT(session_id) DO UPDATE SET
                    metadata_json = excluded.metadata_json,
                    updated_at = excluded.updated_at
                """,
                (
                    session_id,
                    json.dumps(
                        metadata,
                        ensure_ascii=False,
                        sort_keys=True,
                        separators=(",", ":"),
                    ),
                    existing["created_at"]
                    if existing is not None
                    else timestamp,
                    timestamp,
                ),
            )
            self._insert_records(
                connection,
                "runtime_messages",
                session_id,
                "ordinal",
                update.get("messages", []),
                "message",
                "message_json",
            )
            self._insert_records(
                connection,
                "runtime_events",
                session_id,
                "seq",
                update.get("events", []),
                "event",
                "event_json",
            )
            connection.commit()

    @staticmethod
    def _insert_records(
        connection: sqlite3.Connection,
        table: str,
        session_id: str,
        key: str,
        records: list,
        value: str,
        column: str,
    ) -> None:
        for record in records:
            identifier = record[key]
            payload = json.dumps(
                record[value],
                ensure_ascii=False,
                sort_keys=True,
                separators=(",", ":"),
            )
            row = connection.execute(
                f"""
                SELECT {column} FROM {table}
                WHERE session_id = ? AND {key} = ?
                """,
                (session_id, identifier),
            ).fetchone()
            if row is not None:
                if row[column] != payload:
                    connection.rollback()
                    raise RuntimeError("Runtime 持久化记录冲突")
                continue
            connection.execute(
                f"""
                INSERT INTO {table}(session_id, {key}, {column})
                VALUES (?, ?, ?)
                """,
                (session_id, identifier, payload),
            )

    def delete(self, session_id: str) -> bool:
        with self._lock, self._connect() as connection:
            return connection.execute(
                "DELETE FROM runtime_sessions WHERE session_id = ?",
                (session_id,),
            ).rowcount == 1
