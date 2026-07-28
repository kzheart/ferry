"""Pi tool-call normalization and result pairing."""
from __future__ import annotations

import copy

from ...sessions.model import ToolCall, ToolResult, ToolResultBlock
from ...sessions.tool_ops import CanonicalOp
from .dialect import DIALECT


def normalize_input(name: str, value):
    source = copy.deepcopy(value)
    parsed = DIALECT.parse(name, source)
    if parsed is None:
        return CanonicalOp.TOOL_INVOKE, {
            "namespace": "pi", "name": name, "input": source,
        }
    return parsed


def call_from_part(part: dict, message_id: str) -> ToolCall:
    name = str(part.get("name") or "")
    op, value = normalize_input(name, part.get("arguments") or {})
    return ToolCall(
        name=name, op=op, input=value,
        source_call_id=part.get("id"), source_message_id=message_id,
    )


def result_from_message(message: dict) -> ToolResult:
    blocks = []
    for part in message.get("content") or []:
        if part.get("type") == "text":
            blocks.append(ToolResultBlock("text", text=str(part.get("text") or "")))
        elif part.get("type") == "image":
            blocks.append(ToolResultBlock(
                "image", data=part.get("data"), mime_type=part.get("mimeType"),
            ))
        else:
            blocks.append(ToolResultBlock("json", data=copy.deepcopy(part)))
    details = copy.deepcopy(message.get("details"))
    attachments = [] if details is None else (
        details if isinstance(details, list) else [details]
    )
    return ToolResult(
        status="error" if message.get("isError") else "success",
        blocks=blocks,
        attachments=attachments,
    )
