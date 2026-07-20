"""Claude Code reader: JSONL 会话文件 -> 规范化会话树。"""
import json
from pathlib import Path

from ...domain.model import AgentEdge, Block, Message, RawRecord, Session, ToolCall
from ...domain.reasoning import visible_text

TOOL_OPS = {"Bash": "shell.exec", "Read": "fs.read",
            "Write": "fs.write", "Edit": "fs.edit"}


def _norm_input(name: str, inp: dict) -> dict:
    if name == "Edit":
        return {"file_path": inp.get("file_path", ""),
                "old": inp.get("old_string", ""),
                "new": inp.get("new_string", "")}
    if name == "Read":
        return {"file_path": inp.get("file_path", "")}
    if name == "Write":
        return {"file_path": inp.get("file_path", ""),
                "content": inp.get("content", "")}
    if name == "Bash":
        return {"command": inp.get("command", "")}
    return inp


def _result_text(block) -> str:
    content = block.get("content")
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return "\n".join(item.get("text", "") for item in content
                         if isinstance(item, dict) and
                         item.get("type") == "text")
    return ""


def _load(path: Path) -> list[dict]:
    return [json.loads(line) for line in path.read_text().splitlines()
            if line.strip()]


def _agent_id(lines: list[dict], path: Path) -> str | None:
    for record in lines:
        if record.get("agentId"):
            return record["agentId"]
    name = path.stem
    return name[6:] if name.startswith("agent-") else None


def _read_transcript(path: Path, is_child: bool = False) -> Session:
    lines = _load(path)
    messages = [record for record in lines
                if record.get("type") in ("user", "assistant") and
                (is_child or not record.get("isSidechain"))]
    first = messages[0] if messages else {}
    agent_id = _agent_id(lines, path) if is_child else None
    source_id = agent_id or first.get("sessionId", path.stem)
    session = Session(source_tool="claude", source_id=source_id,
                      cwd=first.get("cwd", ""), agent_id=agent_id)
    session.raw_records = [
        RawRecord(source="claude", record_type=record.get("type", ""),
                  payload=record, ordinal=index,
                  timestamp=record.get("timestamp"), location=str(path))
        for index, record in enumerate(lines)
    ]

    for record in lines:
        if record.get("type") == "ai-title":
            session.title = record.get("title") or record.get("aiTitle", "") \
                or session.title
        if record.get("type") == "fork-context-ref":
            session.forked_from_id = record.get("parentLastUuid")
            session.parent_id = record.get("parentSessionId")
            session.meta["fork_context_ref"] = record

    pending: dict[str, ToolCall] = {}
    for record in messages:
        body = record.get("message") or {}
        content = body.get("content")
        role = body.get("role")
        common = dict(source_id=record.get("uuid"),
                      parent_ids=[record["parentUuid"]]
                      if record.get("parentUuid") else [],
                      turn_id=record.get("promptId"),
                      agent_id=record.get("agentId") or agent_id,
                      created_at=record.get("timestamp"), raw=[record])
        if isinstance(content, str):
            session.messages.append(Message(
                role=role, blocks=[Block("text", content)], **common))
            continue

        blocks, result_carrier = [], False
        for item in content or []:
            kind = item.get("type")
            if kind == "text":
                blocks.append(Block("text", item.get("text", "")))
            elif kind == "thinking":
                text = visible_text(item.get("thinking"))
                if text is not None:
                    blocks.append(Block("text", text))
                    session.lose("thinking 降级为 text(丢弃 signature)")
                else:
                    session.lose("thinking 无可见正文,丢弃(含 signature)")
            elif kind == "tool_use":
                name = item.get("name", "")
                op = "agent.spawn" if name == "Agent" else TOOL_OPS.get(name)
                source_input = item.get("input") or {}
                tool = ToolCall(
                    name=name, op=op,
                    input=_norm_input(name, source_input) if op else source_input,
                    output="", source_call_id=item.get("id"),
                    meta={"claude_input": source_input})
                pending[item.get("id")] = tool
                blocks.append(Block("tool", tool=tool))
            elif kind == "tool_result":
                result_carrier = True
                tool = pending.pop(item.get("tool_use_id"), None)
                if tool is None:
                    session.lose(f"孤儿 tool_result: {item.get('tool_use_id')}")
                    continue
                tool.output = _result_text(item)
                tool.source_result_id = record.get("uuid")
                result = record.get("toolUseResult")
                if isinstance(result, dict):
                    tool.meta.update(result)
                    tool.status = result.get("status")
            else:
                session.lose(f"未知内容块类型丢弃: {kind}")
        if result_carrier and not any(
                block.kind == "text" and block.text.strip()
                for block in blocks):
            continue
        if blocks:
            session.messages.append(Message(role=role, blocks=blocks,
                                            **common))
    for tool in pending.values():
        session.lose(f"未配对 tool_use({tool.name})按无输出处理")
    return session


def _spawns(session: Session) -> dict[str, dict]:
    calls = {}
    for message in session.messages:
        for block in message.blocks:
            tool = block.tool if block.kind == "tool" else None
            if tool and tool.name == "Agent":
                calls[tool.source_call_id] = {
                    "tool": tool, "message_id": message.source_id}

    spawns = {}
    for raw in session.raw_records:
        record = raw.payload
        result = record.get("toolUseResult")
        if not isinstance(result, dict) or not result.get("agentId"):
            continue
        content = (record.get("message") or {}).get("content") or []
        result_block = next((item for item in content
                             if isinstance(item, dict) and
                             item.get("type") == "tool_result"), {})
        call_id = result_block.get("tool_use_id")
        info = calls.get(call_id, {})
        spawns[result["agentId"]] = {
            **info, "call_id": call_id, "result_id": record.get("uuid"),
            "result": result,
        }
        if info.get("tool"):
            info["tool"].meta.update(result)
    return spawns


def read(path: str) -> Session:
    main_path = Path(path)
    root = _read_transcript(main_path)
    root.root_id = root.source_id
    child_dir = main_path.with_suffix("") / "subagents"
    child_paths = sorted(child_dir.rglob("agent-*.jsonl")) \
        if child_dir.exists() else []
    journals = {}
    if child_dir.exists():
        for journal in child_dir.rglob("journal.jsonl"):
            records = _load(journal)
            relative = str(journal.relative_to(main_path.with_suffix("")))
            journals[relative] = records
            root.raw_records.extend(
                RawRecord(source="claude",
                          record_type=f"workflow.{record.get('type', 'unknown')}",
                          payload=record, ordinal=index,
                          timestamp=record.get("timestamp"), location=str(journal))
                for index, record in enumerate(records))
    if journals:
        root.meta["workflow_journals"] = journals
    sessions = [_read_transcript(child_path, is_child=True)
                for child_path in child_paths]
    by_agent = {session.agent_id: session for session in sessions
                if session.agent_id}
    containers = [root, *sessions]
    spawn_by_parent = {id(session): _spawns(session) for session in containers}

    assigned = set()
    for parent in containers:
        for agent_id, spawn in spawn_by_parent[id(parent)].items():
            child = by_agent.get(agent_id)
            if child is None or child is parent or id(child) in assigned:
                continue
            child.parent_id = parent.source_id
            child.root_id = root.source_id
            child.agent_path = str(Path(child.raw_records[0].location)
                                   .relative_to(main_path.parent))
            tool = spawn.get("tool")
            tool_input = tool.input if tool and isinstance(tool.input, dict) else {}
            result = spawn["result"]
            edge = AgentEdge(
                parent_session_id=parent.source_id,
                child_session_id=child.source_id,
                source_call_id=spawn.get("call_id"),
                spawn_message_id=spawn.get("message_id"),
                result_message_id=spawn.get("result_id"), agent_id=agent_id,
                agent_path=child.agent_path,
                agent_type=tool_input.get("subagent_type"),
                prompt=tool_input.get("prompt", ""), status=result.get("status"),
                meta={"toolUseResult": result})
            parent.children.append(child)
            parent.agent_edges.append(edge)
            assigned.add(id(child))

    for child in sessions:
        if id(child) in assigned:
            continue
        child.parent_id = root.source_id
        child.root_id = root.source_id
        child.agent_path = str(Path(child.raw_records[0].location)
                               .relative_to(main_path.parent))
        root.children.append(child)
        root.agent_edges.append(AgentEdge(
            parent_session_id=root.source_id,
            child_session_id=child.source_id, agent_id=child.agent_id,
            agent_path=child.agent_path,
            meta={"association": "directory-fallback"}))
        root.lose(f"子代理 {child.agent_id} 未找到 Agent 启动记录,按根子节点保留")
    return root
