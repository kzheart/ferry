"""Claude Code writer: 规范化会话树 -> 主会话与 subagent JSONL。"""
import json
import re
import time
import uuid
from pathlib import Path

from ...domain.model import (
    AgentEdge,
    Session,
    ToolCall,
    ToolResult,
    ToolResultBlock,
)
from ...domain.tool_ops import CanonicalOp, has_valid_tool_input
from ..base.narration import narrate
from .native_schema import templates


OP_WRITERS = {
    CanonicalOp.SHELL_EXEC: ("Bash", lambda i: {
        "command": i.get("command", ""),
        **({"timeout": i["timeout_ms"]} if "timeout_ms" in i else {}),
        **({"run_in_background": i["background"]} if "background" in i else {}),
    }),
    CanonicalOp.FS_WRITE: ("Write", lambda i: {"file_path": i.get("file_path", ""),
                                                 "content": i.get("content", "")}),
    CanonicalOp.FS_READ: ("Read", lambda i: {
        "file_path": i.get("file_path", ""),
        **{key: i[key] for key in ("offset", "limit") if key in i},
    }),
    CanonicalOp.FS_EDIT: ("Edit", lambda i: {"file_path": i.get("file_path", ""),
                                               "old_string": i.get("old", ""),
                                               "new_string": i.get("new", ""),
                                               **({"replace_all": i["replace_all"]}
                                                  if "replace_all" in i else {})}),
    CanonicalOp.FS_SEARCH: ("Grep", lambda i: {
        "pattern": i.get("query", ""),
        **({"path": i["path"]} if "path" in i else {}),
        **({"glob": i["glob"]} if "glob" in i else {}),
    }),
    CanonicalOp.FS_GLOB: ("Glob", lambda i: {
        "pattern": i.get("pattern", ""),
        **({"path": i["path"]} if "path" in i else {}),
    }),
    CanonicalOp.WEB_FETCH: ("WebFetch", lambda i: {
        "url": i.get("url", ""),
        "prompt": i.get("prompt", "Fetch this URL and preserve its relevant content."),
    }),
    CanonicalOp.WEB_SEARCH: ("WebSearch", lambda i: {
        "query": i.get("query", ""),
    }),
}

OP_FIDELITY = {op: "native" for op in OP_WRITERS} | {
    CanonicalOp.AGENT_SPAWN: "native",
    CanonicalOp.FS_PATCH: "degrade",
    CanonicalOp.TOOL_INVOKE: "degrade",
}


def _agent_input(value: dict) -> dict:
    return {
        "description": value.get("description", ""),
        "prompt": value.get("prompt", ""),
        **(
            {"subagent_type": value["subagent_type"]}
            if value.get("subagent_type")
            else {}
        ),
        **({"name": value["task_name"]} if "task_name" in value else {}),
        **({"model": value["model"]} if "model" in value else {}),
        **({"mode": value["fork_mode"]} if "fork_mode" in value else {}),
        **(
            {"reasoning_effort": value["reasoning_effort"]}
            if "reasoning_effort" in value
            else {}
        ),
    }


def _slug(path: str) -> str:
    return re.sub(r"[^A-Za-z0-9]", "-", str(Path(path).resolve()))


def _load_templates():
    return templates()


def _clone(value):
    return json.loads(json.dumps(value))


def _result_block_payload(block) -> dict:
    if block.kind == "text":
        return {"type": "text", "text": block.text}
    if block.kind == "image" and block.data is not None:
        return {
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": block.mime_type or "application/octet-stream",
                "data": block.data,
            },
        }
    if block.kind == "tool_reference":
        return {"type": "tool_reference", **block.metadata}
    value = {
        "kind": block.kind, "data": block.data, "mime_type": block.mime_type,
        "filename": block.filename, "uri": block.uri,
        "metadata": block.metadata,
    }
    return {"type": "text", "text": json.dumps(value, ensure_ascii=False)}


def _claude_result(tool) -> tuple[str | list, dict]:
    result = tool.result
    if result is None:
        return "", {
            "status": "unknown",
            "stdout": "",
            "stderr": "",
            "interrupted": False,
            "isImage": False,
        }
    content = [_result_block_payload(block) for block in result.blocks]
    native = {
        "status": result.status,
        "interrupted": result.status == "interrupted",
        "isImage": any(block.kind == "image" for block in result.blocks),
        "canonicalToolResult": {
            "status": result.status,
            "attachments": result.attachments,
            "metadata": result.metadata,
            "blocks": [{
                "kind": block.kind,
                "text": block.text,
                "data": block.data,
                "mime_type": block.mime_type,
                "filename": block.filename,
                "uri": block.uri,
                "metadata": block.metadata,
            } for block in result.blocks],
        },
    }
    if result.stdout is not None:
        native["stdout"] = result.stdout
    if result.stderr is not None:
        native["stderr"] = result.stderr
    if result.exit_code is not None:
        native["exit_code"] = result.exit_code
    if result.truncated is not None:
        native["truncated"] = result.truncated
    return content, native


def _agent_ids(session: Session) -> dict[str, str]:
    result = {}
    for parent in session.walk():
        edges = {edge.child_session_id: edge for edge in parent.agent_edges}
        for child in parent.children:
            new_id = "a" + uuid.uuid4().hex[:16]
            result[child.source_id] = new_id
            if child.agent_id:
                result[child.agent_id] = new_id
            edge = edges.get(child.source_id)
            if edge and edge.agent_id:
                result[edge.agent_id] = new_id
    return result


def _uuid_map(session: Session) -> dict[str, str]:
    values = []
    for node in session.walk():
        for raw in node.raw_records:
            payload = raw.payload if isinstance(raw.payload, dict) else {}
            if payload.get("uuid"):
                values.append(payload["uuid"])
    return {old: str(uuid.uuid4()) for old in dict.fromkeys(values)}


def _iso_now(offset: float = 0) -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%S",
                         time.gmtime(time.time() + offset)) + ".000Z"


def _ensure_resume_fields(record: dict, *, cwd: str | None = None,
                          stamp: str | None = None) -> dict:
    """Claude CLI refuses to resume sessions missing these conversation fields."""
    kind = record.get("type")
    if kind in ("user", "assistant", "system"):
        record.setdefault("timestamp", stamp or _iso_now())
        record.setdefault("userType", "external")
        if cwd:
            record["cwd"] = cwd
        record.setdefault("version", record.get("version") or "ferry")
        record.setdefault("isSidechain", False)
    elif kind in ("queue-operation", "progress", "last-prompt") and stamp:
        record.setdefault("timestamp", stamp)
    if cwd and "cwd" in record:
        record["cwd"] = cwd
    return record


def _rewrite_record(value: dict, sid: str, agent_map: dict[str, str],
                    uuid_map: dict[str, str], cwd: str | None = None,
                    stamp: str | None = None) -> dict:
    record = _clone(value)
    for key in ("sessionId", "parentSessionId"):
        if key in record:
            record[key] = sid
    if isinstance(record.get("agentId"), str):
        record["agentId"] = agent_map.get(record["agentId"], record["agentId"])
    for key in ("uuid", "parentUuid", "sourceToolAssistantUUID",
                "parentLastUuid", "leafUuid"):
        if isinstance(record.get(key), str):
            record[key] = uuid_map.get(record[key], record[key])
    result = record.get("toolUseResult")
    if isinstance(result, dict):
        if isinstance(result.get("agentId"), str):
            result["agentId"] = agent_map.get(
                result["agentId"], result["agentId"]
            )
    return _ensure_resume_fields(record, cwd=cwd, stamp=stamp)


def _raw_lines(session: Session, sid: str, agent_map: dict[str, str],
                uuid_map: dict[str, str], cwd: str | None = None) -> list[dict]:
    lines = []
    start = time.time() - 2
    for index, raw in enumerate(
            sorted(session.raw_records, key=lambda item: item.ordinal)):
        if not isinstance(raw.payload, dict) or raw.record_type.startswith("workflow."):
            continue
        stamp = time.strftime("%Y-%m-%dT%H:%M:%S",
                              time.gmtime(start + index * 2)) + ".000Z"
        lines.append(_rewrite_record(
            raw.payload, sid, agent_map, uuid_map, cwd=cwd, stamp=stamp))
    return lines


def _child_path(destination: Path, sid: str, child: Session,
                new_agent: str) -> Path:
    source = Path(child.agent_path or "")
    parts = source.parts
    if "workflows" in parts:
        index = parts.index("workflows")
        suffix = Path(*parts[index:index + 2])
        return destination / sid / "subagents" / suffix / f"agent-{new_agent}.jsonl"
    return destination / sid / "subagents" / f"agent-{new_agent}.jsonl"


def _edge_for_tool(session: Session, tool) -> AgentEdge | None:
    for edge in session.agent_edges:
        if tool.source_call_id and edge.source_call_id == tool.source_call_id:
            return edge
        if tool.meta.get("agentId") and edge.agent_id == tool.meta["agentId"]:
            return edge
    return None


def _generated_lines(session: Session, sid: str, cwd: str, templates: dict,
                     agent_map: dict[str, str], source_uuids: dict[str, str],
                     fork_parent: str | None = None,
                     tool_decider=None) -> list[dict]:
    records, parent = [], None
    agent_id = agent_map.get(session.source_id)
    timestamp = time.time() - len(session.messages) * 2

    if agent_id:
        records.append({"type": "fork-context-ref", "agentId": agent_id,
                        "parentSessionId": sid,
                        "parentLastUuid": fork_parent,
                        "contextLength": session.meta.get(
                            "fork_context_ref", {}).get("contextLength", 0)})

    def base(kind: str) -> dict:
        nonlocal parent, timestamp
        record = _clone(templates[kind])
        record["uuid"] = str(uuid.uuid4())
        record["parentUuid"] = parent
        record["sessionId"] = sid
        record["cwd"] = cwd
        record["isSidechain"] = bool(agent_id)
        if agent_id:
            record["agentId"] = agent_id
        else:
            record.pop("agentId", None)
        timestamp += 2
        record["timestamp"] = time.strftime(
            "%Y-%m-%dT%H:%M:%S", time.gmtime(timestamp)) + ".000Z"
        record["userType"] = "external"
        record.setdefault("version", record.get("version") or "ferry")
        for key in ("toolUseResult", "sourceToolAssistantUUID", "promptSource"):
            record.pop(key, None)
        parent = record["uuid"]
        return _ensure_resume_fields(record, cwd=cwd, stamp=record["timestamp"])

    def remember(message, record, *, replace=False):
        if message.source_id:
            if replace:
                source_uuids[message.source_id] = record["uuid"]
            else:
                source_uuids.setdefault(message.source_id, record["uuid"])

    def add_text(message, text):
        kind = "assistant" if message.role == "assistant" else "user"
        record = base(kind)
        record["message"] = ({"role": "user", "content": text}
                             if kind == "user" else
                             {**record["message"], "content": [
                                 {"type": "text", "text": text}]})
        records.append(record)
        remember(message, record)

    emitted_children = set()

    def add_tool(message, tool, edge_override=None):
        edge = edge_override or (_edge_for_tool(session, tool)
                                  if tool.op == CanonicalOp.AGENT_SPAWN or
                                  tool.name == "Agent" else None)
        if edge:
            native_name = "Agent"
            native_input = (
                _agent_input(tool.input) if isinstance(tool.input, dict) else {}
            )
            emitted_children.add(edge.child_session_id)
        elif tool.op == CanonicalOp.TOOL_INVOKE:
            native_name = str(tool.input["name"])
            native_input = _clone(tool.input["input"])
        else:
            native_name, convert = OP_WRITERS[tool.op]
            native_input = convert(tool.input)
        use_id = "toolu_" + uuid.uuid4().hex[:24]
        assistant = base("assistant")
        assistant["message"]["content"] = [{
            "type": "tool_use", "id": use_id,
            "name": native_name, "input": native_input}]
        records.append(assistant)
        if message is not None:
            # 子会话必须 fork 在真实 Agent 调用上，而不是同一 canonical
            # 消息里先输出的文字记录上。
            remember(message, assistant, replace=edge is not None)
        elif edge and edge.spawn_message_id:
            source_uuids[edge.spawn_message_id] = assistant["uuid"]

        result_content, native_result = _claude_result(tool)
        user = base("user")
        user["message"] = {"role": "user", "content": [{
            "type": "tool_result", "tool_use_id": use_id,
            "content": result_content,
            **({"is_error": True} if tool.result and
               tool.result.status == "error" else {}),
        }]}
        if native_name == "Bash":
            user["toolUseResult"] = native_result
        elif edge:
            old_agent = edge.child_session_id
            result = _clone(edge.meta.get("toolUseResult", {}))
            result.update({"agentId": agent_map.get(old_agent, old_agent),
                           "status": edge.status or result.get("status", "completed")})
            user["toolUseResult"] = result
        elif tool.result is not None:
            user["toolUseResult"] = native_result
        records.append(user)

    for message in session.messages:
        texts = []
        for block in message.blocks:
            if block.kind == "text":
                texts.append(block.text)
            elif block.kind == "tool" and block.tool:
                tool = block.tool
                decision = tool_decider(
                    tool, session, message) if tool_decider else None
                native = decision.rendered is not None if decision is not None else (
                    (tool.op in OP_WRITERS and
                     has_valid_tool_input(tool.op, tool.input))
                    or ((tool.op == CanonicalOp.AGENT_SPAWN or
                         tool.name == "Agent") and
                        has_valid_tool_input(tool.op, tool.input) and
                        _edge_for_tool(session, tool)))
                if native:
                    if texts:
                        add_text(message, "\n\n".join(texts))
                        texts = []
                    add_tool(message, tool)
                else:
                    params = {"tool_name": tool.name}
                    if decision is not None:
                        params.update({
                            "fidelity": decision.fidelity,
                            "reason_codes": list(decision.reason_codes),
                            "ignored_fields": sorted(decision.ignored_fields),
                        })
                    session.lose("migration.tool_degraded", **params)
                    texts.append(narrate(tool))
        if texts:
            add_text(message, "\n\n".join(texts))

    children = {child.source_id: child for child in session.children}
    for edge in session.agent_edges:
        child = children.get(edge.child_session_id)
        if child is None or edge.child_session_id in emitted_children:
            continue
        result = ""
        for message in reversed(child.messages):
            if message.role == "assistant":
                result = "\n".join(block.text for block in message.blocks
                                   if block.kind == "text" and block.text)
                if result:
                    break
        tool = ToolCall(
            name="Agent", op=CanonicalOp.AGENT_SPAWN,
            input={"description": child.title or "migrated subagent",
                   "prompt": edge.prompt,
                   "subagent_type": edge.agent_type or child.agent_type or "general"},
            result=ToolResult(
                status="success",
                blocks=[ToolResultBlock("text", text=result)] if result else [],
            ))
        add_tool(None, tool, edge)
    return records


def _write_jsonl(path: Path, records: list[dict]):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text("\n".join(json.dumps(record, ensure_ascii=False)
                                    for record in records) + "\n")
    temporary.replace(path)


def write(sess: Session, cwd: str | None = None,
          dest_root: str | Path | None = None,
          tool_decider=None) -> tuple[str, Path]:
    """写出会话树，返回根会话的新 ID 与主 JSONL 路径。"""
    templates = _load_templates()
    sid = str(uuid.uuid4())
    cwd = cwd or sess.cwd
    destination = (Path(dest_root) if dest_root is not None else
                   Path.home() / ".claude" / "projects" / _slug(cwd))
    main_path = destination / f"{sid}.jsonl"
    agent_map = _agent_ids(sess)
    use_raw = sess.source_tool == "claude" and all(
        node.raw_records for node in sess.walk())

    created = []
    try:
        if use_raw:
            uuid_map = _uuid_map(sess)
            _write_jsonl(main_path, _raw_lines(sess, sid, agent_map, uuid_map, cwd))
            created.append(main_path)
            for child in list(sess.walk())[1:]:
                new_agent = agent_map.get(child.source_id)
                child_path = _child_path(destination, sid, child, new_agent)
                _write_jsonl(child_path, _raw_lines(
                    child, sid, agent_map, uuid_map, child.cwd or cwd))
                created.append(child_path)
            for relative, records in sess.meta.get("workflow_journals", {}).items():
                source = Path(relative)
                parts = source.parts
                index = parts.index("workflows") if "workflows" in parts else 0
                target = destination / sid / "subagents" / Path(*parts[index:])
                rewritten = [
                    _rewrite_record(record, sid, agent_map, uuid_map, cwd=cwd)
                    for record in records]
                _write_jsonl(target, rewritten)
                created.append(target)
            return sid, main_path

        source_uuids = {}
        root_records = _generated_lines(
            sess, sid, cwd, templates, agent_map, source_uuids,
            tool_decider=tool_decider)
        _write_jsonl(main_path, root_records)
        created.append(main_path)
        edges = {edge.child_session_id: edge for node in sess.walk()
                 for edge in node.agent_edges}
        for child in list(sess.walk())[1:]:
            edge = edges.get(child.source_id)
            old_parent_uuid = (child.forked_from_id or
                               (edge.spawn_message_id if edge else None))
            fork_parent = source_uuids.get(old_parent_uuid)
            if fork_parent is None and root_records:
                fork_parent = root_records[-1].get("uuid")
                child.lose("migration.fork_parent_fallback")
            records = _generated_lines(child, sid, child.cwd or cwd, templates,
                                       agent_map, source_uuids, fork_parent,
                                       tool_decider)
            new_agent = agent_map.get(child.source_id)
            child_path = _child_path(destination, sid, child, new_agent)
            _write_jsonl(child_path, records)
            created.append(child_path)
        return sid, main_path
    except Exception:
        for path in created:
            path.unlink(missing_ok=True)
        raise
