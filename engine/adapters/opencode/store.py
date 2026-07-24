"""OpenCode 当前 SQLite 存储与官方 CLI 边界。"""
from __future__ import annotations

import json
import os
import sqlite3
import subprocess
import tempfile
from pathlib import Path

from ...errors import (
    AgentFormatChangedError,
    SessionNotFoundError,
    SessionStoreUnavailableError,
)
from ...system import executables
from ...system.paths import opencode_database_path


DB_PATH = opencode_database_path()

_CURRENT_DB_COLUMNS = {
    "session": {
        "id",
        "slug",
        "project_id",
        "directory",
        "path",
        "title",
        "version",
        "summary_additions",
        "summary_deletions",
        "summary_files",
        "cost",
        "tokens_input",
        "tokens_output",
        "tokens_reasoning",
        "tokens_cache_read",
        "tokens_cache_write",
        "time_created",
        "time_updated",
        "parent_id",
        "agent",
        "model",
        "permission",
        "share_url",
        "revert",
        "time_archived",
        "time_compacting",
    },
    "message": {"id", "session_id", "data", "time_created"},
    "part": {"id", "message_id", "session_id", "data", "time_created"},
}


def run_command(args, **kwargs) -> str:
    result = subprocess.run(
        executables.argv("opencode", *args),
        capture_output=True,
        text=True,
        timeout=120,
        **executables.RUN_FLAGS,
        **kwargs,
    )
    if result.returncode != 0:
        raise RuntimeError(
            f"opencode {' '.join(args)} 失败: {result.stderr[-400:]}"
        )
    return result.stdout


def export_session(session_id: str) -> dict:
    """通过临时文件接收 export，避免大响应被管道缓冲截断。"""
    descriptor, path = tempfile.mkstemp(
        prefix="rh-oc-export-", suffix=".json"
    )
    os.close(descriptor)
    try:
        with open(path, "w") as output:
            result = subprocess.run(
                executables.argv("opencode", "export", session_id),
                stdout=output,
                stderr=subprocess.PIPE,
                text=True,
                timeout=120,
                **executables.RUN_FLAGS,
            )
        if result.returncode != 0:
            raise RuntimeError(
                f"opencode export 失败: {(result.stderr or '')[-400:]}"
            )
        return json.loads(Path(path).read_text())
    finally:
        try:
            os.unlink(path)
        except OSError:
            pass


def open_database():
    """只读打开并严格校验 Ferry 当前支持的 OpenCode SQLite 结构。"""
    if not DB_PATH.exists():
        raise SessionStoreUnavailableError(
            "opencode", f"数据库不存在: {DB_PATH}"
        )
    try:
        connection = sqlite3.connect(
            f"file:{DB_PATH.resolve()}?mode=ro", uri=True
        )
        connection.row_factory = sqlite3.Row
        connection.execute("BEGIN")
    except (OSError, sqlite3.Error) as error:
        raise SessionStoreUnavailableError(
            "opencode", f"数据库不可只读访问: {error}"
        ) from error

    try:
        for table, required in _CURRENT_DB_COLUMNS.items():
            columns = {
                str(row["name"])
                for row in connection.execute(f'PRAGMA table_info("{table}")')
            }
            missing = sorted(required - columns)
            if missing:
                raise AgentFormatChangedError(
                    "opencode",
                    f"sqlite.{table}",
                    sorted(required),
                    sorted(columns),
                )
        return connection
    except AgentFormatChangedError:
        connection.close()
        raise
    except sqlite3.Error as error:
        connection.close()
        raise AgentFormatChangedError(
            "opencode",
            "sqlite.schema",
            "readable current schema",
            str(error),
        ) from error


def _session_info(row) -> dict:
    cost = row["cost"]
    if isinstance(cost, float) and cost.is_integer():
        cost = int(cost)
    info = {
        "id": row["id"],
        "slug": row["slug"],
        "projectID": row["project_id"],
        "directory": row["directory"],
        "path": row["path"] or "",
        "title": row["title"],
        "version": row["version"],
        "summary": {
            "additions": row["summary_additions"] or 0,
            "deletions": row["summary_deletions"] or 0,
            "files": row["summary_files"] or 0,
        },
        "cost": cost,
        "tokens": {
            "input": row["tokens_input"],
            "output": row["tokens_output"],
            "reasoning": row["tokens_reasoning"],
            "cache": {
                "read": row["tokens_cache_read"],
                "write": row["tokens_cache_write"],
            },
        },
        "time": {
            "created": row["time_created"],
            "updated": row["time_updated"],
        },
    }
    if row["parent_id"]:
        info["parentID"] = row["parent_id"]
    if row["agent"]:
        info["agent"] = row["agent"]
    if row["model"]:
        info["model"] = json.loads(row["model"])
    if row["permission"]:
        info["permission"] = json.loads(row["permission"])
    if row["share_url"]:
        info["share"] = {"url": row["share_url"]}
    if row["revert"]:
        info["revert"] = json.loads(row["revert"])
    if row["time_archived"]:
        info["time"]["archived"] = row["time_archived"]
    if row["time_compacting"]:
        info["time"]["compacting"] = row["time_compacting"]
    return info


def export_from_database(connection, session_id: str) -> dict | None:
    """直读 SQLite 构造当前官方 export 形状。"""
    try:
        row = connection.execute(
            "SELECT * FROM session WHERE id = ?", (session_id,)
        ).fetchone()
        if row is None:
            return None
        parts_by_message: dict[str, list] = {}
        for part in connection.execute(
            "SELECT id, message_id, session_id, data FROM part "
            "WHERE session_id = ? ORDER BY time_created, id",
            (session_id,),
        ):
            data = json.loads(part["data"])
            data.update(
                id=part["id"],
                sessionID=part["session_id"],
                messageID=part["message_id"],
            )
            parts_by_message.setdefault(part["message_id"], []).append(data)
        messages = []
        for message in connection.execute(
            "SELECT id, session_id, data FROM message "
            "WHERE session_id = ? ORDER BY time_created, id",
            (session_id,),
        ):
            data = json.loads(message["data"])
            data.update(id=message["id"], sessionID=message["session_id"])
            messages.append(
                {
                    "info": data,
                    "parts": parts_by_message.get(message["id"], []),
                }
            )
        return {"info": _session_info(row), "messages": messages}
    except (
        sqlite3.Error,
        json.JSONDecodeError,
        KeyError,
        IndexError,
    ) as error:
        raise AgentFormatChangedError(
            "opencode",
            f"session.{session_id}",
            "current session/message/part JSON",
            type(error).__name__,
        ) from error


def load_native_payload(session_id: str) -> dict:
    connection = open_database()
    try:
        payload = export_from_database(connection, session_id)
        if payload is None:
            raise SessionNotFoundError("opencode", session_id)
        return payload
    finally:
        connection.close()


def import_payload(payload: dict, session_id: str, cwd: str) -> None:
    descriptor, path = tempfile.mkstemp(
        prefix=f"rh-import-{session_id}-", suffix=".json"
    )
    os.close(descriptor)
    temporary = Path(path)
    try:
        temporary.write_text(json.dumps(payload, ensure_ascii=False))
        output = run_command(["import", str(temporary)], cwd=cwd)
        if session_id not in output:
            raise RuntimeError(f"import 结果异常: {output[-300:]}")
    finally:
        try:
            temporary.unlink()
        except OSError:
            pass


def delete_session(session_id: str, cwd: str | None = None) -> None:
    run_command(["session", "delete", session_id], cwd=cwd)
