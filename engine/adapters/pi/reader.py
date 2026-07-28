"""Pi v3 active-branch projection to the canonical session model."""
from __future__ import annotations

import json
from pathlib import Path

from ...errors import AgentFormatChangedError, SessionNotFoundError
from ...sessions.model import (
    Block, ContextCompaction, Message, Session, ToolCall, ToolResult,
    ToolResultBlock,
)
from ...sessions.tool_ops import CanonicalOp
from ..shared.media import image_from_base64
from ..shared.scanner import split_jsonl_lines
from .tool_calls import call_from_part, result_from_message

def _load(path: Path) -> tuple[dict, list[dict], list[int]]:
    try:
        lines = split_jsonl_lines(path.read_text())
    except OSError as error:
        raise SessionNotFoundError("pi", str(path)) from error
    records, malformed = [], []
    nonempty = [index for index, line in enumerate(lines) if line.strip()]
    final = nonempty[-1] if nonempty else -1
    for index, line in enumerate(lines):
        if not line.strip():
            continue
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            if index != final:
                malformed.append(index + 1)
            continue
        if isinstance(value, dict):
            records.append(value)
    if not records:
        raise AgentFormatChangedError("pi", "header", "Pi v3 session", None)
    header = records.pop(0)
    if (header.get("type") != "session" or header.get("version") != 3
            or not all(isinstance(header.get(key), str) and header[key]
                       for key in ("id", "timestamp", "cwd"))):
        raise AgentFormatChangedError(
            "pi", "header", {"type": "session", "version": 3}, header,
        )
    return header, records, malformed


def _active_branch(entries: list[dict]) -> tuple[list[dict], set[str]]:
    valid = [entry for entry in entries
             if isinstance(entry.get("id"), str) and "parentId" in entry]
    if not valid:
        return [], set()
    by_id = {entry["id"]: entry for entry in valid}
    branch, seen = [], set()
    current = valid[-1]
    while current and current["id"] not in seen:
        branch.append(current)
        seen.add(current["id"])
        parent_id = current.get("parentId")
        current = by_id.get(parent_id) if parent_id is not None else None
    branch.reverse()
    return branch, seen


def _content_blocks(content, source_id: str, session: Session, calls=None) -> list[Block]:
    parts = [{"type": "text", "text": content}] if isinstance(content, str) else content
    blocks = []
    for index, part in enumerate(parts if isinstance(parts, list) else []):
        if not isinstance(part, dict):
            continue
        kind = part.get("type")
        if kind == "text":
            blocks.append(Block("text", str(part.get("text") or "")))
        elif kind == "thinking":
            blocks.append(Block("thinking", str(part.get("thinking") or "")))
        elif kind == "toolCall" and calls is not None:
            call = call_from_part(part, source_id)
            blocks.append(Block("tool", tool=call))
            if call.source_call_id:
                calls[call.source_call_id] = call
        elif kind == "image":
            asset = image_from_base64(
                f"pi:{source_id}:{index}",
                str(part.get("mimeType") or ""), part.get("data"),
            )
            if asset is None:
                session.lose(
                    "migration.unknown_block_dropped",
                    source="pi", entry_id=source_id, block_type="image",
                    index=index,
                )
            else:
                blocks.append(Block("image", image=asset))
    return blocks


def read(path: str) -> Session:
    header, entries, malformed = _load(Path(path))
    branch, selected = _active_branch(entries)
    session = Session(
        "pi", header["id"], header["cwd"],
    )
    for line in malformed:
        session.lose("session.malformed_record", line=line)
    unselected = [entry["id"] for entry in entries
                  if isinstance(entry.get("id"), str) and entry["id"] not in selected]
    if unselected:
        session.lose(
            "migration.unknown_block_dropped",
            source="pi", block_type="inactive_branch",
            entry_ids=unselected,
        )

    calls = {}
    last_message_id = None
    for entry in branch:
        kind, entry_id = entry.get("type"), entry["id"]
        if kind == "session_info":
            if isinstance(entry.get("name"), str):
                session.title = entry["name"]
            continue
        if kind == "model_change":
            session.model_provider = entry.get("provider")
            session.model = entry.get("modelId")
            continue
        if kind == "branch_summary":
            summary = str(entry.get("summary") or "")
            session.messages.append(Message(
                "user", [Block("text", summary)], source_id=entry_id,
                parent_ids=[entry["parentId"]] if entry.get("parentId") else [],
                created_at=entry.get("timestamp"),
            ))
            last_message_id = entry_id
            continue
        if kind == "compaction":
            session.context_compactions.append(ContextCompaction(
                id=entry_id, source="pi",
                after_message_id=last_message_id,
                event_locator=entry_id, created_at=entry.get("timestamp"),
                summary_status="available" if entry.get("summary") else "missing",
                summary_text=str(entry.get("summary") or ""),
                tail_status=(
                    "located"
                    if entry.get("firstKeptEntryId") in selected
                    else "unknown"
                ),
                tail_start_locator=entry.get("firstKeptEntryId"),
                tail_start_message_index=next((
                    index for index, message in enumerate(session.messages, 1)
                    if message.source_id == entry.get("firstKeptEntryId")
                ), None),
                metrics=({"tokens_before": entry["tokensBefore"]}
                         if entry.get("tokensBefore") is not None else {}),
                source_meta={"from_hook": bool(entry.get("fromHook"))},
            ))
            continue
        if kind != "message":
            if kind in {"thinking_level_change", "label"}:
                continue
            session.lose(
                "migration.unknown_block_dropped",
                source="pi", entry_id=entry_id, block_type=kind,
            )
            continue

        message = entry.get("message") or {}
        role = message.get("role")
        if role == "bashExecution":
            output = str(message.get("output") or "")
            call_id = f"bash:{entry_id}"
            result = ToolResult(
                status=("interrupted" if message.get("cancelled")
                        else "error" if message.get("exitCode") not in (None, 0)
                        else "success"),
                blocks=[ToolResultBlock("text", text=output)] if output else [],
                stdout=output,
                exit_code=(message.get("exitCode")
                           if isinstance(message.get("exitCode"), int)
                           and not isinstance(message.get("exitCode"), bool)
                           else None),
                truncated=(message.get("truncated")
                           if isinstance(message.get("truncated"), bool)
                           else None),
                attachments=([{"full_output_path": message["fullOutputPath"]}]
                             if message.get("fullOutputPath") else []),
            )
            call = ToolCall(
                name="bash", op=CanonicalOp.SHELL_EXEC,
                input={"command": str(message.get("command") or "")},
                result=result, source_call_id=call_id,
                source_result_id=entry_id, source_message_id=entry_id,
            )
            session.messages.append(Message(
                "user", [Block("tool", tool=call)], source_id=entry_id,
                parent_ids=[entry["parentId"]] if entry.get("parentId") else [],
                created_at=entry.get("timestamp"),
            ))
            last_message_id = entry_id
            continue
        if role == "toolResult":
            call = calls.get(message.get("toolCallId"))
            if call is None:
                session.lose("session.orphan_tool_result",
                             tool_call_id=message.get("toolCallId"))
            else:
                call.result = result_from_message(message)
                call.source_result_id = entry_id
            continue
        if role not in {"user", "assistant"}:
            session.lose(
                "migration.unknown_block_dropped",
                source="pi", entry_id=entry_id,
                block_type=f"message.{role}",
            )
            continue

        blocks = _content_blocks(
            message.get("content"), entry_id, session,
            calls if role == "assistant" else None,
        )
        if role == "assistant":
            session.model_provider = message.get("provider") or session.model_provider
            session.model = message.get("model") or session.model
        elif not session.title:
            text = " ".join(block.text for block in blocks if block.kind == "text").strip()
            session.title = text[:80] + ("…" if len(text) > 80 else "")
        session.messages.append(Message(
            role, blocks, source_id=entry_id,
            parent_ids=[entry["parentId"]] if entry.get("parentId") else [],
            created_at=entry.get("timestamp"),
        ))
        last_message_id = entry_id
    for call_id, call in calls.items():
        if call.result is None:
            session.lose("session.unpaired_tool_use", tool_call_id=call_id)
    return session
