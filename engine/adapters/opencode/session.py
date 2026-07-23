"""OpenCode reader/writer:走官方 `opencode export` / `opencode import`,不直接碰 SQLite。

处理 OpenCode 会话存储。export 形状:
    {"info": <session 行>, "messages": [{"info": <message.data>, "parts": [<part.data>...]}]}
"""
import json
import os
import re
import secrets
import sqlite3
import subprocess
import tempfile
import time
from pathlib import Path

from ...domain.model import (
    AgentEdge,
    Block,
    ContextCompaction,
    Message,
    RawRecord,
    Session,
    ToolCall,
    ToolResult,
    ToolResultBlock,
    normalize_tool_result_status,
)
from ...domain.reasoning import visible_text
from ...domain.tool_ops import CanonicalOp, has_valid_tool_input
from ...domain.usage import iso_ms
from ...infrastructure import executables
from ..base.media import image_from_data_url
from ..base.narration import narrate
from .formats import FORMATS

TOOL_OPS = {
    "bash": CanonicalOp.SHELL_EXEC,
    "read": CanonicalOp.FS_READ,
    "write": CanonicalOp.FS_WRITE,
    "edit": CanonicalOp.FS_EDIT,
    "apply_patch": CanonicalOp.FS_PATCH,
    "grep": CanonicalOp.FS_SEARCH,
    "glob": CanonicalOp.FS_GLOB,
    "webfetch": CanonicalOp.WEB_FETCH,
    "websearch": CanonicalOp.WEB_SEARCH,
}
OPENCODE_DB = Path.home() / ".local" / "share" / "opencode" / "opencode.db"


def _patch_operations(patch: str) -> list[dict]:
    return [
        {"operation": operation.lower(), "path": path.strip()}
        for operation, path in re.findall(
            r"^\*\*\* (Add|Update|Delete) File: ([^\r\n]+)$",
            patch, re.MULTILINE,
        )
    ]


def _canonical_tool_input(name: str, source_input):
    inp = dict(source_input) if isinstance(source_input, dict) else source_input
    if name == "task":
        value = {
            "description": str(inp.get("description") or "migrated subagent"),
            "prompt": str(inp.get("prompt") or ""),
            "subagent_type": str(inp.get("subagent_type") or
                                 inp.get("agent") or "general"),
        }
        for field in ("task_name", "model", "fork_mode", "reasoning_effort"):
            if inp.get(field) is not None:
                value[field] = str(inp[field])
        return CanonicalOp.AGENT_SPAWN, value
    op = TOOL_OPS.get(name)
    if op is None:
        return CanonicalOp.TOOL_INVOKE, {
            "namespace": "opencode", "name": name, "input": inp,
        }
    if not isinstance(inp, dict):
        return op, inp
    if name == "bash":
        value = {"command": inp.get("command", "")}
        aliases = {
            "workdir": "workdir", "timeout": "timeout_ms",
            "timeout_ms": "timeout_ms",
            "run_in_background": "background", "background": "background",
        }
        for source, target in aliases.items():
            if source in inp:
                value[target] = inp[source]
        return op, value
    if name == "read":
        value = {"file_path": inp.get("filePath", inp.get("file_path", ""))}
        value.update({key: inp[key] for key in ("offset", "limit") if key in inp})
        return op, value
    if name == "write":
        return op, {
            "file_path": inp.get("filePath", inp.get("file_path", "")),
            "content": inp.get("content", ""),
        }
    if name == "edit":
        value = {
            "file_path": inp.get("filePath", inp.get("file_path", "")),
            "old": inp.get("oldString", inp.get("old", "")),
            "new": inp.get("newString", inp.get("new", "")),
        }
        if "replaceAll" in inp or "replace_all" in inp:
            value["replace_all"] = inp.get("replaceAll", inp.get("replace_all"))
        return op, value
    if name == "apply_patch":
        patch = inp.get("patchText", inp.get("raw_patch", ""))
        return op, {"operations": _patch_operations(str(patch)),
                    "raw_patch": str(patch)}
    if name == "grep":
        value = {"query": inp.get("pattern", inp.get("query", ""))}
        if "path" in inp:
            value["path"] = inp["path"]
        if "include" in inp or "glob" in inp:
            value["glob"] = inp.get("include", inp.get("glob"))
        if "limit" in inp:
            value["max_results"] = inp["limit"]
        return op, value
    if name == "glob":
        value = {"pattern": inp.get("pattern", "")}
        if "path" in inp:
            value["path"] = inp["path"]
        return op, value
    if name == "webfetch":
        value = {"url": inp.get("url", "")}
        aliases = {"format": "format", "timeout": "timeout_ms"}
        for source, target in aliases.items():
            if source in inp:
                value[target] = inp[source]
        return op, value
    if name == "websearch":
        value = {"query": inp.get("query", "")}
        if "numResults" in inp:
            value["num_results"] = inp["numResults"]
        return op, value
    return op, inp


def _oc(args, **kw):
    r = subprocess.run(executables.argv("opencode", *args),
                       capture_output=True, text=True,
                       timeout=120, **executables.RUN_FLAGS, **kw)
    if r.returncode != 0:
        raise RuntimeError(f"opencode {' '.join(args)} 失败: {r.stderr[-400:]}")
    return r.stdout


def _oc_export(session_id: str) -> dict:
    """export 大会话经 pipe 会被截断到 64KB(Bun/opencode 管道缓冲),改写临时文件。"""
    fd, path = tempfile.mkstemp(prefix="rh-oc-export-", suffix=".json")
    os.close(fd)
    try:
        with open(path, "w") as f:
            r = subprocess.run(
                executables.argv("opencode", "export", session_id),
                stdout=f, stderr=subprocess.PIPE, text=True, timeout=120,
                **executables.RUN_FLAGS)
        if r.returncode != 0:
            raise RuntimeError(
                f"opencode export 失败: {(r.stderr or '')[-400:]}")
        return json.loads(Path(path).read_text())
    finally:
        try:
            os.unlink(path)
        except OSError:
            pass


def _new_id(prefix: str) -> str:
    return f"{prefix}_{secrets.token_hex(6)}{secrets.token_urlsafe(12)[:14]}"


def _new_ordered_id(prefix: str, ordinal: int) -> str:
    """生成同一父记录内可按字典序恢复原顺序的 ID。"""
    return f"{prefix}_{ordinal:08x}{secrets.token_hex(10)}"


# ---------- reader ----------

def _opencode_result(state: dict) -> ToolResult:
    metadata = _clone(state.get("metadata") or {})
    canonical = metadata.pop("canonicalToolResult", None)
    canonical = canonical if isinstance(canonical, dict) else {}
    if isinstance(state.get("status"), str):
        metadata["source_status"] = state["status"]
    native_state = {
        key: _clone(value) for key, value in state.items()
        if key not in {
            "input", "output", "error", "metadata", "attachments", "status",
            "time",
        }
    }
    if native_state:
        metadata["opencode_state"] = native_state
    status = normalize_tool_result_status(
        canonical.get("status") or state.get("status"))
    if metadata.get("interrupted") is True:
        status = "interrupted"

    output = state.get("output")
    blocks = []
    canonical_blocks = canonical.get("blocks")
    if isinstance(canonical_blocks, list):
        for item in canonical_blocks:
            if not isinstance(item, dict):
                blocks.append(ToolResultBlock("json", data=item))
                continue
            kind = item.get("kind")
            if kind not in {"text", "json", "image", "file", "tool_reference"}:
                kind = "json"
                item = {"data": item}
            blocks.append(ToolResultBlock(
                kind, text=item.get("text", ""), data=item.get("data"),
                mime_type=item.get("mime_type"),
                filename=item.get("filename"), uri=item.get("uri"),
                metadata=item.get("metadata") or {},
            ))
    elif isinstance(output, str):
        if output:
            blocks.append(ToolResultBlock("text", text=output))
    elif output is not None:
        blocks.append(ToolResultBlock("json", data=_clone(output)))

    error = state.get("error")
    if isinstance(error, str) and error:
        if (not isinstance(canonical_blocks, list) and
                not any(block.kind == "text" and block.text == error
                        for block in blocks)):
            blocks.append(ToolResultBlock(
                "text", text=error, metadata={"stream": "error"},
            ))
        if status == "unknown":
            status = "error"

    attachments = _clone(state.get("attachments") or [])
    if not isinstance(attachments, list):
        attachments = [attachments]
    for attachment in attachments if not isinstance(canonical_blocks, list) else []:
        if not isinstance(attachment, dict):
            blocks.append(ToolResultBlock("json", data=attachment,
                                          metadata={"attachment": True}))
            continue
        if attachment.get("type") == "file":
            blocks.append(ToolResultBlock(
                "file",
                mime_type=attachment.get("mime"),
                filename=attachment.get("filename"),
                uri=attachment.get("url"),
                metadata={
                    key: value for key, value in attachment.items()
                    if key not in {"type", "mime", "filename", "url"}
                },
            ))
        else:
            blocks.append(ToolResultBlock(
                "json", data=attachment,
                metadata={"attachment": True,
                          "native_type": attachment.get("type")},
            ))

    exit_code = metadata.get("exit")
    if isinstance(exit_code, bool) or not isinstance(exit_code, int):
        exit_code = None
    truncated = metadata.get("truncated")
    if not isinstance(truncated, bool):
        truncated = None
    stdout = metadata.get("stdout")
    stderr_value = error if isinstance(error, str) else metadata.get("stderr")
    return ToolResult(
        status=status,
        blocks=blocks,
        stdout=stdout if isinstance(stdout, str) else None,
        stderr=stderr_value if isinstance(stderr_value, str) else None,
        exit_code=exit_code,
        truncated=truncated,
        attachments=attachments,
        metadata=metadata,
    )

def _db_conn():
    """只读打开当前库;库缺失或 schema 不兼容时返回 None,由 CLI export 兜底。"""
    if not OPENCODE_DB.exists():
        return None
    try:
        conn = sqlite3.connect(f"file:{OPENCODE_DB.resolve()}?mode=ro", uri=True)
        conn.row_factory = sqlite3.Row
        conn.execute("SELECT id FROM session LIMIT 1")
        conn.execute("SELECT id, session_id, data FROM message LIMIT 1")
        conn.execute("SELECT id, message_id, session_id, data FROM part LIMIT 1")
        return conn
    except (OSError, sqlite3.Error):
        return None


def _db_info(row) -> dict:
    """由 session 行构造官方 export 的 info 形状(已与 `opencode export` 对拍)。"""
    cost = row["cost"]
    if isinstance(cost, float) and cost.is_integer():
        cost = int(cost)
    info = {"id": row["id"], "slug": row["slug"], "projectID": row["project_id"],
            "directory": row["directory"], "path": row["path"] or "",
            "title": row["title"], "version": row["version"],
            "summary": {"additions": row["summary_additions"] or 0,
                        "deletions": row["summary_deletions"] or 0,
                        "files": row["summary_files"] or 0},
            "cost": cost,
            "tokens": {"input": row["tokens_input"], "output": row["tokens_output"],
                       "reasoning": row["tokens_reasoning"],
                       "cache": {"read": row["tokens_cache_read"],
                                 "write": row["tokens_cache_write"]}},
            "time": {"created": row["time_created"], "updated": row["time_updated"]}}
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


def _db_export(conn, session_id: str) -> dict | None:
    """直读 SQLite 构造 export 形状,免去每会话一次 `opencode export` 子进程。"""
    try:
        row = conn.execute("SELECT * FROM session WHERE id = ?", (session_id,)).fetchone()
        if row is None:
            return None
        parts_by_mid: dict[str, list] = {}
        for part in conn.execute(
                "SELECT id, message_id, session_id, data FROM part "
                "WHERE session_id = ? ORDER BY time_created, id", (session_id,)):
            data = json.loads(part["data"])
            data.update(id=part["id"], sessionID=part["session_id"],
                        messageID=part["message_id"])
            parts_by_mid.setdefault(part["message_id"], []).append(data)
        messages = []
        for message in conn.execute(
                "SELECT id, session_id, data FROM message "
                "WHERE session_id = ? ORDER BY time_created, id", (session_id,)):
            data = json.loads(message["data"])
            data.update(id=message["id"], sessionID=message["session_id"])
            messages.append({"info": data, "parts": parts_by_mid.get(message["id"], [])})
        return {"info": _db_info(row), "messages": messages}
    except (sqlite3.Error, json.JSONDecodeError, KeyError, IndexError):
        return None


def _db_child_ids(session_id: str) -> list[str]:
    """只读查询当前库；库不存在/版本不兼容时由 export 中的 task 关系兜底。"""
    try:
        uri = f"file:{OPENCODE_DB.resolve()}?mode=ro"
        with sqlite3.connect(uri, uri=True) as db:
            rows = db.execute(
                "SELECT id FROM session WHERE parent_id = ? "
                "ORDER BY time_created, id", (session_id,)).fetchall()
        return [row[0] for row in rows]
    except (OSError, sqlite3.Error):
        return []


def _parse_session(data: dict) -> tuple[Session, list[AgentEdge]]:
    info = data["info"]
    sess = Session(source_tool="opencode", source_id=info["id"],
                   cwd=info.get("directory", ""), title=info.get("title", ""),
                   parent_id=info.get("parentID"), agent_id=info.get("agent"),
                   meta={"opencode_export": _clone(data)})
    session_raw = RawRecord("opencode", "session", _clone(info))
    sess.raw_records.append(session_raw)
    edges = []
    pending_compactions = []
    last_visible_message_id = None
    raw_message_indexes = {
        (message.get("info") or {}).get("id"): index
        for index, message in enumerate(data.get("messages", []), start=1)
    }
    for ordinal, m in enumerate(data.get("messages", [])):
        role = m["info"].get("role", "user")
        blocks = []
        mid = m["info"].get("id")
        compaction_part = next((
            part for part in m.get("parts", [])
            if part.get("type") == "compaction"), None)
        if compaction_part is not None:
            tail_locator = compaction_part.get("tail_start_id")
            tail_index = raw_message_indexes.get(tail_locator)
            compaction = ContextCompaction(
                id=mid or f"compaction:{ordinal}",
                source="opencode",
                after_message_id=last_visible_message_id,
                event_locator=mid,
                created_at=(m["info"].get("time") or {}).get("created"),
                trigger=("automatic" if compaction_part.get("auto") is True
                         else "manual" if compaction_part.get("auto") is False
                         else "unknown"),
                state="incomplete",
                tail_status="located" if tail_index is not None else "unknown",
                tail_start_locator=tail_locator,
                tail_start_message_index=tail_index,
                source_meta={
                    key: _clone(value) for key, value in compaction_part.items()
                    if key not in {"type", "tail_start_id"}
                },
            )
            sess.context_compactions.append(compaction)
            pending_compactions.append(compaction)
        is_summary = (
            m["info"].get("mode") == "compaction"
            or m["info"].get("summary") is True
        )
        if is_summary:
            summary = "\n".join(
                str(part.get("text") or "") for part in m.get("parts", [])
                if part.get("type") == "text" and part.get("text"))
            compaction = next((
                item for item in reversed(pending_compactions)
                if item.summary_message_id is None), None)
            if compaction is not None:
                compaction.summary_message_id = mid
                compaction.summary_text = summary
                compaction.summary_status = "available" if summary else "missing"
                compaction.state = "completed" if summary else "incomplete"
        message_raw = RawRecord(
            "opencode", "message", _clone(m["info"]), ordinal=ordinal,
            timestamp=(m["info"].get("time") or {}).get("created"))
        sess.raw_records.append(message_raw)
        message_records = [message_raw]
        for part_ordinal, p in enumerate(m.get("parts", [])):
            part_raw = RawRecord(
                "opencode", "part", _clone(p), ordinal=part_ordinal,
                timestamp=(p.get("time") or
                           (p.get("state") or {}).get("time") or {}).get("start"),
                location=mid or "")
            sess.raw_records.append(part_raw)
            message_records.append(part_raw)
            pt = p.get("type")
            if pt == "text":
                blocks.append(Block("text", p.get("text", "")))
            elif pt == "file" and str(p.get("mime", "")).startswith("image/"):
                image = image_from_data_url(
                    f"{mid}:image:{part_ordinal}", p.get("url", ""), p.get("filename"))
                if image is None:
                    sess.lose("migration.unknown_block_dropped", kind="file")
                else:
                    blocks.append(Block("image", image=image))
            elif pt == "reasoning":
                text = visible_text(p.get("text"))
                if text is not None:
                    blocks.append(Block("text", text))
                    sess.lose("migration.reasoning_metadata_dropped", metadata_kind="metadata")
                else:
                    sess.lose("migration.reasoning_dropped", metadata_kind="metadata")
            elif pt == "tool":
                st = p.get("state", {})
                source_input = st.get("input")
                op, inp = _canonical_tool_input(
                    p.get("tool", "?"), source_input or {})
                blocks.append(Block("tool", tool=ToolCall(
                    name=p.get("tool", "?"),
                    op=op,
                    input=inp,
                    output="",
                    meta=_clone(st.get("metadata") or {}),
                    source_call_id=p.get("callID"),
                    started_at=(st.get("time") or {}).get("start"),
                    ended_at=(st.get("time") or {}).get("end"),
                    result=_opencode_result(st))))
                metadata = st.get("metadata") or {}
                child_id = metadata.get("sessionId")
                if p.get("tool") == "task" and child_id:
                    edges.append(AgentEdge(
                        parent_session_id=info["id"],
                        child_session_id=child_id,
                        source_call_id=p.get("callID"), spawn_message_id=mid,
                        agent_id=inp.get("subagent_type") or inp.get("agent"),
                        agent_type=inp.get("subagent_type"),
                        prompt=inp.get("prompt", ""), status=st.get("status"),
                        association="task-metadata", confidence=1.0,
                        meta=_clone(metadata)))
            elif pt in ("step-start", "step-finish"):
                continue
            else:
                sess.lose("migration.unknown_block_dropped", kind=pt)
        if blocks:
            minfo = m["info"]
            parent_id = minfo.get("parentID")
            sess.messages.append(Message(
                role=role, blocks=blocks, raw=message_records, source_id=mid,
                parent_ids=[parent_id] if parent_id else [],
                agent_id=minfo.get("agent"),
                created_at=(minfo.get("time") or {}).get("created")))
            if not is_summary:
                last_visible_message_id = mid
    compacting = (info.get("time") or {}).get("compacting")
    if compacting and not any(
            compaction.state == "in_progress"
            for compaction in sess.context_compactions):
        sess.context_compactions.append(ContextCompaction(
            id=f"{info['id']}:compacting",
            source="opencode",
            after_message_id=last_visible_message_id,
            created_at=compacting,
            state="in_progress",
        ))
    return sess, edges


def _read(session_id: str, *, allow_cli: bool) -> Session:
    seen: dict[str, Session] = {}
    conn = _db_conn()
    if conn is None and not allow_cli:
        raise RuntimeError("OpenCode 数据库不可只读访问，拒绝执行 Agent 预览")

    def export(sid: str) -> dict:
        if conn is not None:
            data = _db_export(conn, sid)
            if data is not None:
                return data
        if not allow_cli:
            raise RuntimeError("OpenCode 会话无法从数据库只读加载，拒绝执行 Agent 预览")
        return _oc_export(sid)

    def child_ids_of(sid: str) -> list[str]:
        if conn is not None:
            try:
                return [row[0] for row in conn.execute(
                    "SELECT id FROM session WHERE parent_id = ? "
                    "ORDER BY time_created, id", (sid,))]
            except sqlite3.Error:
                pass
        return _db_child_ids(sid) if allow_cli else []

    def visit(sid: str, root_id: str) -> Session:
        if sid in seen:
            return seen[sid]
        sess, task_edges = _parse_session(export(sid))
        seen[sid] = sess
        sess.root_id = root_id

        task_by_child = {}
        for edge in task_edges:
            task_by_child.setdefault(edge.child_session_id, []).append(edge)
        child_ids = child_ids_of(sid)
        db_child_ids = set(child_ids)
        for child_id in task_by_child:
            if child_id not in child_ids:
                child_ids.append(child_id)

        for child_id in child_ids:
            if child_id in seen:
                continue
            child = visit(child_id, root_id)
            if child_id not in db_child_ids and child.parent_id != sid:
                sess.lose("session.child_foreign_ignored", child_id=child_id)
                continue
            if child.parent_id and child.parent_id != sid:
                sess.lose("session.child_parent_conflict", child_id=child_id)
                continue
            child.parent_id = sid
            sess.children.append(child)
            edges = task_by_child.get(child_id) or [AgentEdge(
                parent_session_id=sid, child_session_id=child_id,
                association="sqlite-parent", confidence=0.9,
                meta={"association": "sqlite-parent"},
            )]
            for edge in edges:
                edge.agent_id = edge.agent_id or child.agent_id
                edge.agent_path = edge.agent_path or child.agent_path
                edge.agent_type = edge.agent_type or child.agent_type
                sess.agent_edges.append(edge)
        return sess

    try:
        return visit(session_id, session_id)
    finally:
        if conn is not None:
            conn.close()


def read(session_id: str) -> Session:
    return _read(session_id, allow_cli=True)


def read_preview(session_id: str) -> Session:
    """Agent 预览专用：只读 SQLite，不启动 CLI、不创建临时文件。"""
    return _read(session_id, allow_cli=False)


# ---------- writer ----------

def _template():
    """Load the latest declarative OpenCode format profile."""
    return FORMATS.templates()


def _clone(o):
    return json.loads(json.dumps(o))


def _write_shell_exec(add_tool_part, tool) -> bool:
    inputs = tool.input if isinstance(tool.input, dict) else {}
    command = inputs.get("command")
    if not command:
        return False
    native_input = {"command": command}
    if "workdir" in inputs:
        native_input["workdir"] = inputs["workdir"]
    if "timeout_ms" in inputs:
        native_input["timeout"] = inputs["timeout_ms"]
    if "background" in inputs:
        native_input["run_in_background"] = inputs["background"]
    return add_tool_part(
        "bash", native_input, tool.output, command,
        {"output": tool.output, "exit": tool.meta.get("exit", 0) or 0,
         "truncated": False}, tool)


def _write_fs_read(add_tool_part, tool) -> bool:
    inputs = tool.input if isinstance(tool.input, dict) else {}
    path = inputs.get("file_path")
    if not path:
        return False
    native_input = {"filePath": path}
    native_input.update({key: inputs[key] for key in ("offset", "limit")
                         if key in inputs})
    return add_tool_part("read", native_input, tool.output, path,
                         {"truncated": False}, tool)


def _write_fs_write(add_tool_part, tool) -> bool:
    inputs = tool.input if isinstance(tool.input, dict) else {}
    path = inputs.get("file_path")
    if not path:
        return False
    return add_tool_part(
        "write", {"filePath": path, "content": inputs.get("content", "")},
        tool.output or "Wrote file successfully.", path,
        {"filepath": path, "exists": False, "truncated": False,
         "diagnostics": {}}, tool)


def _write_fs_edit(add_tool_part, tool) -> bool:
    inputs = tool.input if isinstance(tool.input, dict) else {}
    path = inputs.get("file_path")
    if not path:
        return False
    return add_tool_part(
        "edit", {"filePath": path, "oldString": inputs.get("old", ""),
                 "newString": inputs.get("new", "")},
        tool.output or "Edited file.", path, {"truncated": False}, tool)


def _write_fs_patch(add_tool_part, tool) -> bool:
    inputs = tool.input if isinstance(tool.input, dict) else {}
    patch = inputs.get("raw_patch")
    if not patch:
        return False
    return add_tool_part(
        "apply_patch", {"patchText": patch}, tool.output or "Applied patch.",
        "apply patch", {"truncated": False}, tool)


def _write_fs_search(add_tool_part, tool) -> bool:
    inputs = tool.input if isinstance(tool.input, dict) else {}
    query = inputs.get("query")
    if not query:
        return False
    native_input = {"pattern": query}
    if "path" in inputs:
        native_input["path"] = inputs["path"]
    if "glob" in inputs:
        native_input["include"] = inputs["glob"]
    return add_tool_part("grep", native_input, tool.output, str(query),
                         {"truncated": False}, tool)


def _write_fs_glob(add_tool_part, tool) -> bool:
    inputs = tool.input if isinstance(tool.input, dict) else {}
    pattern = inputs.get("pattern")
    if not pattern:
        return False
    native_input = {"pattern": pattern}
    if "path" in inputs:
        native_input["path"] = inputs["path"]
    return add_tool_part("glob", native_input, tool.output, str(pattern),
                         {"truncated": False}, tool)


def _write_web_fetch(add_tool_part, tool) -> bool:
    inputs = tool.input if isinstance(tool.input, dict) else {}
    url = inputs.get("url")
    if not url:
        return False
    native_input = {"url": url}
    if "format" in inputs:
        native_input["format"] = inputs["format"]
    if "timeout_ms" in inputs:
        native_input["timeout"] = inputs["timeout_ms"]
    return add_tool_part("webfetch", native_input, tool.output, str(url),
                         {"truncated": False}, tool)


def _write_web_search(add_tool_part, tool) -> bool:
    inputs = tool.input if isinstance(tool.input, dict) else {}
    query = inputs.get("query")
    if not query:
        return False
    native_input = {"query": query}
    if "num_results" in inputs:
        native_input["numResults"] = inputs["num_results"]
    return add_tool_part("websearch", native_input, tool.output, str(query),
                         {"truncated": False}, tool)


def _write_tool_invoke(add_tool_part, tool) -> bool:
    inputs = tool.input if isinstance(tool.input, dict) else {}
    name = inputs.get("name")
    native_input = inputs.get("input")
    if not name or not isinstance(native_input, (dict, str)):
        return False
    return add_tool_part(str(name), native_input, tool.output, str(name),
                         {"historical": True, "truncated": False}, tool)


OP_WRITERS = {
    CanonicalOp.SHELL_EXEC: _write_shell_exec,
    CanonicalOp.FS_READ: _write_fs_read,
    CanonicalOp.FS_WRITE: _write_fs_write,
    CanonicalOp.FS_EDIT: _write_fs_edit,
    CanonicalOp.FS_PATCH: _write_fs_patch,
    CanonicalOp.FS_SEARCH: _write_fs_search,
    CanonicalOp.FS_GLOB: _write_fs_glob,
    CanonicalOp.WEB_FETCH: _write_web_fetch,
    CanonicalOp.WEB_SEARCH: _write_web_search,
    CanonicalOp.TOOL_INVOKE: _write_tool_invoke,
}

OP_FIDELITY = {op: "native" for op in OP_WRITERS} | {
    # Task links are emitted after child sessions have been assigned IDs.
    CanonicalOp.AGENT_SPAWN: "native",
}


def _message_times(messages: list[Message], now: int) -> list[int]:
    """保留源会话顺序，并为 OpenCode 生成严格递增的毫秒时间戳。"""
    parsed = [iso_ms(message.created_at) for message in messages]
    known = [value for value in parsed if value is not None]
    fallback = (min(known) if known else now) - len(messages)
    ordered = []
    previous = None
    for value in parsed:
        candidate = value if value is not None else (
            previous + 1 if previous is not None else fallback)
        current = candidate if previous is None else max(candidate, previous + 1)
        ordered.append(current)
        previous = current
    return ordered


def _normalize_payload_message_times(payload: dict) -> None:
    """按 export 数组顺序消除时间戳并列，避免随机 ID 成为排序依据。"""
    messages = payload.get("messages", [])
    source_times = []
    for message in messages:
        info = message.get("info") if isinstance(message.get("info"), dict) else {}
        source_time = info.get("time") if isinstance(info.get("time"), dict) else {}
        source_times.append(source_time)
    parsed = [iso_ms(source_time.get("created")) for source_time in source_times]
    known = [value for value in parsed if value is not None]
    fallback = (min(known) if known else int(time.time() * 1000)) - len(messages)
    created_times = []
    previous_created = None
    for value in parsed:
        candidate = value if value is not None else (
            previous_created + 1 if previous_created is not None else fallback)
        created = candidate if previous_created is None else max(
            candidate, previous_created + 1)
        created_times.append(created)
        previous_created = created
    previous_completed = None
    for message, source_time, original_created, created in zip(
            messages, source_times, parsed, created_times):
        info = message.get("info")
        if not isinstance(info, dict):
            info = {}
            message["info"] = info
        normalized_time = dict(source_time)
        normalized_time["created"] = created
        if "completed" in source_time:
            original_completed = iso_ms(source_time.get("completed"))
            duration = max(0, original_completed - original_created) \
                if original_completed is not None and original_created is not None else 0
            completed = created + duration
            if previous_completed is not None:
                completed = max(completed, previous_completed + 1)
            normalized_time["completed"] = completed
            previous_completed = completed
        info["time"] = normalized_time

    if messages:
        info = payload.get("info")
        if not isinstance(info, dict):
            info = {}
            payload["info"] = info
        session_time = info.get("time") if isinstance(info.get("time"), dict) else {}
        info["time"] = session_time
        source_created = iso_ms(session_time.get("created"))
        source_updated = iso_ms(session_time.get("updated"))
        session_time["created"] = min(
            source_created if source_created is not None else created_times[0],
            created_times[0],
        )
        session_time["updated"] = max(
            source_updated if source_updated is not None else created_times[-1],
            created_times[-1],
        )
    else:
        info = payload.get("info")
        if not isinstance(info, dict):
            info = {}
            payload["info"] = info
        session_time = info.get("time") if isinstance(info.get("time"), dict) else {}
        now = int(time.time() * 1000)
        created = iso_ms(session_time.get("created"))
        updated = iso_ms(session_time.get("updated"))
        created = created if created is not None else now
        updated = max(created, updated if updated is not None else created)
        info["time"] = {**session_time, "created": created, "updated": updated}


def _assistant_result(sess: Session) -> str:
    for message in reversed(sess.messages):
        if message.role == "assistant":
            text = "\n".join(block.text for block in message.blocks
                             if block.kind == "text" and block.text)
            if text:
                return text
    return ""


def _task_part(tpl: dict, sid: str, mid: str, ordinal: int, child: Session,
               child_sid: str, edge: AgentEdge | None, when: int,
               source_call_id: str | None = None) -> dict:
    part = _clone(tpl["part.tool"])
    prompt = edge.prompt if edge else ""
    part.update({
        "id": _new_ordered_id("prt", ordinal), "messageID": mid,
        "sessionID": sid, "type": "tool", "tool": "task",
        "callID": (edge.source_call_id if edge and edge.source_call_id
                   else source_call_id or "call-" + secrets.token_hex(8)),
        "state": {
            "status": "completed",
            "input": {"description": child.title or "migrated subagent",
                      "prompt": prompt,
                      "subagent_type": (edge.agent_type if edge else None)
                      or child.agent_type or "general"},
            "output": _assistant_result(child),
            "title": child.title or "Subagent",
            "metadata": {"parentSessionId": sid, "sessionId": child_sid},
            "time": {"start": when, "end": when},
        },
    })
    return part


def _canonical_payload(sess: Session, sid: str, cwd: str, parent_sid: str | None,
                       tpl: dict, sid_map: dict[str, str] | None = None,
                       tool_decider=None) -> dict:
    now = int(time.time() * 1000)
    message_times = _message_times(sess.messages, now)
    session_created = message_times[0] if message_times else now
    if sess.messages and sess.messages[0].role == "assistant":
        session_created -= 1
    session_updated = message_times[-1] if message_times else session_created
    info = _clone(tpl["info"])
    info.update({"id": sid, "directory": cwd,
                  "title": sess.title or f"migrated from {sess.source_tool}",
                  "time": {"created": session_created,
                           "updated": session_updated}})
    # `opencode import` strictly validates complete Session.Info. The profile
    # keeps only structural fields, so required defaults are completed here.
    info.setdefault("slug", f"ferry-{sid[-8:].lower()}")
    info.setdefault("projectID", "global")
    info.setdefault("path", "")
    info.setdefault("agent", "build")
    info.setdefault("summary", {"additions": 0, "deletions": 0, "files": 0})
    info.setdefault("cost", 0)
    info.setdefault("tokens", {
        "input": 0, "output": 0, "reasoning": 0,
        "cache": {"read": 0, "write": 0},
    })
    if parent_sid:
        info["parentID"] = parent_sid
    else:
        info.pop("parentID", None)
    for k in ("share",):
        info.pop(k, None)

    messages = []
    last_user_mid = None
    sid_map = sid_map or {}
    children = {child.source_id: child for child in sess.children}
    edges = {edge.child_session_id: edge for edge in sess.agent_edges}
    linked_children = set()
    emitted_edges = set()
    provider_id = str(sess.meta.get("model_provider") or "openai")
    model_id = str(sess.meta.get("model") or "gpt-5.6-sol")
    for m, message_time in zip(sess.messages, message_times):
        mid = _new_id("msg")
        minfo = _clone(tpl.get(f"msg.{m.role}", tpl["msg.user"]))
        minfo.update({"id": mid, "sessionID": sid})
        if m.role == "assistant":
            if last_user_mid is None:
                last_user_mid = _new_id("msg")
                parent_info = _clone(tpl["msg.user"])
                parent_info.update({
                    "id": last_user_mid, "sessionID": sid, "role": "user",
                    "time": {"created": message_time - 1}, "agent": "build",
                    "model": {"providerID": provider_id,
                              "modelID": model_id},
                    "summary": {"diffs": []},
                })
                parent_part = _clone(tpl["part.text"])
                parent_part.update({
                    "id": _new_ordered_id("prt", 0), "messageID": last_user_mid,
                    "sessionID": sid, "type": "text",
                    "text": "[Migrated subagent task]",
                })
                messages.append({"info": parent_info, "parts": [parent_part]})
            # completed + finish 缺失会让 runtime 认为该轮未结束而死循环
            minfo["time"] = {"created": message_time,
                             "completed": message_time}
            minfo["finish"] = "stop"
            minfo.update({
                "mode": "build", "agent": "build",
                "path": {"cwd": cwd, "root": cwd}, "cost": 0,
                "tokens": {"total": 0, "input": 0, "output": 0,
                           "reasoning": 0,
                           "cache": {"write": 0, "read": 0}},
                "modelID": model_id, "providerID": provider_id,
            })
            if last_user_mid:
                minfo["parentID"] = last_user_mid
            else:
                minfo.pop("parentID", None)
        else:
            minfo["time"] = {"created": message_time}
            minfo.update({
                "agent": "build",
                "model": {"providerID": provider_id, "modelID": model_id},
                "summary": {"diffs": []},
            })
            last_user_mid = mid
        parts = []

        def add_part(ptype, fill):
            key = f"part.{ptype}"
            if key not in tpl:
                return False
            p = _clone(tpl[key])
            p.update({"id": _new_ordered_id("prt", len(parts)), "messageID": mid,
                      "sessionID": sid})
            p.update(fill)
            parts.append(p)
            return True

        def add_tool_part(tool, native_input, output, title, metadata,
                          canonical_tool):
            st = _clone(tpl["part.tool"]["state"])
            result = canonical_tool.result
            state_status = {
                "success": "completed",
                "error": "error",
                "running": "running",
                "pending": "pending",
            }.get(result.status if result else "", "completed")
            native_metadata = dict(metadata)
            if result is not None:
                native_metadata.update(result.metadata)
                native_metadata["canonicalToolResult"] = {
                    "status": result.status,
                    "blocks": [{
                        "kind": block.kind,
                        "text": block.text,
                        "data": block.data,
                        "mime_type": block.mime_type,
                        "filename": block.filename,
                        "uri": block.uri,
                        "metadata": block.metadata,
                    } for block in result.blocks],
                }
                if result.exit_code is not None:
                    native_metadata["exit"] = result.exit_code
                if result.truncated is not None:
                    native_metadata["truncated"] = result.truncated
                if result.stdout is not None:
                    native_metadata["stdout"] = result.stdout
                if result.stderr is not None:
                    native_metadata["stderr"] = result.stderr
            st.clear()
            st.update({"status": state_status, "input": native_input})
            if state_status == "pending":
                st["raw"] = ""
            else:
                st.update({"title": title[:80], "metadata": native_metadata,
                           "time": {"start": message_time}})
                if state_status in {"completed", "error"}:
                    st["time"]["end"] = message_time
                if state_status == "error":
                    st["error"] = (result.stderr if result and result.stderr
                                   else output or "Tool failed")
                elif state_status == "completed":
                    st["output"] = output
                else:
                    st["output"] = output
            if result is not None and result.attachments:
                st["attachments"] = result.attachments
            return add_part("tool", {"tool": tool,
                                     "callID": "call-" + secrets.token_hex(8),
                                     "state": st})

        for b in m.blocks:
            if b.kind == "text":
                add_part("text", {"text": b.text})
            elif b.kind == "tool":
                t = b.tool
                decision = tool_decider(t, sess, m) if tool_decider else None
                if t.op == CanonicalOp.AGENT_SPAWN:
                    candidates = [
                        candidate for candidate in sess.agent_edges
                        if id(candidate) not in emitted_edges]
                    edge = next((candidate for candidate in candidates
                                 if t.source_call_id and candidate.source_call_id ==
                                 t.source_call_id), None)
                    if edge is None:
                        at_message = [candidate for candidate in candidates
                                      if candidate.spawn_message_id == m.source_id]
                        edge = at_message[0] if len(at_message) == 1 else None
                    child = children.get(edge.child_session_id) if edge else None
                    child_sid = sid_map.get(edge.child_session_id) if edge else None
                    if (child is not None and child_sid is not None and
                            (decision is None or decision.rendered is not None)):
                        parts.append(_task_part(
                            tpl, sid, mid, len(parts), child, child_sid, edge,
                            message_time, t.source_call_id))
                        emitted_edges.add(id(edge))
                        linked_children.add(child.source_id)
                    else:
                        params = {"tool_name": t.name}
                        if decision is not None:
                            params.update({
                                "fidelity": decision.fidelity,
                                "reason_codes": list(decision.reason_codes),
                                "ignored_fields": sorted(decision.ignored_fields),
                            })
                        sess.lose("migration.tool_degraded", **params)
                        add_part("text", {"text": narrate(t)})
                    continue
                writer = OP_WRITERS.get(t.op)
                if ((decision is not None and decision.rendered is None) or
                        writer is None or not has_valid_tool_input(t.op, t.input) or
                        not writer(add_tool_part, t)):
                    params = {"tool_name": t.name}
                    if decision is not None:
                        params.update({
                            "fidelity": decision.fidelity,
                            "reason_codes": list(decision.reason_codes),
                            "ignored_fields": sorted(decision.ignored_fields),
                        })
                    sess.lose("migration.tool_degraded", **params)
                    add_part("text", {"text": narrate(t)})
        for child_id, edge in edges.items():
            if (child_id in linked_children or edge.spawn_message_id != m.source_id or
                    child_id not in children or child_id not in sid_map):
                continue
            parts.append(_task_part(
                tpl, sid, mid, len(parts), children[child_id], sid_map[child_id],
                edge, message_time))
            linked_children.add(child_id)
        if parts:
            if m.role == "assistant":
                minfo["finish"] = ("tool-calls" if any(
                    part.get("type") == "tool" for part in parts) else "stop")
            messages.append({"info": minfo, "parts": parts})
    return {"info": info, "messages": messages}


def _payload_from_records(sess: Session) -> dict | None:
    session_record = next((r for r in sess.raw_records
                           if r.source == "opencode" and
                           r.record_type == "session"), None)
    message_records = sorted(
        (r for r in sess.raw_records if r.source == "opencode" and
         r.record_type == "message"), key=lambda r: r.ordinal)
    if not session_record or not message_records:
        return None
    parts_by_message = {}
    for record in sess.raw_records:
        if record.source == "opencode" and record.record_type == "part":
            parts_by_message.setdefault(record.location, []).append(record)
    messages = []
    for record in message_records:
        mid = record.payload.get("id", "")
        parts = sorted(parts_by_message.get(mid, []), key=lambda r: r.ordinal)
        messages.append({"info": _clone(record.payload),
                         "parts": [_clone(part.payload) for part in parts]})
    return {"info": _clone(session_record.payload), "messages": messages}


def _raw_payload(sess: Session) -> dict | None:
    payload = sess.meta.get("opencode_export")
    if isinstance(payload, dict) and "info" in payload:
        return _clone(payload)
    return _payload_from_records(sess)


def _remap_payload(payload: dict, sid: str, cwd: str,
                   parent_sid: str | None, sid_map: dict[str, str]) -> dict:
    payload = _clone(payload)
    info = payload["info"]
    info["id"] = sid
    info["directory"] = cwd
    if parent_sid:
        info["parentID"] = parent_sid
    else:
        info.pop("parentID", None)

    message_ids = {}
    for message in payload.get("messages", []):
        old_id = message["info"].get("id")
        if old_id:
            message_ids[old_id] = _new_id("msg")

    last_user_mid = None
    for message in payload.get("messages", []):
        minfo = message["info"]
        old_id = minfo.get("id")
        mid = message_ids.get(old_id) or _new_id("msg")
        minfo["id"] = mid
        minfo["sessionID"] = sid
        if minfo.get("role") == "user":
            last_user_mid = mid
        elif minfo.get("role") == "assistant":
            minfo["parentID"] = message_ids.get(
                minfo.get("parentID"), last_user_mid)
            if minfo["parentID"] is None:
                minfo.pop("parentID")

        for ordinal, part in enumerate(message.get("parts", [])):
            part["id"] = _new_ordered_id("prt", ordinal)
            part["messageID"] = mid
            part["sessionID"] = sid
            if part.get("tool") == "task":
                state = part.get("state") if isinstance(part.get("state"), dict) else {}
                metadata = state.get("metadata") \
                    if isinstance(state.get("metadata"), dict) else {}
                state["metadata"] = metadata
                part["state"] = state
                metadata["parentSessionId"] = sid
                child_id = metadata.get("sessionId")
                if child_id in sid_map:
                    metadata["sessionId"] = sid_map[child_id]
    _normalize_payload_message_times(payload)
    return payload


def _ensure_task_links(payload: dict, sess: Session, sid: str,
                       sid_map: dict[str, str], tpl: dict) -> None:
    linked = set()
    for message in payload.get("messages", []):
        for part in message.get("parts", []):
            if part.get("tool") == "task":
                child_id = ((part.get("state") or {}).get("metadata") or {}).get(
                    "sessionId")
                if child_id:
                    linked.add(child_id)
                    if child_id in sid_map:
                        linked.add(sid_map[child_id])

    last_user = next(((message.get("info") or {}).get("id")
                      for message in reversed(payload.get("messages", []))
                      if (message.get("info") or {}).get("role") == "user"), None)
    message_times = []
    for message in payload.get("messages", []):
        minfo = message.get("info") or {}
        message_time = minfo.get("time") if isinstance(minfo.get("time"), dict) else {}
        message_times.append(message_time.get("created"))
    now = max((value for value in message_times if isinstance(value, int)),
              default=int(time.time() * 1000)) + 1
    edges = {edge.child_session_id: edge for edge in sess.agent_edges}
    for child in sess.children:
        target_child = sid_map[child.source_id]
        if target_child in linked:
            continue
        edge = edges.get(child.source_id)
        spawn_message = next((message for message in payload.get("messages", [])
                              if edge and edge.spawn_message_id and
                              message.get("info", {}).get("id") ==
                              edge.spawn_message_id), None)
        if spawn_message is not None:
            minfo = spawn_message["info"]
            when = iso_ms((minfo.get("time") or {}).get("created")) or now
            parts = spawn_message.setdefault("parts", [])
            parts.append(_task_part(
                tpl, sid, minfo["id"], len(parts), child, target_child, edge,
                when))
            if minfo.get("role") == "assistant":
                minfo["finish"] = "tool-calls"
                minfo.setdefault("time", {})["completed"] = max(
                    when, iso_ms((minfo.get("time") or {}).get("completed")) or when)
            linked.add(target_child)
            continue
        mid = _new_id("msg")
        minfo = _clone(tpl["msg.assistant"])
        cwd = payload["info"]["directory"]
        provider_id = str(sess.meta.get("model_provider") or "openai")
        model_id = str(sess.meta.get("model") or "gpt-5.6-sol")
        if last_user is None:
            last_user = _new_id("msg")
            user_info = _clone(tpl["msg.user"])
            user_info.update({
                "id": last_user, "sessionID": sid, "role": "user",
                "time": {"created": now - 1}, "agent": "build",
                "model": {"providerID": provider_id, "modelID": model_id},
                "summary": {"diffs": []},
            })
            user_part = _clone(tpl["part.text"])
            user_part.update({
                "id": _new_ordered_id("prt", 0), "messageID": last_user,
                "sessionID": sid, "type": "text",
                "text": "[Migrated subagent task]",
            })
            payload.setdefault("messages", []).append(
                {"info": user_info, "parts": [user_part]})
        minfo.update({"id": mid, "sessionID": sid,
                      "time": {"created": now, "completed": now},
                      "finish": "tool-calls", "mode": "build",
                      "agent": "build", "path": {"cwd": cwd, "root": cwd},
                      "cost": 0,
                      "tokens": {"total": 0, "input": 0, "output": 0,
                                 "reasoning": 0,
                                 "cache": {"write": 0, "read": 0}},
                      "modelID": model_id, "providerID": provider_id})
        if last_user:
            minfo["parentID"] = last_user
        else:
            minfo.pop("parentID", None)
        part = _task_part(tpl, sid, mid, 0, child, target_child, edge, now)
        payload.setdefault("messages", []).append({"info": minfo, "parts": [part]})
        linked.add(target_child)
        now += 1
    if sess.children:
        info = payload["info"]
        session_time = info.get("time") if isinstance(info.get("time"), dict) else {}
        session_time["updated"] = now - 1
        info["time"] = session_time


def _import_payload(payload: dict, sid: str, cwd: str) -> None:
    fd, path = tempfile.mkstemp(prefix=f"rh-import-{sid}-", suffix=".json")
    os.close(fd)
    tmp = Path(path)
    try:
        tmp.write_text(json.dumps(payload, ensure_ascii=False))
        # import 会把会话挂到进程 cwd 对应的项目,JSON 里的 directory 不生效
        out = _oc(["import", str(tmp)], cwd=cwd)
        if sid not in out:
            raise RuntimeError(f"import 结果异常: {out[-300:]}")
    finally:
        try:
            tmp.unlink()
        except OSError:
            pass


def write(sess: Session, cwd: str | None = None,
          tool_decider=None) -> tuple[str, Path]:
    sessions = list(sess.walk())
    sid_map = {node.source_id: _new_id("ses") for node in sessions}
    parent_map = {}
    for parent in sessions:
        for child in parent.children:
            parent_map[id(child)] = sid_map[parent.source_id]

    target_cwd = str(Path(cwd or sess.cwd).resolve())
    tpl = None
    prepared = []
    for node in sessions:
        sid = sid_map[node.source_id]
        node_cwd = target_cwd if cwd is not None else str(
            Path(node.cwd or target_cwd).resolve())
        parent_sid = parent_map.get(id(node))
        payload = _raw_payload(node)
        is_raw = payload is not None
        if payload is not None:
            if node.children:
                if tpl is None:
                    tpl = _template()
                # 原生 payload 尚未重映射时，edge.spawn_message_id 仍可精确定位。
                _ensure_task_links(payload, node, sid, sid_map, tpl)
            payload = _remap_payload(payload, sid, node_cwd, parent_sid, sid_map)
        else:
            if tpl is None:
                tpl = _template()
            payload = _canonical_payload(
                node, sid, node_cwd, parent_sid, tpl, sid_map=sid_map,
                tool_decider=tool_decider)
        if node.children and not is_raw:
            if tpl is None:
                tpl = _template()
            _ensure_task_links(payload, node, sid, sid_map, tpl)
        prepared.append((payload, sid, node_cwd))

    imported = []
    try:
        for payload, sid, node_cwd in prepared:
            # import 可能先插入 session 再因消息 schema 失败；调用前登记，
            # 确保半写入的当前会话也进入回滚。
            imported.append(sid)
            _import_payload(payload, sid, node_cwd)
    except Exception:
        for imported_sid in reversed(imported):
            try:
                _oc(["session", "delete", imported_sid], cwd=target_cwd)
            except Exception:
                pass
        raise

    return sid_map[sess.source_id], OPENCODE_DB
