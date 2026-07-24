from __future__ import annotations

import json
import sqlite3
from dataclasses import asdict
from typing import Callable


class OperationStore:
    def __init__(
        self,
        connect: Callable[[], sqlite3.Connection],
        lock,
    ):
        self._connect = connect
        self._lock = lock

    def recover_interrupted(self) -> None:
        with self._lock, self._connect() as connection:
            connection.execute(
                """
                UPDATE operation_plans
                SET status = 'failed',
                    error_type = 'EngineRestarted',
                    updated_at = CAST(strftime('%s', 'now') AS INTEGER) * 1000
                WHERE status IN ('queued', 'applying')
                """
            )

    @staticmethod
    def _audit(
        connection: sqlite3.Connection,
        plan_id: str,
        event: str,
        at: int,
        details: dict | None = None,
    ) -> None:
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
                    details or {},
                    ensure_ascii=False,
                    sort_keys=True,
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
                connection,
                plan.plan_id,
                "planned",
                updated_at,
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
            plan_id,
            "planned",
            "expired",
            now,
            error_type=None,
            event="expired",
        )

    def claim(self, plan_id: str, now: int) -> bool:
        return self.transition(
            plan_id,
            "planned",
            "applying",
            now,
            error_type=None,
            event="applying",
        )

    def enqueue(self, plan_id: str, now: int) -> bool:
        return self.transition(
            plan_id,
            "planned",
            "queued",
            now,
            error_type=None,
            event="queued",
        )

    def claim_queued(self, plan_id: str, now: int) -> bool:
        return self.transition(
            plan_id,
            "queued",
            "applying",
            now,
            error_type=None,
            event="applying",
        )

    def cancel(self, plan_id: str, expected: str, now: int) -> bool:
        return self.transition(
            plan_id,
            expected,
            "cancelled",
            now,
            error_type=None,
            event="cancelled",
        )

    def transition(
        self,
        plan_id: str,
        expected: str,
        status: str,
        now: int,
        *,
        error_type: str | None,
        event: str,
    ) -> bool:
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
                    connection,
                    plan_id,
                    event,
                    now,
                    {"error_type": error_type} if error_type else {},
                )
            connection.commit()
            return changed == 1

    def finish(
        self,
        plan_id: str,
        result_json: str,
        result_digest: str,
        now: int,
    ) -> None:
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
                connection,
                plan_id,
                "applied",
                now,
                {"result_digest": result_digest},
            )
            connection.commit()

    def fail(self, plan_id: str, error_type: str, now: int) -> None:
        if not self.transition(
            plan_id,
            "applying",
            "failed",
            now,
            error_type=error_type,
            event="failed",
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

    def store_recovery(
        self,
        recovery_id: str,
        tool: str,
        snapshot: str,
        now: int,
    ) -> None:
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
            recovery_id,
            "available",
            "restoring",
            now,
        )

    def complete_recovery(self, recovery_id: str, now: int) -> bool:
        return self._transition_recovery(
            recovery_id,
            "restoring",
            "restored",
            now,
        )

    def release_recovery(self, recovery_id: str, now: int) -> bool:
        return self._transition_recovery(
            recovery_id,
            "restoring",
            "available",
            now,
        )

    def _transition_recovery(
        self,
        recovery_id: str,
        expected: str,
        status: str,
        now: int,
    ) -> bool:
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
