"""Canonical Session 到 OpenCode 当前原生结构的写入转换。"""
import json
import secrets
import time
from pathlib import Path

from ...sessions.model import (
    AgentEdge,
    Message,
    Session,
    tool_result_text,
)
from ...sessions.tool_ops import CanonicalOp, has_valid_tool_input
from ...sessions.usage import iso_ms
from ..shared.narration import narrate
from .native_schema import templates
from . import store as native_store

def _new_id(prefix: str) -> str:
    return f"{prefix}_{secrets.token_hex(6)}{secrets.token_urlsafe(12)[:14]}"


def _new_ordered_id(prefix: str, ordinal: int) -> str:
    """生成同一父记录内可按字典序恢复原顺序的 ID。"""
    return f"{prefix}_{ordinal:08x}{secrets.token_hex(10)}"


def _template():
    return templates()


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
    output = tool_result_text(tool.result)
    return add_tool_part(
        "bash", native_input, output, command,
        {"output": output,
         "exit": tool.result.exit_code
         if tool.result and tool.result.exit_code is not None else 0,
         "truncated": bool(tool.result and tool.result.truncated)}, tool)


def _write_fs_read(add_tool_part, tool) -> bool:
    inputs = tool.input if isinstance(tool.input, dict) else {}
    path = inputs.get("file_path")
    if not path:
        return False
    native_input = {"filePath": path}
    native_input.update({key: inputs[key] for key in ("offset", "limit")
                         if key in inputs})
    return add_tool_part(
        "read", native_input, tool_result_text(tool.result), path,
        {"truncated": bool(tool.result and tool.result.truncated)}, tool)


def _write_fs_write(add_tool_part, tool) -> bool:
    inputs = tool.input if isinstance(tool.input, dict) else {}
    path = inputs.get("file_path")
    if not path:
        return False
    return add_tool_part(
        "write", {"filePath": path, "content": inputs.get("content", "")},
        tool_result_text(tool.result) or "Wrote file successfully.", path,
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
        tool_result_text(tool.result) or "Edited file.", path,
        {"truncated": False}, tool)


def _write_fs_patch(add_tool_part, tool) -> bool:
    inputs = tool.input if isinstance(tool.input, dict) else {}
    patch = inputs.get("raw_patch")
    if not patch:
        return False
    return add_tool_part(
        "apply_patch", {"patchText": patch},
        tool_result_text(tool.result) or "Applied patch.",
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
    return add_tool_part(
        "grep", native_input, tool_result_text(tool.result), str(query),
        {"truncated": False}, tool)


def _write_fs_glob(add_tool_part, tool) -> bool:
    inputs = tool.input if isinstance(tool.input, dict) else {}
    pattern = inputs.get("pattern")
    if not pattern:
        return False
    native_input = {"pattern": pattern}
    if "path" in inputs:
        native_input["path"] = inputs["path"]
    return add_tool_part(
        "glob", native_input, tool_result_text(tool.result), str(pattern),
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
    return add_tool_part(
        "webfetch", native_input, tool_result_text(tool.result), str(url),
        {"truncated": False}, tool)


def _write_web_search(add_tool_part, tool) -> bool:
    inputs = tool.input if isinstance(tool.input, dict) else {}
    query = inputs.get("query")
    if not query:
        return False
    native_input = {"query": query}
    if "num_results" in inputs:
        native_input["numResults"] = inputs["num_results"]
    return add_tool_part(
        "websearch", native_input, tool_result_text(tool.result), str(query),
        {"truncated": False}, tool)


def _write_tool_invoke(add_tool_part, tool) -> bool:
    inputs = tool.input if isinstance(tool.input, dict) else {}
    name = inputs.get("name")
    native_input = inputs.get("input")
    if not name or not isinstance(native_input, (dict, str)):
        return False
    return add_tool_part(
        str(name), native_input, tool_result_text(tool.result), str(name),
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
    provider_id = str(sess.model_provider or "openai")
    model_id = str(sess.model or "gpt-5.6-sol")
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
        provider_id = str(sess.model_provider or "openai")
        model_id = str(sess.model or "gpt-5.6-sol")
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


def write(sess: Session, cwd: str | None = None,
          tool_decider=None,
          native_payloads: dict[str, dict] | None = None) -> tuple[str, Path]:
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
        explicit_payload = (native_payloads or {}).get(node.source_id)
        payload = (_clone(explicit_payload)
                   if isinstance(explicit_payload, dict) else None)
        has_native_payload = payload is not None
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
        if node.children and not has_native_payload:
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
            native_store.import_payload(payload, sid, node_cwd)
    except Exception:
        for imported_sid in reversed(imported):
            try:
                native_store.delete_session(imported_sid, cwd=target_cwd)
            except Exception:
                pass
        raise

    return sid_map[sess.source_id], native_store.DB_PATH
