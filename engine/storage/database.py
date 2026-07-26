"""Ferry 自有状态 SQLite 的连接与 schema 组合根。

只有 Python Engine 打开并写入此数据库。Rust 与 Ferry Runtime 必须通过
Engine RPC 访问，避免多个运行时竞争同一个事务边界。
"""
from __future__ import annotations

import hashlib
import json
import sqlite3
import threading
import time
from pathlib import Path

from ..operations.history_store import MigrationHistoryStore
from ..operations.metadata_store import SessionMetadataStore
from ..operations.state_store import OperationStore
from ..organization.store import OrganizationStore
from ..organization.summary_store import SessionSummaryStore
from ..runtime.store import RuntimeSessionStore


SCHEMA_VERSION = 8


def now_ms() -> int:
    return int(time.time() * 1000)


def canonical_json(value) -> str:
    return json.dumps(
        value,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
        allow_nan=False,
    )


def digest_json(value_json: str) -> str:
    return hashlib.sha256(value_json.encode()).hexdigest()


def digest_value(value) -> str:
    return digest_json(canonical_json(value))


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


_instances: dict[Path, StateDatabase] = {}
_instances_lock = threading.Lock()


def get_state_database(
    path: Path,
    *,
    recover_interrupted: bool = True,
) -> StateDatabase:
    """按库文件复用 StateDatabase 实例。

    每次 RPC 都新建一个的话,mkdir、schema 探测与六个 Store 都要重建一遍,
    而 Ask Ferry 每提交一次就走一趟。连接本身仍是每次操作现开(见 `_connect`),
    所以复用实例不会把 sqlite3 连接跨线程共享。
    """
    key = Path(path)
    with _instances_lock:
        existing = _instances.get(key)
        if existing is not None:
            return existing
        database = StateDatabase(key, recover_interrupted=recover_interrupted)
        _instances[key] = database
        return database


def state_database_path(ports) -> Path:
    """EngineContext 下 Ferry 自有状态库的位置。"""
    return Path(ports.snapshot_dir()) / "ferry-state.sqlite3"


def state_database(ports) -> StateDatabase:
    """按 EngineContext 打开状态库。

    元数据/历史/摘要等读写路径不能把正在执行的 Operation 标为中断；该恢复
    动作仅由 OperationService 重启时执行。
    """
    return StateDatabase(state_database_path(ports), recover_interrupted=False)


def cached_state_database(ports) -> StateDatabase:
    """同 `state_database`，但复用实例（见 `get_state_database`）。"""
    return get_state_database(state_database_path(ports),
                              recover_interrupted=False)
