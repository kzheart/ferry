"""Ferry 自有状态 SQLite。

只有 Python Engine 打开并写入此数据库。Rust 与 Ferry Runtime 必须通过
Engine RPC 访问，避免多个运行时竞争同一个事务边界。
"""
from __future__ import annotations

import json
import sqlite3
import threading
from pathlib import Path

from .migration_history import MigrationHistoryStore
from .operation_store import OperationStore
from .runtime_sessions import RuntimeSessionStore
from .session_metadata import (
    SessionMetadataStore,
    merge_metadata,
    metadata_entry,
    metadata_key,
)
from .session_summaries import SessionSummaryStore


SCHEMA_VERSION = 8


class StateDatabase:
    def __init__(self, path: Path, *, recover_interrupted: bool = True):
        self.path = path
        self._lock = threading.RLock()
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self._initialize()
        self.operations = OperationStore(self._connect, self._lock)
        if recover_interrupted:
            self.operations.recover_interrupted()
        self.runtime_sessions = RuntimeSessionStore(
            self._connect,
            self._lock,
        )
        self.metadata = SessionMetadataStore(self._connect, self._lock)
        self.summaries = SessionSummaryStore(self._connect, self._lock)
        self.migration_history = MigrationHistoryStore(
            self._connect,
            self._lock,
        )

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
                    CREATE TABLE session_metadata (
                        tool TEXT NOT NULL,
                        session_id TEXT NOT NULL,
                        value_json TEXT NOT NULL,
                        updated_at INTEGER NOT NULL,
                        PRIMARY KEY(tool, session_id)
                    );
                    CREATE TABLE migration_history (
                        sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                        history_id TEXT NOT NULL UNIQUE,
                        entry_json TEXT NOT NULL
                    );
                    CREATE TABLE session_summaries (
                        tool TEXT NOT NULL,
                        session_id TEXT NOT NULL,
                        record_json TEXT NOT NULL,
                        updated_at INTEGER NOT NULL,
                        PRIMARY KEY(tool, session_id)
                    );
                    CREATE TABLE organization_proposals (
                        proposal_id TEXT PRIMARY KEY,
                        generation_key TEXT NOT NULL,
                        status TEXT NOT NULL,
                        proposal_json TEXT NOT NULL,
                        created_at INTEGER NOT NULL,
                        updated_at INTEGER NOT NULL
                    );
                    CREATE INDEX organization_proposals_generation
                        ON organization_proposals(generation_key, status);
                    CREATE TABLE organization_proposal_targets (
                        proposal_id TEXT NOT NULL,
                        position INTEGER NOT NULL,
                        tool TEXT NOT NULL,
                        session_id TEXT NOT NULL,
                        fingerprint TEXT NOT NULL,
                        PRIMARY KEY(proposal_id, position),
                        UNIQUE(proposal_id, tool, session_id),
                        FOREIGN KEY(proposal_id) REFERENCES organization_proposals(proposal_id)
                    );
                    CREATE INDEX organization_targets_identity
                        ON organization_proposal_targets(tool, session_id, fingerprint);
                    CREATE TABLE organization_signals (
                        sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                        proposal_id TEXT NOT NULL,
                        event TEXT NOT NULL,
                        at INTEGER NOT NULL,
                        payload_json TEXT NOT NULL,
                        FOREIGN KEY(proposal_id) REFERENCES organization_proposals(proposal_id)
                    );
                    CREATE TABLE runtime_sessions (
                        session_id TEXT PRIMARY KEY,
                        metadata_json TEXT NOT NULL,
                        created_at TEXT NOT NULL,
                        updated_at TEXT NOT NULL
                    );
                    CREATE INDEX runtime_sessions_recent
                        ON runtime_sessions(updated_at DESC);
                    CREATE TABLE runtime_messages (
                        session_id TEXT NOT NULL,
                        ordinal INTEGER NOT NULL,
                        message_json TEXT NOT NULL,
                        PRIMARY KEY(session_id, ordinal),
                        FOREIGN KEY(session_id) REFERENCES runtime_sessions(session_id)
                            ON DELETE CASCADE
                    );
                    CREATE TABLE runtime_events (
                        session_id TEXT NOT NULL,
                        seq INTEGER NOT NULL,
                        event_json TEXT NOT NULL,
                        PRIMARY KEY(session_id, seq),
                        FOREIGN KEY(session_id) REFERENCES runtime_sessions(session_id)
                            ON DELETE CASCADE
                    );
                    PRAGMA user_version = 8;
                    COMMIT;
                """)
    @staticmethod
    def _organization_proposal(row: sqlite3.Row) -> dict:
        return json.loads(row["proposal_json"])

    @staticmethod
    def _organization_signal(connection: sqlite3.Connection, event: str,
                             proposal: dict, at: int, **extra) -> None:
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
                proposal["proposal_id"], event, at,
                json.dumps(
                    payload, ensure_ascii=False, sort_keys=True,
                    separators=(",", ":"),
                ),
            ),
        )

    @staticmethod
    def _store_organization_proposal(connection: sqlite3.Connection,
                                     proposal: dict) -> None:
        connection.execute(
            """
            UPDATE organization_proposals
            SET status = ?, proposal_json = ?, updated_at = ?
            WHERE proposal_id = ?
            """,
            (
                proposal["status"],
                json.dumps(
                    proposal, ensure_ascii=False, sort_keys=True,
                    separators=(",", ":"),
                ),
                proposal["updated_at"],
                proposal["proposal_id"],
            ),
        )

    @staticmethod
    def _organization_pending(row: sqlite3.Row | None) -> dict:
        if row is None:
            return {"outcome": "missing"}
        proposal = StateDatabase._organization_proposal(row)
        if proposal["status"] != "pending":
            return {"outcome": "not-pending", "proposal": proposal}
        return {"outcome": "pending", "proposal": proposal}

    def create_or_get_organization_proposal(self, proposal: dict) -> dict:
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
                return {"proposal": self._organization_proposal(row), "cache_hit": True}
            connection.execute(
                """
                INSERT INTO organization_proposals(
                    proposal_id, generation_key, status, proposal_json,
                    created_at, updated_at
                ) VALUES (?, ?, ?, ?, ?, ?)
                """,
                (
                    proposal["proposal_id"], proposal["generation_key"],
                    proposal["status"], json.dumps(
                        proposal, ensure_ascii=False, sort_keys=True,
                        separators=(",", ":"),
                    ),
                    proposal["created_at"], proposal["updated_at"],
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
                        proposal["proposal_id"], position, target["tool"],
                        target["id"], target["fingerprint"],
                    )
                    for position, target in enumerate(proposal["targets"])
                ],
            )
            connection.commit()
        return {"proposal": proposal, "cache_hit": False}

    def get_organization_proposal(self, proposal_id: str) -> dict | None:
        with self._lock, self._connect() as connection:
            row = connection.execute(
                """
                SELECT proposal_json FROM organization_proposals
                WHERE proposal_id = ?
                """,
                (proposal_id,),
            ).fetchone()
        return self._organization_proposal(row) if row is not None else None

    def list_organization_proposals(self, status: str | None = None) -> list[dict]:
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
        return [self._organization_proposal(row) for row in rows]

    def modify_organization_proposal(self, proposal: dict, now: int) -> dict:
        with self._lock, self._connect() as connection:
            connection.execute("BEGIN IMMEDIATE")
            row = connection.execute(
                """
                SELECT proposal_json FROM organization_proposals
                WHERE proposal_id = ?
                """,
                (proposal["proposal_id"],),
            ).fetchone()
            outcome = self._organization_pending(row)
            if outcome["outcome"] != "pending":
                connection.commit()
                return outcome
            proposal["updated_at"] = now
            proposal["modified"] = True
            self._store_organization_proposal(connection, proposal)
            self._organization_signal(connection, "modified", proposal, now)
            connection.commit()
        return {"outcome": "modified", "proposal": proposal}

    def decide_organization_proposal(self, proposal_id: str, decision: str,
                                     now: int) -> dict:
        with self._lock, self._connect() as connection:
            connection.execute("BEGIN IMMEDIATE")
            row = connection.execute(
                """
                SELECT proposal_json FROM organization_proposals
                WHERE proposal_id = ?
                """,
                (proposal_id,),
            ).fetchone()
            outcome = self._organization_pending(row)
            if outcome["outcome"] != "pending":
                connection.commit()
                return outcome
            proposal = outcome["proposal"]
            if decision == "reject":
                proposal["status"] = "rejected"
                proposal["updated_at"] = now
                self._store_organization_proposal(connection, proposal)
                self._organization_signal(connection, "rejected", proposal, now)
                connection.commit()
                return {"outcome": "rejected", "proposal": proposal}

            for target in proposal["targets"]:
                summary = connection.execute(
                    """
                    SELECT record_json FROM session_summaries
                    WHERE tool = ? AND session_id = ?
                    """,
                    (target["tool"], target["id"]),
                ).fetchone()
                current_fingerprint = (
                    json.loads(summary["record_json"])["fingerprint"]
                    if summary is not None else None
                )
                if current_fingerprint != target["fingerprint"]:
                    proposal["status"] = "stale"
                    proposal["updated_at"] = now
                    self._store_organization_proposal(connection, proposal)
                    self._organization_signal(connection, "stale", proposal, now)
                    connection.commit()
                    return {
                        "outcome": "stale-summary", "proposal": proposal,
                        "target": {"tool": target["tool"], "id": target["id"]},
                    }

            current_metadata: dict[str, dict] = {}
            for target in proposal["targets"]:
                key = metadata_key(target["tool"], target["id"])
                metadata = connection.execute(
                    """
                    SELECT value_json FROM session_metadata
                    WHERE tool = ? AND session_id = ?
                    """,
                    (target["tool"], target["id"]),
                ).fetchone()
                value = metadata_entry(metadata)
                if value != target["current"]:
                    proposal["status"] = "stale"
                    proposal["updated_at"] = now
                    self._store_organization_proposal(connection, proposal)
                    self._organization_signal(
                        connection, "stale", proposal, now,
                        reason="metadata_changed",
                    )
                    connection.commit()
                    return {"outcome": "stale-metadata", "proposal": proposal}
                current_metadata[key] = value

            applied: dict[str, dict] = {}
            for target in proposal["targets"]:
                key = metadata_key(target["tool"], target["id"])
                entry = merge_metadata(
                    current_metadata[key], target["suggested"],
                )
                if entry:
                    connection.execute(
                        """
                        INSERT INTO session_metadata(tool, session_id, value_json, updated_at)
                        VALUES (?, ?, ?, ?)
                        ON CONFLICT(tool, session_id) DO UPDATE SET
                            value_json = excluded.value_json,
                            updated_at = excluded.updated_at
                        """,
                        (
                            target["tool"], target["id"], json.dumps(
                                entry, ensure_ascii=False, sort_keys=True,
                                separators=(",", ":"),
                            ), now,
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
            proposal["status"] = "approved"
            proposal["updated_at"] = now
            proposal["applied"] = applied
            self._store_organization_proposal(connection, proposal)
            self._organization_signal(
                connection, "accepted", proposal, now,
                modified=bool(proposal.get("modified")),
            )
            connection.commit()
        return {"outcome": "approved", "proposal": proposal}

    def invalidate_organization_proposals(self, tool: str, session_id: str,
                                          fingerprint: str, now: int) -> int:
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
                proposal = self._organization_proposal(row)
                proposal["status"] = "stale"
                proposal["updated_at"] = now
                self._store_organization_proposal(connection, proposal)
                self._organization_signal(connection, "stale", proposal, now)
            connection.commit()
        return len(rows)

    def list_organization_signals(self, proposal_id: str | None = None) -> list[dict]:
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
