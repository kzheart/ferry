from __future__ import annotations

import json
import sqlite3
from typing import Callable

from ..storage.session_metadata import (
    merge_metadata,
    metadata_entry,
    metadata_key,
)


class OrganizationStore:
    def __init__(
        self,
        connect: Callable[[], sqlite3.Connection],
        lock,
    ):
        self._connect = connect
        self._lock = lock

    @staticmethod
    def _proposal(row: sqlite3.Row) -> dict:
        return json.loads(row["proposal_json"])

    @staticmethod
    def _signal(
        connection: sqlite3.Connection,
        event: str,
        proposal: dict,
        at: int,
        **extra,
    ) -> None:
        payload = {
            "event": event,
            "proposal_id": proposal["proposal_id"],
            "generation_key": proposal["generation_key"],
            "target_count": len(proposal["targets"]),
            "at": at,
            **extra,
        }
        connection.execute(
            """
            INSERT INTO organization_signals(
                proposal_id, event, at, payload_json
            ) VALUES (?, ?, ?, ?)
            """,
            (
                proposal["proposal_id"],
                event,
                at,
                json.dumps(
                    payload,
                    ensure_ascii=False,
                    sort_keys=True,
                    separators=(",", ":"),
                ),
            ),
        )

    @staticmethod
    def _store(connection: sqlite3.Connection, proposal: dict) -> None:
        connection.execute(
            """
            UPDATE organization_proposals
            SET status = ?, proposal_json = ?, updated_at = ?
            WHERE proposal_id = ?
            """,
            (
                proposal["status"],
                json.dumps(
                    proposal,
                    ensure_ascii=False,
                    sort_keys=True,
                    separators=(",", ":"),
                ),
                proposal["updated_at"],
                proposal["proposal_id"],
            ),
        )

    @classmethod
    def _pending(cls, row: sqlite3.Row | None) -> dict:
        if row is None:
            return {"outcome": "missing"}
        proposal = cls._proposal(row)
        if proposal["status"] != "pending":
            return {"outcome": "not-pending", "proposal": proposal}
        return {"outcome": "pending", "proposal": proposal}

    def create_or_get(self, proposal: dict) -> dict:
        with self._lock, self._connect() as connection:
            connection.execute("BEGIN IMMEDIATE")
            row = connection.execute(
                """
                SELECT proposal_json FROM organization_proposals
                WHERE generation_key = ? AND status != 'stale'
                ORDER BY created_at DESC
                LIMIT 1
                """,
                (proposal["generation_key"],),
            ).fetchone()
            if row is not None:
                connection.commit()
                return {
                    "proposal": self._proposal(row),
                    "cache_hit": True,
                }
            connection.execute(
                """
                INSERT INTO organization_proposals(
                    proposal_id, generation_key, status, proposal_json,
                    created_at, updated_at
                ) VALUES (?, ?, ?, ?, ?, ?)
                """,
                (
                    proposal["proposal_id"],
                    proposal["generation_key"],
                    proposal["status"],
                    json.dumps(
                        proposal,
                        ensure_ascii=False,
                        sort_keys=True,
                        separators=(",", ":"),
                    ),
                    proposal["created_at"],
                    proposal["updated_at"],
                ),
            )
            connection.executemany(
                """
                INSERT INTO organization_proposal_targets(
                    proposal_id, position, tool, session_id, fingerprint
                ) VALUES (?, ?, ?, ?, ?)
                """,
                [
                    (
                        proposal["proposal_id"],
                        position,
                        target["tool"],
                        target["id"],
                        target["fingerprint"],
                    )
                    for position, target in enumerate(proposal["targets"])
                ],
            )
            connection.commit()
        return {"proposal": proposal, "cache_hit": False}

    def get(self, proposal_id: str) -> dict | None:
        with self._lock, self._connect() as connection:
            row = connection.execute(
                """
                SELECT proposal_json FROM organization_proposals
                WHERE proposal_id = ?
                """,
                (proposal_id,),
            ).fetchone()
        return self._proposal(row) if row is not None else None

    def list(self, status: str | None = None) -> list[dict]:
        with self._lock, self._connect() as connection:
            if status is None:
                rows = connection.execute(
                    """
                    SELECT proposal_json FROM organization_proposals
                    ORDER BY created_at DESC
                    """
                ).fetchall()
            else:
                rows = connection.execute(
                    """
                    SELECT proposal_json FROM organization_proposals
                    WHERE status = ?
                    ORDER BY created_at DESC
                    """,
                    (status,),
                ).fetchall()
        return [self._proposal(row) for row in rows]

    def modify(self, proposal: dict, now: int) -> dict:
        with self._lock, self._connect() as connection:
            connection.execute("BEGIN IMMEDIATE")
            row = connection.execute(
                """
                SELECT proposal_json FROM organization_proposals
                WHERE proposal_id = ?
                """,
                (proposal["proposal_id"],),
            ).fetchone()
            outcome = self._pending(row)
            if outcome["outcome"] != "pending":
                connection.commit()
                return outcome
            proposal["updated_at"] = now
            proposal["modified"] = True
            self._store(connection, proposal)
            self._signal(connection, "modified", proposal, now)
            connection.commit()
        return {"outcome": "modified", "proposal": proposal}

    def decide(self, proposal_id: str, decision: str, now: int) -> dict:
        with self._lock, self._connect() as connection:
            connection.execute("BEGIN IMMEDIATE")
            row = connection.execute(
                """
                SELECT proposal_json FROM organization_proposals
                WHERE proposal_id = ?
                """,
                (proposal_id,),
            ).fetchone()
            outcome = self._pending(row)
            if outcome["outcome"] != "pending":
                connection.commit()
                return outcome
            proposal = outcome["proposal"]
            if decision == "reject":
                proposal["status"] = "rejected"
                proposal["updated_at"] = now
                self._store(connection, proposal)
                self._signal(connection, "rejected", proposal, now)
                connection.commit()
                return {"outcome": "rejected", "proposal": proposal}

            stale = self._validate_targets(connection, proposal, now)
            if stale is not None:
                connection.commit()
                return stale

            applied = self._apply_metadata(connection, proposal, now)
            proposal["status"] = "approved"
            proposal["updated_at"] = now
            proposal["applied"] = applied
            self._store(connection, proposal)
            self._signal(
                connection,
                "accepted",
                proposal,
                now,
                modified=bool(proposal.get("modified")),
            )
            connection.commit()
        return {"outcome": "approved", "proposal": proposal}

    def _mark_stale(
        self,
        connection: sqlite3.Connection,
        proposal: dict,
        now: int,
        *,
        reason: str | None = None,
    ) -> None:
        proposal["status"] = "stale"
        proposal["updated_at"] = now
        self._store(connection, proposal)
        details = {"reason": reason} if reason else {}
        self._signal(connection, "stale", proposal, now, **details)

    def _validate_targets(
        self,
        connection: sqlite3.Connection,
        proposal: dict,
        now: int,
    ) -> dict | None:
        for target in proposal["targets"]:
            summary = connection.execute(
                """
                SELECT record_json FROM session_summaries
                WHERE tool = ? AND session_id = ?
                """,
                (target["tool"], target["id"]),
            ).fetchone()
            fingerprint = (
                json.loads(summary["record_json"])["fingerprint"]
                if summary is not None else None
            )
            if fingerprint != target["fingerprint"]:
                self._mark_stale(connection, proposal, now)
                return {
                    "outcome": "stale-summary",
                    "proposal": proposal,
                    "target": {
                        "tool": target["tool"],
                        "id": target["id"],
                    },
                }

        for target in proposal["targets"]:
            metadata = connection.execute(
                """
                SELECT value_json FROM session_metadata
                WHERE tool = ? AND session_id = ?
                """,
                (target["tool"], target["id"]),
            ).fetchone()
            if metadata_entry(metadata) != target["current"]:
                self._mark_stale(
                    connection,
                    proposal,
                    now,
                    reason="metadata_changed",
                )
                return {
                    "outcome": "stale-metadata",
                    "proposal": proposal,
                }
        return None

    def _apply_metadata(
        self,
        connection: sqlite3.Connection,
        proposal: dict,
        now: int,
    ) -> dict[str, dict]:
        applied = {}
        for target in proposal["targets"]:
            key = metadata_key(target["tool"], target["id"])
            entry = merge_metadata(target["current"], target["suggested"])
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
                        target["tool"],
                        target["id"],
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
                    (target["tool"], target["id"]),
                )
            applied[key] = entry
        return applied

    def invalidate(
        self,
        tool: str,
        session_id: str,
        fingerprint: str,
        now: int,
    ) -> int:
        with self._lock, self._connect() as connection:
            connection.execute("BEGIN IMMEDIATE")
            rows = connection.execute(
                """
                SELECT p.proposal_json
                FROM organization_proposals AS p
                JOIN organization_proposal_targets AS t
                    ON t.proposal_id = p.proposal_id
                WHERE p.status = 'pending'
                    AND t.tool = ?
                    AND t.session_id = ?
                    AND t.fingerprint != ?
                """,
                (tool, session_id, fingerprint),
            ).fetchall()
            for row in rows:
                proposal = self._proposal(row)
                self._mark_stale(connection, proposal, now)
            connection.commit()
        return len(rows)

    def list_signals(self, proposal_id: str | None = None) -> list[dict]:
        with self._lock, self._connect() as connection:
            if proposal_id is None:
                rows = connection.execute(
                    """
                    SELECT payload_json FROM organization_signals
                    ORDER BY sequence
                    """
                ).fetchall()
            else:
                rows = connection.execute(
                    """
                    SELECT payload_json FROM organization_signals
                    WHERE proposal_id = ?
                    ORDER BY sequence
                    """,
                    (proposal_id,),
                ).fetchall()
        return [json.loads(row["payload_json"]) for row in rows]
