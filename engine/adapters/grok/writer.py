"""Create current Grok bundles and maintain its schema-v4 search index."""
from __future__ import annotations

import json
import os
import re
import shutil
import socket
import sqlite3
import subprocess
import sys
import time
import uuid
from pathlib import Path
from urllib.parse import quote

from ...sessions.model import tool_result_text
from ...system.paths import grok_home
from ..shared.writing import write_jsonl
from .blake3 import blake3_hex
from .reader import read

# BLAKE3 实现已迁往 grok/blake3.py；保留旧名给既有测试的导入。
_blake3 = blake3_hex

SEARCH_SCHEMA_VERSION = "4"
SEARCH_COLUMNS = (
    "session_id", "cwd", "updated_at", "title", "content",
    "content_hash", "last_indexed_offset",
)
def _create_search_schema(database):
    statements = (
        """CREATE TABLE meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        )""",
        """CREATE TABLE session_docs (
            session_id TEXT PRIMARY KEY,
            cwd TEXT NOT NULL,
            updated_at INTEGER NOT NULL,
            title TEXT NOT NULL,
            content TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            last_indexed_offset INTEGER NOT NULL DEFAULT 0
        )""",
        """CREATE VIRTUAL TABLE session_docs_fts USING fts5(
            title, content, content='session_docs', content_rowid='rowid'
        )""",
        """CREATE TRIGGER session_docs_ai AFTER INSERT ON session_docs BEGIN
            INSERT INTO session_docs_fts(rowid, title, content)
            VALUES (new.rowid, new.title, new.content);
        END""",
        """CREATE TRIGGER session_docs_ad AFTER DELETE ON session_docs BEGIN
            INSERT INTO session_docs_fts(session_docs_fts, rowid, title, content)
            VALUES ('delete', old.rowid, old.title, old.content);
        END""",
        """CREATE TRIGGER session_docs_au AFTER UPDATE ON session_docs BEGIN
            INSERT INTO session_docs_fts(session_docs_fts, rowid, title, content)
            VALUES ('delete', old.rowid, old.title, old.content);
            INSERT INTO session_docs_fts(rowid, title, content)
            VALUES (new.rowid, new.title, new.content);
        END""",
    )
    for statement in statements:
        database.execute(statement)
    database.execute(
        "INSERT INTO meta(key, value) VALUES (?, ?)",
        ("session_search_schema_version", SEARCH_SCHEMA_VERSION),
    )


def _normalized_sql(value):
    return re.sub(r"\s+", " ", str(value or "")).lower()


def _validate_search_schema(database):
    try:
        version = database.execute(
            "SELECT value FROM meta WHERE key=?",
            ("session_search_schema_version",),
        ).fetchone()
        columns = tuple(row[1] for row in database.execute(
            "PRAGMA table_info(session_docs)"
        ))
        rows = {
            (kind, name): _normalized_sql(sql)
            for kind, name, sql in database.execute(
                """SELECT type, name, sql FROM sqlite_schema
                   WHERE name IN (
                       'session_docs_fts', 'session_docs_ai',
                       'session_docs_ad', 'session_docs_au'
                   )"""
            )
        }
    except sqlite3.Error as error:
        raise RuntimeError(
            "Grok session_search.sqlite 结构或版本不受支持"
        ) from error
    fts = rows.get(("table", "session_docs_fts"), "")
    trigger_fragments = {
        "session_docs_ai": ("after insert on session_docs",
                            "insert into session_docs_fts"),
        "session_docs_ad": ("after delete on session_docs",
                            "values ('delete', old.rowid"),
        "session_docs_au": ("after update on session_docs",
                            "values ('delete', old.rowid",
                            "values (new.rowid"),
    }
    valid_triggers = all(
        all(fragment in rows.get(("trigger", name), "")
            for fragment in fragments)
        for name, fragments in trigger_fragments.items()
    )
    if (
        version != (SEARCH_SCHEMA_VERSION,)
        or columns != SEARCH_COLUMNS
        or "using fts5" not in fts
        or "content='session_docs'" not in fts
        or "content_rowid='rowid'" not in fts
        or not valid_triggers
    ):
        raise RuntimeError("Grok session_search.sqlite 结构或版本不受支持")


def _host_discriminator():
    value = re.sub(r"[^a-z0-9]", "-", socket.gethostname().lower())[:24]
    return value.strip("-") or None


def _search_database_path(sessions_root):
    base = sessions_root / "session_search.sqlite"
    host = _host_discriminator()
    per_host = (
        base.with_name(f"session_search.h-{host}.sqlite") if host else None
    )
    override = os.environ.get("GROK_SQLITE_JOURNAL_MODE", "").lower()
    if override == "truncate" or (
        not override and _is_network_filesystem(sessions_root)
    ):
        return per_host or base
    if per_host and per_host.exists() and not base.exists():
        return per_host
    return base


def _is_network_filesystem(path):
    command = (
        ["/usr/bin/stat", "-f", "%T", str(path)]
        if sys.platform == "darwin"
        else ["stat", "-f", "-c", "%T", str(path)]
    )
    try:
        result = subprocess.run(
            command, capture_output=True, text=True, timeout=3,
        )
    except (OSError, subprocess.TimeoutExpired):
        return False
    filesystem = result.stdout.strip().lower()
    return any(marker in filesystem for marker in (
        "nfs", "smb", "cifs", "afp", "webdav", "sshfs",
    ))


def _reject_database_symlink(path):
    if path.is_symlink():
        raise RuntimeError("拒绝通过符号链接维护 Grok 搜索索引")


def _backup_database(database, path):
    backup = path.with_name(path.name + ".ferry-backup")
    _reject_database_symlink(backup)
    temporary = backup.with_name(
        f".{backup.name}.{os.getpid()}.{uuid.uuid4().hex}.tmp"
    )
    target = sqlite3.connect(temporary)
    try:
        database.backup(target)
    finally:
        target.close()
    os.chmod(temporary, 0o600)
    os.replace(temporary, backup)
    return backup


def _updated_at(summary):
    try:
        from datetime import datetime

        return int(datetime.fromisoformat(
            str(summary.get("updated_at")).replace("Z", "+00:00")
        ).timestamp())
    except (ValueError, TypeError):
        return int(time.time())


def _index_content(bundle):
    parts = []
    for line in (bundle / "chat_history.jsonl").read_text().splitlines():
        if not line.strip():
            continue
        row = json.loads(line)
        content = row.get("content")
        if isinstance(content, str):
            parts.append(content)
        elif isinstance(content, list):
            parts.extend(
                str(item.get("text") or "") for item in content
                if isinstance(item, dict) and item.get("type") == "text"
            )
        for tool in row.get("tool_calls") or []:
            if isinstance(tool, dict):
                parts.append(str(tool.get("name") or ""))
                parts.append(json.dumps(
                    tool.get("arguments") or {}, ensure_ascii=False,
                    sort_keys=True,
                ))
    return "\n".join(part for part in parts if part)


def _index_doc(bundle):
    summary = json.loads((bundle / "summary.json").read_text())
    sid, cwd = summary["info"]["id"], summary["info"]["cwd"]
    title = str(summary.get("generated_title")
                or summary.get("session_summary") or "")
    content = _index_content(bundle)
    digest = blake3_hex(
        title.encode() + b"\0" + content.encode()
    )
    return (
        sid, cwd, _updated_at(summary), title, content, digest,
        (bundle / "updates.jsonl").stat().st_size,
    )


def index_bundles(bundles, sessions_root):
    sessions_root = Path(sessions_root)
    sessions_root.mkdir(parents=True, exist_ok=True)
    database_path = _search_database_path(sessions_root)
    _reject_database_symlink(database_path)
    existed = database_path.exists()
    database = sqlite3.connect(database_path, timeout=5)
    try:
        database.execute("PRAGMA busy_timeout=5000")
        if existed:
            _validate_search_schema(database)
            _backup_database(database, database_path)
        database.execute("BEGIN IMMEDIATE")
        if not existed:
            _create_search_schema(database)
            _validate_search_schema(database)
        for doc in (_index_doc(Path(bundle)) for bundle in bundles):
            database.execute(
                """INSERT INTO session_docs(
                       session_id,cwd,updated_at,title,content,content_hash,
                       last_indexed_offset
                   ) VALUES(?,?,?,?,?,?,?)
                   ON CONFLICT(session_id) DO UPDATE SET
                       cwd=excluded.cwd, updated_at=excluded.updated_at,
                       title=excluded.title, content=excluded.content,
                       content_hash=excluded.content_hash,
                       last_indexed_offset=excluded.last_indexed_offset""",
                doc,
            )
        database.commit()
    except Exception:
        database.rollback()
        raise
    finally:
        try:
            database.close()
        except sqlite3.Error:
            pass
    return database_path


def index_bundle(bundle: Path, sessions_root: Path):
    return index_bundles((bundle,), sessions_root)


def delete_index_rows(session_ids, sessions_root):
    database_path = _search_database_path(Path(sessions_root))
    if not database_path.exists():
        return
    _reject_database_symlink(database_path)
    database = sqlite3.connect(database_path, timeout=5)
    try:
        database.execute("PRAGMA busy_timeout=5000")
        _validate_search_schema(database)
        _backup_database(database, database_path)
        database.execute("BEGIN IMMEDIATE")
        database.executemany(
            "DELETE FROM session_docs WHERE session_id=?",
            ((session_id,) for session_id in session_ids),
        )
        database.commit()
    except Exception:
        database.rollback()
        raise
    finally:
        database.close()


def _write_json(path, value):
    with path.open("w") as stream:
        json.dump(value, stream, ensure_ascii=False)
        stream.flush()
        os.fsync(stream.fileno())


def _rendered_tool(block, session, message, tool_decider):
    tool = block.tool
    if not tool_decider:
        rendered = {
            "name": tool.name, "input": tool.input,
            "output": tool_result_text(tool.result),
        }
    else:
        decision = tool_decider(tool, session, message)
        if decision.rendered is None:
            history = json.dumps(tool.input, ensure_ascii=False, default=str)
            output = tool_result_text(tool.result)
            return {"narration": (
                f"[Tool {tool.name}] {history}"
                + (f"\n{output}" if output else "")
            )}
        rendered = decision.rendered
    status = tool.result.status if tool.result else None
    return (
        str(rendered.get("name") or tool.name),
        rendered.get("input", tool.input),
        str(rendered.get("output") or tool_result_text(tool.result)),
        {
            "success": "Completed", "error": "Failed",
            "pending": "Pending",
        }.get(status, "Completed"),
        tool.result is not None,
    )


def _native_rows(session, sid, tool_decider=None):
    chat, updates, prompt_index = [], [], 0
    model = session.model or "grok-code-fast-1"
    for message in session.messages:
        if message.role == "user":
            content = []
            for block in message.blocks:
                if block.kind == "text":
                    content.append({"type": "text", "text": block.text})
                elif block.kind == "tool" and block.tool:
                    rendered = _rendered_tool(
                        block, session, message, tool_decider,
                    )
                    narration = (
                        rendered["narration"]
                        if isinstance(rendered, dict)
                        else "[Tool {}] {}\n{}".format(
                            rendered[0],
                            json.dumps(
                                rendered[1], ensure_ascii=False, default=str,
                            ),
                            rendered[2],
                        ).rstrip()
                    )
                    content.append({"type": "text", "text": narration})
            chat.append({"type": "user", "id": uuid.uuid4().hex,
                         "content": content})
            updates.append({"method": "session/update", "params": {
                "sessionId": sid, "update": {
                    "sessionUpdate": "user_message_chunk",
                    "content": {"type": "text", "text":
                                "".join(item["text"] for item in content)},
                    "_meta": {"promptIndex": prompt_index, "modelId": model},
                }, "_meta": {"eventId": uuid.uuid4().hex},
            }})
            prompt_index += 1
            continue
        prompt_id = uuid.uuid4().hex
        assistant = {"type": "assistant", "id": uuid.uuid4().hex,
                     "content": "", "model_id": model}
        tool_results = []
        for block in message.blocks:
            if block.kind == "text" and block.text:
                assistant["content"] += block.text
                updates.append({"method": "session/update", "params": {
                    "sessionId": sid, "update": {
                        "sessionUpdate": "agent_message_chunk",
                        "content": {"type": "text", "text": block.text},
                    }, "_meta": {"eventId": uuid.uuid4().hex,
                                 "promptId": prompt_id},
                }})
            elif block.kind == "tool" and block.tool:
                rendered = _rendered_tool(
                    block, session, message, tool_decider,
                )
                if isinstance(rendered, dict):
                    narration = rendered["narration"]
                    assistant["content"] += narration
                    updates.append({
                        "method": "session/update", "params": {
                            "sessionId": sid, "update": {
                                "sessionUpdate": "agent_message_chunk",
                                "content": {
                                    "type": "text", "text": narration,
                                },
                            }, "_meta": {
                                "eventId": uuid.uuid4().hex,
                                "promptId": prompt_id,
                            },
                        },
                    })
                    continue
                name, tool_input, output, status, has_result = rendered
                call_id = block.tool.source_call_id or uuid.uuid4().hex
                assistant.setdefault("tool_calls", []).append({
                    "id": call_id, "name": name,
                    "arguments": json.dumps(
                        tool_input, ensure_ascii=False,
                        separators=(",", ":"), default=str,
                    ),
                })
                updates.append({"method": "session/update", "params": {
                    "sessionId": sid, "update": {
                        "sessionUpdate": "tool_call", "toolCallId": call_id,
                        "title": name, "kind": name, "status": "pending",
                        "rawInput": tool_input,
                    }, "_meta": {"eventId": uuid.uuid4().hex,
                                 "promptId": prompt_id,
                                 "updateParams": {"toolCallId": call_id,
                                     "kind": name, "status": "Pending"}},
                }})
                if has_result:
                    updates.append({"method": "session/update", "params": {
                        "sessionId": sid, "update": {
                            "sessionUpdate": "tool_call_update",
                            "toolCallId": call_id,
                            "content": [{"type": "text", "text": output}],
                            "rawOutput": output, "kind": name,
                            "status": status.lower(),
                        }, "_meta": {"eventId": uuid.uuid4().hex,
                                     "promptId": prompt_id,
                                     "updateParams": {
                                         "toolCallId": call_id,
                                         "kind": name, "status": status,
                                     }},
                    }})
                    tool_results.append({
                        "type": "tool_result", "id": uuid.uuid4().hex,
                        "tool_call_id": call_id, "content": output,
                    })
        chat.append(assistant)
        chat.extend(tool_results)
    return chat, updates


def _subagent_rows(parent_id, children):
    rows = []
    for child_id, child in children:
        agent_id = child.agent_id or child_id
        rows.extend((
            {"method": "_x.ai/session/update", "params": {
                "sessionId": parent_id, "update": {
                    "sessionUpdate": "subagent_spawned",
                    "subagent_id": agent_id,
                    "parent_session_id": parent_id,
                    "child_session_id": child_id,
                    "subagent_type": child.agent_type or "general-purpose",
                    "description": child.agent_role or child.title or "Migrated child",
                }, "_meta": {"eventId": uuid.uuid4().hex},
            }},
            {"method": "_x.ai/session/update", "params": {
                "sessionId": parent_id, "update": {
                    "sessionUpdate": "subagent_finished",
                    "subagent_id": agent_id, "child_session_id": child_id,
                    "status": "completed", "tool_calls": 0,
                    "turns": 0, "duration_ms": 0,
                }, "_meta": {"eventId": uuid.uuid4().hex},
            }},
        ))
    return rows


def write(session, cwd: str, root: Path | None = None, tool_decider=None):
    sessions_root = Path(root) if root else grok_home() / "sessions"
    target_cwd = Path(cwd).resolve()
    if not target_cwd.is_dir():
        raise FileNotFoundError(f"Grok 迁移目标目录不存在: {target_cwd}")
    nodes = list(session.walk())
    identifiers = {id(node): str(uuid.uuid4()) for node in nodes}
    parents = {
        id(child): identifiers[id(parent)]
        for parent in nodes for child in parent.children
    }
    root_id = identifiers[id(session)]
    temporary_paths, destinations = [], []
    published = []
    now = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    try:
        for node in nodes:
            sid = identifiers[id(node)]
            node_cwd = str(target_cwd)
            project = sessions_root / quote(node_cwd, safe="")
            destination = project / sid
            temporary = project / f".{sid}.{os.getpid()}.tmp"
            temporary.mkdir(parents=True)
            temporary_paths.append(temporary)
            destinations.append(destination)
            children = [
                (identifiers[id(child)], child) for child in node.children
            ]
            chat, updates = _native_rows(node, sid, tool_decider)
            updates.extend(_subagent_rows(sid, children))
            parent_id = parents.get(id(node))
            summary = {
                "info": {"id": sid, "cwd": node_cwd},
                "session_summary": node.title or "Migrated session",
                "generated_title": node.title or "Migrated session",
                "created_at": now, "updated_at": now,
                "num_messages": len(chat), "num_chat_messages": len(chat),
                "current_model_id": node.model or "grok-code-fast-1",
                "chat_format_version": 1, "root_session_id": root_id,
            }
            if parent_id:
                summary["parent_session_id"] = parent_id
            _write_json(temporary / "summary.json", summary)
            write_jsonl(temporary / "updates.jsonl", updates)
            write_jsonl(temporary / "chat_history.jsonl", chat)
            read(str(temporary))
            from .probe import probe_bundle

            report = probe_bundle(temporary)
            if report["status"] != "passed":
                raise RuntimeError(
                    "Grok CLI 无法验收生成会话: "
                    + str(report.get("diagnostic") or report)
                )
        for temporary, destination in reversed(list(zip(
                temporary_paths, destinations))):
            os.replace(temporary, destination)
            descriptor = os.open(destination.parent, os.O_RDONLY)
            try:
                os.fsync(descriptor)
            finally:
                os.close(descriptor)
            published.append(destination)
        index_bundles(published, sessions_root)
        return root_id, destinations[0]
    except Exception:
        for path in temporary_paths:
            shutil.rmtree(path, ignore_errors=True)
        for path in published:
            shutil.rmtree(path, ignore_errors=True)
        raise
