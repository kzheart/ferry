"""Claude Code reader: JSONL 会话文件 -> 规范化会话树。"""
import json
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
from ...domain.tool_ops import CanonicalOp
from ..base.media import image_from_base64

TOOL_OPS = {
    "Bash": CanonicalOp.SHELL_EXEC,
    "Read": CanonicalOp.FS_READ,
    "Write": CanonicalOp.FS_WRITE,
    "Edit": CanonicalOp.FS_EDIT,
    "Grep": CanonicalOp.FS_SEARCH,
    "Glob": CanonicalOp.FS_GLOB,
    "WebFetch": CanonicalOp.WEB_FETCH,
    "WebSearch": CanonicalOp.WEB_SEARCH,
}


def _norm_input(name: str, inp: dict | str) -> dict | str:
    if not isinstance(inp, dict):
        return inp
    if name == "Edit":
        value = {"file_path": inp.get("file_path", ""),
                 "old": inp.get("old_string", ""),
                 "new": inp.get("new_string", "")}
        if "replace_all" in inp:
            value["replace_all"] = inp["replace_all"]
        return value
    if name == "Read":
        value = {"file_path": inp.get("file_path", "")}
        for field in ("offset", "limit"):
            if field in inp:
                value[field] = inp[field]
        return value
    if name == "Write":
        return {"file_path": inp.get("file_path", ""),
                "content": inp.get("content", "")}
    if name == "Bash":
        value = {"command": inp.get("command", "")}
        if "timeout" in inp:
            value["timeout_ms"] = inp["timeout"]
        if "run_in_background" in inp:
            value["background"] = inp["run_in_background"]
        if "dangerouslyDisableSandbox" in inp:
            value["sandbox_policy"] = (
                "dangerously-disable" if inp["dangerouslyDisableSandbox"]
                else "default")
        return value
    if name == "Agent":
        value = {
            "description": inp.get("description", ""),
            "prompt": inp.get("prompt", ""),
            "subagent_type": inp.get("subagent_type", ""),
        }
        aliases = {
            "name": "task_name",
            "model": "model",
            "mode": "fork_mode",
            "reasoning_effort": "reasoning_effort",
        }
        for source, target in aliases.items():
            if source in inp:
                value[target] = inp[source]
        return value
    if name == "Grep":
        value = {"query": inp.get("pattern", "")}
        aliases = {"path": "path", "glob": "glob", "head_limit": "max_results"}
        for source, target in aliases.items():
            if source in inp:
                value[target] = inp[source]
        return value
    if name == "Glob":
        value = {"pattern": inp.get("pattern", "")}
        if "path" in inp:
            value["path"] = inp["path"]
        return value
    if name == "WebFetch":
        value = {"url": inp.get("url", "")}
        if "prompt" in inp:
            value["prompt"] = inp["prompt"]
        return value
    if name == "WebSearch":
        value = {"query": inp.get("query", "")}
        if "allowed_domains" in inp:
            value["domains"] = inp["allowed_domains"]
        return value
    return inp


def _result_status(block: dict, native: dict) -> str:
    if block.get("is_error") is True or native.get("success") is False:
        return "error"
    if native.get("interrupted") is True:
        return "interrupted"
    if native.get("status") == "async_launched":
        return "running"
    if native.get("status") == "teammate_spawned":
        return "success"
    status = normalize_tool_result_status(native.get("status"))
    if "status" in native:
        return status
    return "success"


def _result_blocks(content) -> list[ToolResultBlock]:
    if isinstance(content, str):
        return [ToolResultBlock("text", text=content)] if content else []
    if isinstance(content, dict):
        return [ToolResultBlock(
            "json", data=content,
            metadata={"native_type": content.get("type")},
        )]
    if not isinstance(content, list):
        return [] if content is None else [ToolResultBlock("json", data=content)]

    blocks = []
    for item in content:
        if not isinstance(item, dict):
            blocks.append(ToolResultBlock("json", data=item))
            continue
        kind = item.get("type")
        if kind == "text":
            blocks.append(ToolResultBlock("text", text=item.get("text", "")))
        elif kind == "image":
            source = item.get("source") or {}
            blocks.append(ToolResultBlock(
                "image",
                data=source.get("data"),
                mime_type=source.get("media_type"),
                metadata={
                    key: value for key, value in source.items()
                    if key not in {"data", "media_type"}
                },
            ))
        elif kind == "tool_reference":
            blocks.append(ToolResultBlock(
                "tool_reference",
                metadata={
                    key: value for key, value in item.items()
                    if key != "type"
                },
            ))
        else:
            blocks.append(ToolResultBlock(
                "json", data=item, metadata={"native_type": kind},
            ))
    return blocks


def _tool_result(block: dict, native=None) -> ToolResult:
    native = native if isinstance(native, dict) else {}
    canonical = native.get("canonicalToolResult")
    canonical = canonical if isinstance(canonical, dict) else {}
    exit_code = native.get("exit_code")
    if isinstance(exit_code, bool) or not isinstance(exit_code, int):
        exit_code = None
    truncated = native.get("truncated")
    if not isinstance(truncated, bool):
        truncated = None
    stdout = native.get("stdout")
    stderr = native.get("stderr")
    metadata = dict(canonical.get("metadata") or {})
    metadata["claude_native_result"] = {
        key: value for key, value in native.items()
        if key != "canonicalToolResult"
    }
    metadata["claude_tool_result"] = {
        key: value for key, value in block.items() if key != "content"
    }
    attachments = canonical.get("attachments")
    if not isinstance(attachments, list):
        attachments = []
    blocks = _result_blocks(block.get("content"))
    canonical_blocks = canonical.get("blocks")
    if isinstance(canonical_blocks, list):
        blocks = []
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
    return ToolResult(
        status=canonical.get("status") or _result_status(block, native),
        blocks=blocks,
        stdout=stdout if isinstance(stdout, str) else None,
        stderr=stderr if isinstance(stderr, str) else None,
        exit_code=exit_code,
        truncated=truncated,
        attachments=attachments,
        metadata=metadata,
    )


def _load(path: Path) -> list[dict]:
    records = []
    for line_number, line in enumerate(path.read_text().splitlines(), start=1):
        if not line.strip():
            continue
        try:
            records.append(json.loads(line))
        except json.JSONDecodeError as error:
            records.append({
                "type": "__resume_harness_malformed_jsonl__",
                "line_number": line_number,
                "error": error.msg,
            })
    return records


def _native_agent_id(value: dict) -> str | None:
    agent_id = value.get("agentId")
    return agent_id if isinstance(agent_id, str) and agent_id else None


def _agent_id(lines: list[dict], path: Path) -> str | None:
    for record in lines:
        agent_id = _native_agent_id(record)
        if agent_id:
            return agent_id
    name = path.stem
    return name[6:] if name.startswith("agent-") else None


def _compact_summary_text(record: dict) -> str:
    content = (record.get("message") or {}).get("content")
    if isinstance(content, str):
        return content
    if not isinstance(content, list):
        return ""
    return "\n".join(
        str(item.get("text") or "") for item in content
        if isinstance(item, dict) and item.get("type") == "text"
        and item.get("text")
    )


def _context_compactions(lines: list[dict]) -> list[ContextCompaction]:
    compactions = []
    by_boundary = {}
    for index, record in enumerate(lines):
        if record.get("type") != "system" or \
                record.get("subtype") != "compact_boundary":
            continue
        metadata = record.get("compactMetadata") or {}
        boundary_id = record.get("uuid") or f"compact-boundary:{index}"
        pre_tokens = metadata.get("preTokens")
        post_tokens = metadata.get("postTokens")
        metrics = {
            "pre_tokens": pre_tokens,
            "post_tokens": post_tokens,
            "cumulative_dropped_tokens": metadata.get(
                "cumulativeDroppedTokens"),
            "duration_ms": metadata.get("durationMs"),
        }
        if isinstance(pre_tokens, int) and isinstance(post_tokens, int):
            metrics["dropped_tokens"] = max(0, pre_tokens - post_tokens)
        trigger = metadata.get("trigger")
        compaction = ContextCompaction(
            id=boundary_id,
            source="claude",
            after_message_id=record.get("logicalParentUuid"),
            event_locator=boundary_id,
            created_at=record.get("timestamp"),
            trigger=("automatic" if trigger == "auto"
                     else "manual" if trigger == "manual"
                     else "unknown"),
            state="incomplete",
            metrics={key: value for key, value in metrics.items()
                     if value is not None},
            source_meta={
                "preserved_segment": metadata.get("preservedSegment"),
                "preserved_messages": metadata.get("preservedMessages"),
            },
        )
        compactions.append(compaction)
        by_boundary[boundary_id] = compaction

    for index, record in enumerate(lines):
        if record.get("isCompactSummary") is not True:
            continue
        summary = _compact_summary_text(record)
        parent_id = record.get("parentUuid")
        compaction = by_boundary.get(parent_id)
        if compaction is None:
            compaction = ContextCompaction(
                id=parent_id or record.get("uuid") or f"compact-summary:{index}",
                source="claude",
                after_message_id=record.get("logicalParentUuid"),
                event_locator=parent_id,
                created_at=record.get("timestamp"),
                state="incomplete",
            )
            compactions.append(compaction)
        compaction.summary_message_id = record.get("uuid")
        compaction.summary_text = summary
        compaction.summary_status = "available" if summary else "missing"
        compaction.state = "completed" if summary else "incomplete"
    return compactions


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
    session.context_compactions = _context_compactions(lines)
    session.raw_records = [
        RawRecord(source="claude", record_type=record.get("type", ""),
                  payload=record, ordinal=index,
                  timestamp=record.get("timestamp"), location=str(path))
        for index, record in enumerate(lines)
    ]
    for record in lines:
        if record.get("type") == "__resume_harness_malformed_jsonl__":
            session.lose(
                "session.malformed_record",
                line_number=record["line_number"],
                error=record["error"],
            )

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
        if record.get("isMeta"):
            continue
        body = record.get("message") or {}
        content = body.get("content")
        role = body.get("role")
        common = dict(source_id=record.get("uuid"),
                      parent_ids=[record["parentUuid"]]
                      if record.get("parentUuid") else [],
                      turn_id=record.get("promptId"),
                      agent_id=_native_agent_id(record) or agent_id,
                      created_at=record.get("timestamp"), raw=[record])
        if isinstance(content, str):
            session.messages.append(Message(
                role=role, blocks=[Block("text", content)], **common))
            continue

        blocks, result_carrier = [], False
        for item_index, item in enumerate(content or []):
            kind = item.get("type")
            if kind == "text":
                blocks.append(Block("text", item.get("text", "")))
            elif kind == "image":
                source = item.get("source") or {}
                image = image_from_base64(
                    f"{record.get('uuid')}:image:{item_index}",
                    source.get("media_type", ""), source.get("data", ""))
                if image is None:
                    session.lose("migration.unknown_block_dropped", kind=kind)
                else:
                    blocks.append(Block("image", image=image))
            elif kind == "thinking":
                text = visible_text(item.get("thinking"))
                if text is not None:
                    blocks.append(Block("text", text))
                    session.lose("migration.reasoning_metadata_dropped", metadata_kind="signature")
                else:
                    session.lose("migration.reasoning_dropped", metadata_kind="signature")
            elif kind == "tool_use":
                name = item.get("name", "")
                op = CanonicalOp.AGENT_SPAWN if name == "Agent" else TOOL_OPS.get(name)
                source_input = item.get("input") or {}
                if op is None:
                    op = CanonicalOp.TOOL_INVOKE
                    canonical_input = {
                        "namespace": "claude",
                        "name": name,
                        "input": source_input,
                    }
                else:
                    canonical_input = _norm_input(name, source_input)
                tool = ToolCall(
                    name=name, op=op,
                    input=canonical_input,
                    source_call_id=item.get("id"),
                    meta={"claude_input": source_input})
                pending[item.get("id")] = tool
                blocks.append(Block("tool", tool=tool))
            elif kind == "tool_result":
                result_carrier = True
                tool = pending.pop(item.get("tool_use_id"), None)
                if tool is None:
                    session.lose("session.orphan_tool_result", call_id=item.get("tool_use_id"))
                    continue
                tool.source_result_id = record.get("uuid")
                result = record.get("toolUseResult")
                tool.result = _tool_result(item, result)
            else:
                session.lose("migration.unknown_block_dropped", kind=kind)
        if result_carrier and not any(
                block.kind == "text" and block.text.strip()
                for block in blocks):
            continue
        if blocks:
            session.messages.append(Message(role=role, blocks=blocks,
                                            **common))
    for tool in pending.values():
        session.lose("session.unpaired_tool_use", tool_name=tool.name)
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
        result_agent_id = (
            _native_agent_id(result) if isinstance(result, dict) else None
        )
        if not isinstance(result, dict) or not result_agent_id:
            continue
        content = (record.get("message") or {}).get("content") or []
        result_block = next((item for item in content
                             if isinstance(item, dict) and
                             item.get("type") == "tool_result"), {})
        call_id = result_block.get("tool_use_id")
        info = calls.get(call_id, {})
        spawns[result_agent_id] = {
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
                association="agent-id", confidence=1.0,
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
            association="directory-fallback", confidence=0.25,
            meta={"association": "directory-fallback"}))
        root.lose("session.subagent_unlinked", child_id=child.agent_id)
    return root
