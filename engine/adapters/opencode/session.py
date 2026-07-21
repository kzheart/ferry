"""OpenCode reader/writer:走官方 `opencode export` / `opencode import`,不直接碰 SQLite。

处理 OpenCode 会话存储。export 形状:
    {"info": <session 行>, "messages": [{"info": <message.data>, "parts": [<part.data>...]}]}
"""
import json
import os
import secrets
import sqlite3
import subprocess
import tempfile
import time
from pathlib import Path

from ...domain.model import AgentEdge, Block, Message, RawRecord, Session, ToolCall
from ...domain.reasoning import visible_text
from ...infrastructure import executables
from ...infrastructure.resources import resource_path
from ..base.narration import narrate

TOOL_OPS = {"bash": "shell.exec", "read": "fs.read",
            "write": "fs.write", "edit": "fs.edit"}
GOLDEN = resource_path("golden", "opencode")
OPENCODE_DB = Path.home() / ".local" / "share" / "opencode" / "opencode.db"


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


def _export_from_capture(capture: dict) -> dict:
    """把 golden 中的数据库行快照还原为官方 export 形状。"""
    info = json.loads(capture["session"]["data"]) \
        if "data" in capture["session"] else dict(capture["session"])
    parts = {}
    for row in capture.get("parts", []):
        part = json.loads(row["data"])
        parts.setdefault(row["message_id"], []).append(part)
    messages = []
    for row in capture.get("messages", []):
        messages.append({"info": json.loads(row["data"]),
                         "parts": parts.get(row["id"], [])})
    return {"info": info, "messages": messages}


# ---------- reader ----------

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
    for ordinal, m in enumerate(data.get("messages", [])):
        role = m["info"].get("role", "user")
        blocks = []
        mid = m["info"].get("id")
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
                inp = dict(source_input or {}) if isinstance(source_input, dict) \
                    else (source_input or "")
                if isinstance(inp, dict):
                    if "filePath" in inp:      # 归一化为规范参数名
                        inp["file_path"] = inp.pop("filePath")
                    if "oldString" in inp:
                        inp["old"] = inp.pop("oldString")
                    if "newString" in inp:
                        inp["new"] = inp.pop("newString")
                blocks.append(Block("tool", tool=ToolCall(
                    name=p.get("tool", "?"),
                    op=("agent.spawn" if p.get("tool") == "task" else
                        TOOL_OPS.get(p.get("tool"))),
                    input=inp,
                    output=st.get("output", ""),
                    meta=_clone(st.get("metadata") or {}),
                    source_call_id=p.get("callID"), status=st.get("status"),
                    started_at=(st.get("time") or {}).get("start"),
                    ended_at=(st.get("time") or {}).get("end"))))
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
    return sess, edges


def read(session_id: str) -> Session:
    seen: dict[str, Session] = {}
    conn = _db_conn()

    def export(sid: str) -> dict:
        if conn is not None:
            data = _db_export(conn, sid)
            if data is not None:
                return data
        return _oc_export(sid)

    def child_ids_of(sid: str) -> list[str]:
        if conn is not None:
            try:
                return [row[0] for row in conn.execute(
                    "SELECT id FROM session WHERE parent_id = ? "
                    "ORDER BY time_created, id", (sid,))]
            except sqlite3.Error:
                pass
        return _db_child_ids(sid)

    def visit(sid: str, root_id: str) -> Session:
        if sid in seen:
            return seen[sid]
        sess, task_edges = _parse_session(export(sid))
        seen[sid] = sess
        sess.root_id = root_id

        task_by_child = {edge.child_session_id: edge for edge in task_edges}
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
            edge = task_by_child.get(child_id) or AgentEdge(
                parent_session_id=sid, child_session_id=child_id)
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


# ---------- writer ----------

def _template():
    """用黄金样本会话的官方 export 作为结构模板。"""
    versions = sorted(GOLDEN.iterdir()) if GOLDEN.exists() else []
    if not versions:
        raise RuntimeError("缺少 golden/opencode 样本")
    sample = versions[-1] / "case-02-tools"
    captured = sample / "session.json"
    if captured.exists():
        data = _export_from_capture(json.loads(captured.read_text()))
    else:
        manifest = json.loads((sample / "manifest.json").read_text())
        data = _oc_export(manifest["session_id"])
    tpl = {"info": data["info"]}
    for m in data["messages"]:
        role = m["info"].get("role")
        tpl.setdefault(f"msg.{role}", m["info"])
        for p in m["parts"]:
            tpl.setdefault(f"part.{p.get('type')}", p)
    return tpl


def _clone(o):
    return json.loads(json.dumps(o))


def _canonical_payload(sess: Session, sid: str, cwd: str, parent_sid: str | None,
                       tpl: dict) -> dict:
    now = int(time.time() * 1000)
    info = _clone(tpl["info"])
    info.update({"id": sid, "directory": cwd,
                 "title": sess.title or f"migrated from {sess.source_tool}",
                 "time": {"created": now, "updated": now}})
    if parent_sid:
        info["parentID"] = parent_sid
    else:
        info.pop("parentID", None)
    for k in ("share",):
        info.pop(k, None)

    messages = []
    last_user_mid = None
    for m in sess.messages:
        mid = _new_id("msg")
        minfo = _clone(tpl.get(f"msg.{m.role}", tpl["msg.user"]))
        minfo.update({"id": mid, "sessionID": sid})
        if m.role == "assistant":
            # completed + finish 缺失会让 runtime 认为该轮未结束而死循环
            minfo["time"] = {"created": now, "completed": now}
            minfo["finish"] = ("tool-calls" if any(
                b.kind == "tool" for b in m.blocks) else "stop")
            if last_user_mid:
                minfo["parentID"] = last_user_mid
            else:
                minfo.pop("parentID", None)
        else:
            minfo["time"] = {"created": now}
            last_user_mid = mid
        parts = []

        def add_part(ptype, fill):
            key = f"part.{ptype}"
            if key not in tpl:
                return False
            p = _clone(tpl[key])
            p.update({"id": _new_id("prt"), "messageID": mid,
                      "sessionID": sid})
            p.update(fill)
            parts.append(p)
            return True

        def add_tool_part(tool, native_input, output, title, metadata):
            st = _clone(tpl["part.tool"]["state"])
            st.update({"status": "completed", "input": native_input,
                       "output": output, "title": title[:80],
                       "metadata": metadata})
            return add_part("tool", {"tool": tool,
                                     "callID": "call-" + secrets.token_hex(8),
                                     "state": st})

        for b in m.blocks:
            if b.kind == "text":
                add_part("text", {"text": b.text})
            elif b.kind == "tool":
                t = b.tool
                i = t.input if isinstance(t.input, dict) else {}
                # 常用工具原生映射
                if t.op == "shell.exec" and i.get("command"):
                    add_tool_part("bash", {"command": i["command"]},
                                  t.output, i["command"],
                                  {"output": t.output,
                                   "exit": t.meta.get("exit", 0) or 0,
                                   "truncated": False})
                elif t.op == "fs.read" and i.get("file_path"):
                    add_tool_part("read", {"filePath": i["file_path"]},
                                  t.output, i["file_path"],
                                  {"truncated": False})
                elif t.op == "fs.write" and i.get("file_path"):
                    add_tool_part("write", {"filePath": i["file_path"],
                                            "content": i.get("content", "")},
                                  t.output or "Wrote file successfully.",
                                  i["file_path"],
                                  {"filepath": i["file_path"],
                                   "exists": False, "truncated": False,
                                   "diagnostics": {}})
                elif t.op == "fs.edit" and i.get("file_path"):
                    add_tool_part("edit", {"filePath": i["file_path"],
                                           "oldString": i.get("old", ""),
                                           "newString": i.get("new", "")},
                                  t.output or "Edited file.",
                                  i["file_path"], {"truncated": False})
                else:
                    sess.lose("migration.tool_degraded", tool_name=t.name)
                    add_part("text", {"text": narrate(t)})
        if parts:
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

        for part in message.get("parts", []):
            part["id"] = _new_id("prt")
            part["messageID"] = mid
            part["sessionID"] = sid
            if part.get("tool") == "task":
                metadata = (part.get("state") or {}).get("metadata") or {}
                child_id = metadata.get("sessionId")
                if child_id in sid_map:
                    metadata["sessionId"] = sid_map[child_id]
    return payload


def _assistant_result(sess: Session) -> str:
    for message in reversed(sess.messages):
        if message.role == "assistant":
            text = "\n".join(block.text for block in message.blocks
                             if block.kind == "text" and block.text)
            if text:
                return text
    return ""


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

    last_user = next((message["info"]["id"]
                      for message in reversed(payload.get("messages", []))
                      if message["info"].get("role") == "user"), None)
    now = int(time.time() * 1000)
    edges = {edge.child_session_id: edge for edge in sess.agent_edges}
    for child in sess.children:
        target_child = sid_map[child.source_id]
        if target_child in linked:
            continue
        edge = edges.get(child.source_id)
        mid = _new_id("msg")
        minfo = _clone(tpl["msg.assistant"])
        minfo.update({"id": mid, "sessionID": sid,
                      "time": {"created": now, "completed": now},
                      "finish": "tool-calls"})
        if last_user:
            minfo["parentID"] = last_user
        else:
            minfo.pop("parentID", None)
        part = _clone(tpl["part.tool"])
        prompt = edge.prompt if edge else ""
        part.update({
            "id": _new_id("prt"), "messageID": mid, "sessionID": sid,
            "type": "tool", "tool": "task",
            "callID": (edge.source_call_id if edge and edge.source_call_id
                       else "call-" + secrets.token_hex(8)),
            "state": {
                "status": edge.status if edge and edge.status else "completed",
                "input": {"description": child.title or "migrated subagent",
                          "prompt": prompt,
                          "subagent_type": (edge.agent_type if edge else None)
                          or child.agent_type or "general"},
                "output": _assistant_result(child),
                "title": child.title or "Subagent",
                "metadata": {"parentSessionId": sid,
                             "sessionId": target_child},
                "time": {"start": now, "end": now},
            },
        })
        payload.setdefault("messages", []).append({"info": minfo, "parts": [part]})


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


def write(sess: Session, cwd: str | None = None) -> tuple[str, Path]:
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
        if payload is not None:
            payload = _remap_payload(payload, sid, node_cwd, parent_sid, sid_map)
        else:
            if tpl is None:
                tpl = _template()
            payload = _canonical_payload(node, sid, node_cwd, parent_sid, tpl)
        if node.children:
            if tpl is None:
                tpl = _template()
            _ensure_task_links(payload, node, sid, sid_map, tpl)
        prepared.append((payload, sid, node_cwd))

    imported = []
    try:
        for payload, sid, node_cwd in prepared:
            _import_payload(payload, sid, node_cwd)
            imported.append(sid)
    except Exception:
        for imported_sid in reversed(imported):
            try:
                _oc(["session", "delete", imported_sid], cwd=target_cwd)
            except Exception:
                pass
        raise

    return sid_map[sess.source_id], OPENCODE_DB
