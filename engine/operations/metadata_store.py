"""Ferry 会话元数据的 SQLite 存储。"""

import json
import sqlite3
import threading
from collections.abc import Callable


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
