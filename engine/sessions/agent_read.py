"""供 Ferry Agent 使用的限量、脱敏会话读取。"""
from __future__ import annotations

import json
from pathlib import Path

from ..errors import AgentReferenceError, AgentRequestError
from .index import AgentSessionIndex, IndexedSession
from .model import tool_result_text
from .safety import (
    MAX_AGENT_DTO_BYTES,
    bounded_int,
    finalize_dto,
    record_session_id,
    redact,
    safe_project,
    string_set,
)

MAX_CONTENT_SEARCH_RESULTS = 50
MAX_CONTEXT_MESSAGES = 50
MAX_CONTEXT_BYTES = 64 * 1024
DEFAULT_CONTEXT_BYTES = 24 * 1024


def _take(text: str, remaining: int) -> tuple[str, int, bool]:
    encoded = text.encode("utf-8")
    if len(encoded) <= remaining:
        return text, remaining - len(encoded), False
    clipped = encoded[:max(0, remaining)].decode("utf-8", errors="ignore")
    return clipped, 0, True


def _validate_read_scope(record: IndexedSession) -> None:
    if not record.path_backed:
        return
    path = Path(record.canonical_ref)
    root = Path(record.root or "").resolve(strict=True)
    if record.tool == "claude":
        child_root = path.with_suffix("") / "subagents"
        candidates = child_root.rglob("*.jsonl") if child_root.exists() else ()
    elif record.tool == "codex":
        candidates = root.rglob("rollout*.jsonl")
    else:
        candidates = ()
    for candidate in candidates:
        try:
            resolved = candidate.resolve(strict=True)
        except OSError as error:
            raise AgentReferenceError("会话子树包含失效文件") from error
        if not resolved.is_file() or not resolved.is_relative_to(root):
            raise AgentReferenceError("会话子树超出 Agent 会话根目录")


def read_indexed_session(index: AgentSessionIndex, record: IndexedSession):
    _validate_read_scope(record)
    browser = index.ports.adapter(record.tool).browser
    session = getattr(browser, "read_agent", browser.read)(
        record.canonical_ref,
    )
    index.resolve(record.tool, record.opaque_ref)
    _validate_read_scope(record)
    return session


def _fit_context_result(result: dict, budget: int) -> dict:
    truncation = result["truncation"]
    while len(json.dumps(result, ensure_ascii=False).encode("utf-8")) > budget:
        messages = result["messages"]
        if not messages:
            result["title"] = ""
            break
        removed = messages.pop()
        next_message = removed["message"]
        current_next = result.get("next_from_message")
        result["next_from_message"] = min(current_next, next_message) \
            if isinstance(current_next, int) else next_message
        truncation["omitted_blocks"] += len(removed["blocks"])
        truncation["truncated"] = True
    result["returned_message_count"] = len(result["messages"])
    result["message_range"]["to"] = (
        result["messages"][-1]["message"] if result["messages"] else None
    )
    return result


def _message_native_locator(message, index: int) -> str:
    if isinstance(message.source_id, str) and message.source_id:
        return message.source_id
    return f"index:{index}"


def _message_is_rewritable(_tool: str, message) -> bool:
    return any(block.kind == "text" for block in message.blocks)


def get_session_context(tool: str, opaque_ref: str, from_message: int = 1,
                        limit: int = 20,
                        include_tool_outputs: bool = False,
                        max_bytes: int = DEFAULT_CONTEXT_BYTES, *,
                        index: AgentSessionIndex) -> dict:
    record = index.resolve(tool, opaque_ref)
    first = bounded_int(from_message, 1, 1, 1_000_000, "from_message")
    count = bounded_int(limit, 20, 1, MAX_CONTEXT_MESSAGES, "limit")
    budget = bounded_int(
        max_bytes, DEFAULT_CONTEXT_BYTES, 1024, MAX_CONTEXT_BYTES, "max_bytes",
    )
    if not isinstance(include_tool_outputs, bool):
        raise AgentRequestError("include_tool_outputs 必须是 boolean")
    session = read_indexed_session(index, record)
    total_turns = sum(message.role == "user" for message in session.messages)
    messages, current_turn, remaining = [], 0, budget
    omitted_blocks = omitted_bytes = 0
    exhausted = False
    selected_until = min(len(session.messages), first - 1 + count)
    for message_index, message in enumerate(session.messages):
        if message.role == "user":
            current_turn += 1
        message_number = message_index + 1
        if message_number < first or message_number > selected_until:
            continue
        blocks = []
        message_clipped = False
        for block in message.blocks:
            item = None
            if block.kind == "text":
                original = redact(block.text)
                value, remaining, clipped = _take(original, remaining)
                item = {"kind": "text", "text": value}
                if clipped:
                    message_clipped = True
                    omitted_bytes += (
                        len(original.encode("utf-8"))
                        - len(value.encode("utf-8"))
                    )
            elif block.kind == "tool" and block.tool:
                result = block.tool.result
                item = {
                    "kind": "tool",
                    "name": redact(block.tool.name, 120),
                    "op": (
                        redact(str(block.tool.op), 120)
                        if block.tool.op else None
                    ),
                    "status": redact(result.status, 80) if result else None,
                    "input": "[omitted]",
                    "output": "[omitted]",
                }
                clipped = False
                if include_tool_outputs and remaining:
                    output = redact(tool_result_text(result))
                    value, remaining, output_clipped = _take(
                        output, remaining,
                    )
                    item["output"] = value
                    clipped = clipped or output_clipped
                if clipped:
                    message_clipped = True
                    omitted_blocks += 1
            elif block.kind == "image" and block.image:
                item = {
                    "kind": "image",
                    "id": redact(block.image.id, 200),
                    "mime_type": redact(block.image.mime_type, 120),
                    "filename": (
                        redact(Path(block.image.filename).name, 255)
                        if block.image.filename else None
                    ),
                    "data": "[omitted]",
                }
            else:
                omitted_blocks += 1
            if item is not None:
                blocks.append(item)
            if remaining == 0:
                exhausted = True
                break
        editable = _message_is_rewritable(tool, message)
        item = {
            "message": message_number,
            "turn": current_turn,
            "role": message.role,
            "blocks": blocks,
            "editable": editable,
            "complete": not message_clipped,
        }
        item["locator"] = index.issue_message_locator(
            record,
            _message_native_locator(message, message_index),
            message.role,
            editable,
        )
        messages.append(item)
        if exhausted:
            break
    last_returned = messages[-1]["message"] if messages else first - 1
    has_more = last_returned < len(session.messages)
    result = {
        "tool": tool,
        "ref": opaque_ref,
        "session_id": record_session_id(record, session),
        "title": redact(session.title, 200),
        "project": safe_project(session.cwd),
        "revision": record.revision,
        "message_count": len(session.messages),
        "turn_count": total_turns,
        "returned_message_count": len(messages),
        "message_range": {
            "from": first,
            "to": last_returned if messages else None,
        },
        "next_from_message": last_returned + 1 if has_more else None,
        "messages": messages,
        "truncation": {
            "truncated": exhausted or omitted_blocks > 0,
            "omitted_blocks": omitted_blocks,
            "omitted_bytes": omitted_bytes,
            "budget_bytes": budget,
        },
    }
    return _fit_context_result(result, budget)


def search_session_content(tool: str, opaque_ref: str, terms,
                           roles=None, limit: int = 20, *,
                           index: AgentSessionIndex) -> dict:
    record = index.resolve(tool, opaque_ref)
    wanted = string_set(terms, "terms", 20, 100)
    if not wanted:
        raise AgentRequestError(
            "terms 至少包含一个检索词", {"field": "terms"},
        )
    allowed_roles = string_set(roles, "roles", 2, 16)
    if not allowed_roles <= {"user", "assistant"}:
        raise AgentRequestError(
            "roles 仅允许 user/assistant", {"field": "roles"},
        )
    maximum = bounded_int(
        limit, 20, 1, MAX_CONTENT_SEARCH_RESULTS, "limit",
    )
    normalized = [(term, term.casefold()) for term in sorted(wanted)]
    session = read_indexed_session(index, record)
    total_turns = sum(message.role == "user" for message in session.messages)
    matches = []
    current_turn = 0
    total_matches = 0
    byte_limited = False
    for message_index, message in enumerate(session.messages):
        if message.role == "user":
            current_turn += 1
        if allowed_roles and message.role not in allowed_roles:
            continue
        text = "\n".join(
            block.text
            for block in message.blocks
            if block.kind == "text" and block.text
        )
        folded = text.casefold()
        hit_terms = [
            term
            for term, folded_term in normalized
            if folded_term in folded
        ]
        if not hit_terms:
            continue
        total_matches += 1
        if len(matches) >= maximum:
            continue
        first_hit = min(folded.find(term.casefold()) for term in hit_terms)
        start = max(0, first_hit - 240)
        end = min(len(text), first_hit + 560)
        snippet = (
            ("…" if start else "")
            + text[start:end]
            + ("…" if end < len(text) else "")
        )
        editable = _message_is_rewritable(tool, message)
        item = {
            "message": message_index + 1,
            "turn": current_turn,
            "role": message.role,
            "editable": editable,
            "locator": index.issue_message_locator(
                record,
                _message_native_locator(message, message_index),
                message.role,
                editable,
            ),
            "matched_terms": hit_terms,
            "snippet": redact(snippet, 900),
            "complete": start == 0 and end == len(text),
        }
        candidate = {
            "matches": [*matches, item],
            "message_count": len(session.messages),
            "turn_count": total_turns,
            "total_matches": total_matches,
        }
        if len(json.dumps(candidate, ensure_ascii=False).encode("utf-8")) \
                > MAX_AGENT_DTO_BYTES - 2048:
            byte_limited = True
            continue
        matches.append(item)
    has_more = total_matches > len(matches)
    return finalize_dto({
        "tool": tool,
        "ref": opaque_ref,
        "session_id": record_session_id(record, session),
        "revision": record.revision,
        "message_count": len(session.messages),
        "turn_count": total_turns,
        "matches": matches,
        "returned": len(matches),
        "total_matches": total_matches,
        "has_more": has_more,
        "truncation": {
            "truncated": has_more,
            "reason": (
                "byte_budget"
                if byte_limited else "result_limit"
                if has_more else None
            ),
            "budget_bytes": MAX_AGENT_DTO_BYTES,
        },
    })


def session_read(tool: str, ref: str | None = None, terms=None, roles=None,
                 from_message: int = 1, limit: int = 20,
                 include_tool_outputs: bool = False,
                 max_bytes: int = DEFAULT_CONTEXT_BYTES, *,
                 index: AgentSessionIndex) -> dict:
    if not isinstance(ref, str) or not ref:
        raise AgentRequestError(
            "必须提供 Engine 签发的 ref", {"field": "ref"},
        )
    if terms is not None:
        result = search_session_content(
            tool, ref, terms, roles=roles, limit=limit, index=index,
        )
        result["mode"] = "search"
    else:
        result = get_session_context(
            tool,
            ref,
            from_message=from_message,
            limit=limit,
            include_tool_outputs=include_tool_outputs,
            max_bytes=max_bytes,
            index=index,
        )
        result["mode"] = "context"
    return result
