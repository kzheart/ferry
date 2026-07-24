"""Ferry 自有状态 SQLite 的连接与 schema 组合根。

只有 Python Engine 打开并写入此数据库。Rust 与 Ferry Runtime 必须通过
Engine RPC 访问，避免多个运行时竞争同一个事务边界。
"""
from __future__ import annotations

import sqlite3
import threading
from pathlib import Path

from .migration_history import MigrationHistoryStore
from ..operations.state_store import OperationStore
from ..organization.store import OrganizationStore
from ..organization.summary_store import SessionSummaryStore
from ..runtime.store import RuntimeSessionStore
from .session_metadata import SessionMetadataStore


SCHEMA_VERSION = 8


class StateDatabase:
    def __init__(self, path: Path, *, recover_interrupted: bool = True):
        self.path = path
        self._lock = threading.RLock()
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self._initialize()
        self.operations = OperationStore(self._connect, self._lock)
        self.organization = OrganizationStore(self._connect, self._lock)
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
        if recover_interrupted:
            self.operations.recover_interrupted()

    def _connect(self) -> sqlite3.Connection:
        connection = sqlite3.connect(
            self.path,
            timeout=30,
            isolation_level=None,
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
                        FOREIGN KEY(proposal_id)
                            REFERENCES organization_proposals(proposal_id)
                    );
                    CREATE INDEX organization_targets_identity
                        ON organization_proposal_targets(
                            tool, session_id, fingerprint
                        );
                    CREATE TABLE organization_signals (
                        sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                        proposal_id TEXT NOT NULL,
                        event TEXT NOT NULL,
                        at INTEGER NOT NULL,
                        payload_json TEXT NOT NULL,
                        FOREIGN KEY(proposal_id)
                            REFERENCES organization_proposals(proposal_id)
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
                        FOREIGN KEY(session_id)
                            REFERENCES runtime_sessions(session_id)
                            ON DELETE CASCADE
                    );
                    CREATE TABLE runtime_events (
                        session_id TEXT NOT NULL,
                        seq INTEGER NOT NULL,
                        event_json TEXT NOT NULL,
                        PRIMARY KEY(session_id, seq),
                        FOREIGN KEY(session_id)
                            REFERENCES runtime_sessions(session_id)
                            ON DELETE CASCADE
                    );
                    PRAGMA user_version = 8;
                    COMMIT;
                """)
