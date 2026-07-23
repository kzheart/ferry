"""Ferry 自有状态 SQLite。

只有 Python Engine 打开并写入此数据库。Rust 与 Ferry Runtime 必须通过
Engine RPC 访问，避免多个运行时竞争同一个事务边界。
"""
from __future__ import annotations

import json
import sqlite3
import threading
from dataclasses import asdict
from pathlib import Path


SCHEMA_VERSION = 2


class StateDatabase:
    def __init__(self, path: Path):
        self.path = path
        self._lock = threading.RLock()
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self._initialize()

    def _connect(self) -> sqlite3.Connection:
        connection = sqlite3.connect(
            self.path, timeout=30, isolation_level=None,
        )
        connection.row_factory = sqlite3.Row
        connection.execute("PRAGMA journal_mode = WAL")
        connection.execute("PRAGMA foreign_keys = ON")
        connection.execute("PRAGMA busy_timeout = 30000")
        return connection

    def _initialize(self) -> None:
        with self._lock, self._connect() as connection:
            version = connection.execute("PRAGMA user_version").fetchone()[0]
            if version not in (0, SCHEMA_VERSION):
                raise RuntimeError(
                    f"Ferry state schema 不受支持: {version}"
                )
            if version == 0:
                connection.executescript("""
                    BEGIN IMMEDIATE;
                    CREATE TABLE operation_plans (
                        plan_id TEXT PRIMARY KEY,
                        kind TEXT NOT NULL,
                        input_json TEXT NOT NULL,
                        preview_json TEXT NOT NULL,
                        input_digest TEXT NOT NULL,
                        preview_digest TEXT NOT NULL,
                        base_revision TEXT NOT NULL,
                        document_revision TEXT,
                        created_at INTEGER NOT NULL,
                        expires_at INTEGER NOT NULL,
                        status TEXT NOT NULL,
                        result_json TEXT,
                        error_type TEXT,
                        updated_at INTEGER NOT NULL
                    );
                    CREATE TABLE operation_audit (
                        sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                        plan_id TEXT NOT NULL,
                        event TEXT NOT NULL,
                        at INTEGER NOT NULL,
                        details_json TEXT NOT NULL,
                        FOREIGN KEY(plan_id) REFERENCES operation_plans(plan_id)
                    );
                    CREATE INDEX operation_audit_plan
                        ON operation_audit(plan_id, sequence);
                    CREATE TABLE deletion_recoveries (
                        recovery_id TEXT PRIMARY KEY,
                        tool TEXT NOT NULL,
                        snapshot TEXT NOT NULL,
                        status TEXT NOT NULL,
                        created_at INTEGER NOT NULL,
                        updated_at INTEGER NOT NULL
                    );
                    PRAGMA user_version = 2;
                    COMMIT;
                """)
            connection.execute(
                """
                UPDATE operation_plans
                SET status = 'failed',
                    error_type = 'EngineRestarted',
                    updated_at = CAST(strftime('%s', 'now') AS INTEGER) * 1000
                WHERE status = 'applying'
                """
            )

    @staticmethod
    def _audit(connection: sqlite3.Connection, plan_id: str,
               event: str, at: int, details: dict | None = None) -> None:
        connection.execute(
            """
            INSERT INTO operation_audit(plan_id, event, at, details_json)
            VALUES (?, ?, ?, ?)
            """,
            (
                plan_id,
                event,
                at,
                json.dumps(
                    details or {}, ensure_ascii=False, sort_keys=True,
                    separators=(",", ":"),
                ),
            ),
        )

    def store_plan(self, plan, updated_at: int) -> None:
        record = asdict(plan)
        with self._lock, self._connect() as connection:
            connection.execute("BEGIN IMMEDIATE")
            connection.execute(
                """
                INSERT INTO operation_plans(
                    plan_id, kind, input_json, preview_json,
                    input_digest, preview_digest, base_revision,
                    document_revision, created_at, expires_at,
                    status, result_json, error_type, updated_at
                ) VALUES (
                    :plan_id, :kind, :input_json, :preview_json,
                    :input_digest, :preview_digest, :base_revision,
                    :document_revision, :created_at, :expires_at,
                    'planned', NULL, NULL, :updated_at
                )
                """,
                {**record, "updated_at": updated_at},
            )
            self._audit(
                connection, plan.plan_id, "planned", updated_at,
                {
                    "kind": plan.kind,
                    "input_digest": plan.input_digest,
                    "preview_digest": plan.preview_digest,
                    "base_revision": plan.base_revision,
                    "expires_at": plan.expires_at,
                },
            )
            connection.commit()

    def get(self, plan_id: str) -> dict | None:
        with self._lock, self._connect() as connection:
            row = connection.execute(
                "SELECT * FROM operation_plans WHERE plan_id = ?",
                (plan_id,),
            ).fetchone()
            return dict(row) if row is not None else None

    def expire(self, plan_id: str, now: int) -> None:
        self.transition(
            plan_id, "planned", "expired", now,
            error_type=None, event="expired",
        )

    def claim(self, plan_id: str, now: int) -> bool:
        return self.transition(
            plan_id, "planned", "applying", now,
            error_type=None, event="applying",
        )

    def cancel(self, plan_id: str, now: int) -> bool:
        return self.transition(
            plan_id, "planned", "cancelled", now,
            error_type=None, event="cancelled",
        )

    def transition(self, plan_id: str, expected: str, status: str, now: int,
                   *, error_type: str | None, event: str) -> bool:
        with self._lock, self._connect() as connection:
            connection.execute("BEGIN IMMEDIATE")
            changed = connection.execute(
                """
                UPDATE operation_plans
                SET status = ?, error_type = ?, updated_at = ?
                WHERE plan_id = ? AND status = ?
                """,
                (status, error_type, now, plan_id, expected),
            ).rowcount
            if changed:
                self._audit(
                    connection, plan_id, event, now,
                    {"error_type": error_type} if error_type else {},
                )
            connection.commit()
            return changed == 1

    def finish(self, plan_id: str, result_json: str,
               result_digest: str, now: int) -> None:
        with self._lock, self._connect() as connection:
            connection.execute("BEGIN IMMEDIATE")
            changed = connection.execute(
                """
                UPDATE operation_plans
                SET status = 'applied', result_json = ?,
                    error_type = NULL, updated_at = ?
                WHERE plan_id = ? AND status = 'applying'
                """,
                (result_json, now, plan_id),
            ).rowcount
            if changed != 1:
                connection.rollback()
                raise RuntimeError("Operation 状态提交失败")
            self._audit(
                connection, plan_id, "applied", now,
                {"result_digest": result_digest},
            )
            connection.commit()

    def fail(self, plan_id: str, error_type: str, now: int) -> None:
        if not self.transition(
            plan_id, "applying", "failed", now,
            error_type=error_type, event="failed",
        ):
            raise RuntimeError("Operation 失败状态提交失败")

    def audit(self, plan_id: str) -> list[dict]:
        with self._lock, self._connect() as connection:
            rows = connection.execute(
                """
                SELECT event, at, details_json
                FROM operation_audit
                WHERE plan_id = ?
                ORDER BY sequence
                """,
                (plan_id,),
            ).fetchall()
        return [
            {
                "event": row["event"],
                "at": row["at"],
                "details": json.loads(row["details_json"]),
            }
            for row in rows
        ]

    def store_recovery(self, recovery_id: str, tool: str,
                       snapshot: str, now: int) -> None:
        with self._lock, self._connect() as connection:
            connection.execute(
                """
                INSERT INTO deletion_recoveries(
                    recovery_id, tool, snapshot, status, created_at, updated_at
                ) VALUES (?, ?, ?, 'available', ?, ?)
                """,
                (recovery_id, tool, snapshot, now, now),
            )

    def get_recovery(self, recovery_id: str) -> dict | None:
        with self._lock, self._connect() as connection:
            row = connection.execute(
                """
                SELECT recovery_id, tool, snapshot, status, created_at, updated_at
                FROM deletion_recoveries
                WHERE recovery_id = ?
                """,
                (recovery_id,),
            ).fetchone()
            return dict(row) if row is not None else None

    def claim_recovery(self, recovery_id: str, now: int) -> bool:
        return self._transition_recovery(
            recovery_id, "available", "restoring", now,
        )

    def complete_recovery(self, recovery_id: str, now: int) -> bool:
        return self._transition_recovery(
            recovery_id, "restoring", "restored", now,
        )

    def release_recovery(self, recovery_id: str, now: int) -> bool:
        return self._transition_recovery(
            recovery_id, "restoring", "available", now,
        )

    def _transition_recovery(self, recovery_id: str, expected: str,
                             status: str, now: int) -> bool:
        with self._lock, self._connect() as connection:
            changed = connection.execute(
                """
                UPDATE deletion_recoveries
                SET status = ?, updated_at = ?
                WHERE recovery_id = ? AND status = ?
                """,
                (status, now, recovery_id, expected),
            ).rowcount
            return changed == 1
