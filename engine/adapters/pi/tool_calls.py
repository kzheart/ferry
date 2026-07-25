"""Pi tool-call normalization and result pairing."""
from __future__ import annotations

import copy

from ...sessions.model import ToolCall, ToolResult, ToolResultBlock
from ...sessions.tool_ops import CanonicalOp


TOOL_OPS = {
    "bash": CanonicalOp.SHELL_EXEC,
    "read": CanonicalOp.FS_READ,
    "write": CanonicalOp.FS_WRITE,
    "edit": CanonicalOp.FS_EDIT,
    "grep": CanonicalOp.FS_SEARCH,
    "find": CanonicalOp.FS_GLOB,
    "web_fetch": CanonicalOp.WEB_FETCH,
    "web_search": CanonicalOp.WEB_SEARCH,
}


def normalize_input(name: str, value):
    source = copy.deepcopy(value)
    if not isinstance(source, dict):
        return CanonicalOp.TOOL_INVOKE, {
            "namespace": "pi", "name": name, "input": source,
        }
    op = TOOL_OPS.get(name)
    if op is None:
        return CanonicalOp.TOOL_INVOKE, {
            "namespace": "pi", "name": name, "input": source,
        }
    if name == "bash":
        return op, {key: source[native] for key, native in (
            ("command", "command"), ("workdir", "cwd"), ("timeout_ms", "timeout"),
        ) if native in source}
    if name in {"read", "write", "edit"}:
        path = source.get("path", source.get("file_path", ""))
        mapped = {"file_path": path}
        aliases = {
            "content": "content", "old": "oldText", "new": "newText",
            "offset": "offset", "limit": "limit",
        }
        mapped.update({key: source[native] for key, native in aliases.items()
                       if native in source})
        return op, mapped
    if name == "grep":
        return op, {"query": source.get("pattern", ""),
                    **({"path": source["path"]} if "path" in source else {})}
    if name == "find":
        return op, {"pattern": source.get("pattern", ""),
                    **({"path": source["path"]} if "path" in source else {})}
    return op, source


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
