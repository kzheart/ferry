"""OpenCode 当前原生结构到 Canonical Session 的读取转换。"""
from __future__ import annotations

import copy
import re
import sqlite3

from ...errors import AgentFormatChangedError, SessionNotFoundError
from ...sessions.model import (
    AgentEdge,
    Block,
    ContextCompaction,
    Message,
    Session,
    ToolCall,
    ToolResult,
    ToolResultBlock,
)
from ...sessions.reasoning import visible_text
from ...sessions.tool_ops import CanonicalOp
from ..shared.media import image_from_data_url
from . import store as native_store


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


def _patch_operations(patch: str) -> list[dict]:
    return [
        {"operation": operation.lower(), "path": path.strip()}
        for operation, path in re.findall(
            r"^\*\*\* (Add|Update|Delete) File: ([^\r\n]+)$",
            patch,
            re.MULTILINE,
        )
    ]


def _canonical_tool_input(name: str, source_input):
    inputs = (
        dict(source_input) if isinstance(source_input, dict) else source_input
    )
    if name == "task":
        return CanonicalOp.AGENT_SPAWN, {
            "description": str(
                inputs.get("description") or "migrated subagent"
            ),
            "prompt": str(inputs.get("prompt") or ""),
            "subagent_type": str(inputs.get("subagent_type") or "general"),
        }
    operation = TOOL_OPS.get(name)
    if operation is None:
        return CanonicalOp.TOOL_INVOKE, {
            "namespace": "opencode",
            "name": name,
            "input": inputs,
        }
    if not isinstance(inputs, dict):
        return operation, inputs
    if name == "bash":
        value = {"command": inputs.get("command", "")}
        if "workdir" in inputs:
            value["workdir"] = inputs["workdir"]
        if "timeout" in inputs:
            value["timeout_ms"] = inputs["timeout"]
        if "run_in_background" in inputs:
            value["background"] = inputs["run_in_background"]
        return operation, value
    if name == "read":
        value = {"file_path": inputs.get("filePath", "")}
        value.update(
            {
                key: inputs[key]
                for key in ("offset", "limit")
                if key in inputs
            }
        )
        return operation, value
    if name == "write":
        return operation, {
            "file_path": inputs.get("filePath", ""),
            "content": inputs.get("content", ""),
        }
    if name == "edit":
        value = {
            "file_path": inputs.get("filePath", ""),
            "old": inputs.get("oldString", ""),
            "new": inputs.get("newString", ""),
        }
        if "replaceAll" in inputs:
            value["replace_all"] = inputs["replaceAll"]
        return operation, value
    if name == "apply_patch":
        patch = str(inputs.get("patchText", ""))
        return operation, {
            "operations": _patch_operations(patch),
            "raw_patch": patch,
        }
    if name == "grep":
        value = {"query": inputs.get("pattern", "")}
        if "path" in inputs:
            value["path"] = inputs["path"]
        if "include" in inputs:
            value["glob"] = inputs["include"]
        if "limit" in inputs:
            value["max_results"] = inputs["limit"]
        return operation, value
    if name == "glob":
        value = {"pattern": inputs.get("pattern", "")}
        if "path" in inputs:
            value["path"] = inputs["path"]
        return operation, value
    if name == "webfetch":
        value = {"url": inputs.get("url", "")}
        if "format" in inputs:
            value["format"] = inputs["format"]
        if "timeout" in inputs:
            value["timeout_ms"] = inputs["timeout"]
        return operation, value
    if name == "websearch":
        value = {"query": inputs.get("query", "")}
        if "numResults" in inputs:
            value["num_results"] = inputs["numResults"]
        return operation, value
    return operation, inputs


def _tool_result(state: dict) -> ToolResult:
    metadata = copy.deepcopy(state.get("metadata") or {})
    native_state = {
        key: copy.deepcopy(value)
        for key, value in state.items()
        if key
        not in {
            "input",
            "output",
            "error",
            "metadata",
            "attachments",
            "status",
            "time",
        }
    }
    if native_state:
        metadata["opencode_state"] = native_state
    status = {
        "completed": "success",
        "error": "error",
        "running": "running",
        "pending": "pending",
    }.get(state.get("status"), "unknown")
    if metadata.get("interrupted") is True:
        status = "interrupted"

    output = state.get("output")
    blocks = []
    if isinstance(output, str):
        if output:
            blocks.append(ToolResultBlock("text", text=output))
    elif output is not None:
        blocks.append(ToolResultBlock("json", data=copy.deepcopy(output)))

    error = state.get("error")
    if isinstance(error, str) and error:
        if not any(
            block.kind == "text" and block.text == error for block in blocks
        ):
            blocks.append(ToolResultBlock("text", text=error))
        if status == "unknown":
            status = "error"

    attachments = copy.deepcopy(state.get("attachments") or [])
    if not isinstance(attachments, list):
        attachments = [attachments]
    for attachment in attachments:
        if not isinstance(attachment, dict):
            blocks.append(ToolResultBlock("json", data=attachment))
        elif attachment.get("type") == "file":
            blocks.append(
                ToolResultBlock(
                    "file",
                    mime_type=attachment.get("mime"),
                    filename=attachment.get("filename"),
                    uri=attachment.get("url"),
                )
            )
        else:
            blocks.append(ToolResultBlock("json", data=attachment))

    exit_code = metadata.get("exit")
    if isinstance(exit_code, bool) or not isinstance(exit_code, int):
        exit_code = None
    truncated = metadata.get("truncated")
    if not isinstance(truncated, bool):
        truncated = None
    stdout = metadata.get("stdout")
    stderr = error if isinstance(error, str) else metadata.get("stderr")
    return ToolResult(
        status=status,
        blocks=blocks,
        stdout=stdout if isinstance(stdout, str) else None,
        stderr=stderr if isinstance(stderr, str) else None,
        exit_code=exit_code,
        truncated=truncated,
        attachments=attachments,
    )


def _message_model(data: dict) -> tuple[str | None, str | None]:
    for message in data.get("messages", []):
        info = message.get("info") or {}
        model = info.get("model")
        if isinstance(model, dict):
            provider_id = model.get("providerID")
            model_id = model.get("modelID")
        else:
            provider_id = info.get("providerID")
            model_id = info.get("modelID")
        provider_id = (
            provider_id
            if isinstance(provider_id, str) and provider_id
            else None
        )
        model_id = model_id if isinstance(model_id, str) and model_id else None
        if provider_id is not None or model_id is not None:
            return provider_id, model_id
    return None, None


def parse_session(data: dict) -> tuple[Session, list[AgentEdge]]:
    info = data["info"]
    model_provider, model = _message_model(data)
    session = Session(
        source_tool="opencode",
        source_id=info["id"],
        cwd=info.get("directory", ""),
        title=info.get("title", ""),
        parent_id=info.get("parentID"),
        agent_id=info.get("agent"),
        model_provider=model_provider,
        model=model,
    )
    edges = []
    pending_compactions = []
    last_visible_message_id = None
    raw_message_indexes = {
        (message.get("info") or {}).get("id"): index
        for index, message in enumerate(data.get("messages", []), start=1)
    }
    for ordinal, native_message in enumerate(data.get("messages", [])):
        info = native_message["info"]
        role = info.get("role", "user")
        blocks = []
        message_id = info.get("id")
        compaction_part = next(
            (
                part
                for part in native_message.get("parts", [])
                if part.get("type") == "compaction"
            ),
            None,
        )
        if compaction_part is not None:
            tail_locator = compaction_part.get("tail_start_id")
            tail_index = raw_message_indexes.get(tail_locator)
            compaction = ContextCompaction(
                id=message_id or f"compaction:{ordinal}",
                source="opencode",
                after_message_id=last_visible_message_id,
                event_locator=message_id,
                created_at=(info.get("time") or {}).get("created"),
                trigger=(
                    "automatic"
                    if compaction_part.get("auto") is True
                    else (
                        "manual"
                        if compaction_part.get("auto") is False
                        else "unknown"
                    )
                ),
                state="incomplete",
                tail_status="located" if tail_index is not None else "unknown",
                tail_start_locator=tail_locator,
                tail_start_message_index=tail_index,
                source_meta={
                    key: copy.deepcopy(value)
                    for key, value in compaction_part.items()
                    if key not in {"type", "tail_start_id"}
                },
            )
            session.context_compactions.append(compaction)
            pending_compactions.append(compaction)
        is_summary = (
            info.get("mode") == "compaction" or info.get("summary") is True
        )
        if is_summary:
            summary = "\n".join(
                str(part.get("text") or "")
                for part in native_message.get("parts", [])
                if part.get("type") == "text" and part.get("text")
            )
            compaction = next(
                (
                    item
                    for item in reversed(pending_compactions)
                    if item.summary_message_id is None
                ),
                None,
            )
            if compaction is not None:
                compaction.summary_message_id = message_id
                compaction.summary_text = summary
                compaction.summary_status = (
                    "available" if summary else "missing"
                )
                compaction.state = "completed" if summary else "incomplete"
        for part_ordinal, part in enumerate(
            native_message.get("parts", [])
        ):
            part_type = part.get("type")
            if part_type == "text":
                blocks.append(Block("text", part.get("text", "")))
            elif part_type == "file" and str(
                part.get("mime", "")
            ).startswith("image/"):
                image = image_from_data_url(
                    f"{message_id}:image:{part_ordinal}",
                    part.get("url", ""),
                    part.get("filename"),
                )
                if image is None:
                    session.lose(
                        "migration.unknown_block_dropped", kind="file"
                    )
                else:
                    blocks.append(Block("image", image=image))
            elif part_type == "reasoning":
                text = visible_text(part.get("text"))
                if text is not None:
                    blocks.append(Block("text", text))
                    session.lose(
                        "migration.reasoning_metadata_dropped",
                        metadata_kind="metadata",
                    )
                else:
                    session.lose(
                        "migration.reasoning_dropped",
                        metadata_kind="metadata",
                    )
            elif part_type == "tool":
                state = part.get("state", {})
                operation, inputs = _canonical_tool_input(
                    part.get("tool", "?"), state.get("input") or {}
                )
                blocks.append(
                    Block(
                        "tool",
                        tool=ToolCall(
                            name=part.get("tool", "?"),
                            op=operation,
                            input=inputs,
                            source_call_id=part.get("callID"),
                            started_at=(state.get("time") or {}).get("start"),
                            ended_at=(state.get("time") or {}).get("end"),
                            result=_tool_result(state),
                        ),
                    )
                )
                metadata = state.get("metadata") or {}
                child_id = metadata.get("sessionId")
                if part.get("tool") == "task" and child_id:
                    edges.append(
                        AgentEdge(
                            parent_session_id=data["info"]["id"],
                            child_session_id=child_id,
                            source_call_id=part.get("callID"),
                            spawn_message_id=message_id,
                            agent_id=inputs.get("subagent_type"),
                            agent_type=inputs.get("subagent_type"),
                            prompt=inputs.get("prompt", ""),
                            status=state.get("status"),
                            association="task-metadata",
                            confidence=1.0,
                        )
                    )
            elif part_type not in ("step-start", "step-finish"):
                session.lose(
                    "migration.unknown_block_dropped", kind=part_type
                )
        if blocks:
            parent_id = info.get("parentID")
            session.messages.append(
                Message(
                    role=role,
                    blocks=blocks,
                    source_id=message_id,
                    parent_ids=[parent_id] if parent_id else [],
                    agent_id=info.get("agent"),
                    created_at=(info.get("time") or {}).get("created"),
                )
            )
            if not is_summary:
                last_visible_message_id = message_id
    compacting = (data["info"].get("time") or {}).get("compacting")
    if compacting and not any(
        compaction.state == "in_progress"
        for compaction in session.context_compactions
    ):
        session.context_compactions.append(
            ContextCompaction(
                id=f"{data['info']['id']}:compacting",
                source="opencode",
                after_message_id=last_visible_message_id,
                created_at=compacting,
                state="in_progress",
            )
        )
    return session, edges


def _read(session_id: str) -> Session:
    seen: dict[str, Session] = {}
    connection = native_store.open_database()

    def export(identifier: str) -> dict:
        data = native_store.export_from_database(connection, identifier)
        if data is None:
            raise SessionNotFoundError("opencode", identifier)
        return data

    def child_ids_of(identifier: str) -> list[str]:
        try:
            return [
                row[0]
                for row in connection.execute(
                    "SELECT id FROM session WHERE parent_id = ? "
                    "ORDER BY time_created, id",
                    (identifier,),
                )
            ]
        except sqlite3.Error as error:
            raise AgentFormatChangedError(
                "opencode",
                "sqlite.session.parent_id",
                "queryable parent-child relation",
                str(error),
            ) from error

    def visit(identifier: str, root_id: str) -> Session:
        if identifier in seen:
            return seen[identifier]
        session, task_edges = parse_session(export(identifier))
        seen[identifier] = session
        session.root_id = root_id

        task_by_child = {}
        for edge in task_edges:
            task_by_child.setdefault(edge.child_session_id, []).append(edge)
        child_ids = child_ids_of(identifier)
        database_child_ids = set(child_ids)
        for child_id in task_by_child:
            if child_id not in child_ids:
                child_ids.append(child_id)

        for child_id in child_ids:
            if child_id in seen:
                continue
            child = visit(child_id, root_id)
            if (
                child_id not in database_child_ids
                and child.parent_id != identifier
            ):
                session.lose(
                    "session.child_foreign_ignored", child_id=child_id
                )
                continue
            if child.parent_id and child.parent_id != identifier:
                session.lose(
                    "session.child_parent_conflict", child_id=child_id
                )
                continue
            child.parent_id = identifier
            session.children.append(child)
            edges = task_by_child.get(child_id) or [
                AgentEdge(
                    parent_session_id=identifier,
                    child_session_id=child_id,
                    association="sqlite-parent",
                    confidence=0.9,
                )
            ]
            for edge in edges:
                edge.agent_id = edge.agent_id or child.agent_id
                edge.agent_path = edge.agent_path or child.agent_path
                edge.agent_type = edge.agent_type or child.agent_type
                session.agent_edges.append(edge)
        return session

    try:
        return visit(session_id, session_id)
    finally:
        connection.close()


def read(session_id: str) -> Session:
    return _read(session_id)


def read_preview(session_id: str) -> Session:
    return _read(session_id)
