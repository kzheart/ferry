"""会话目录与 Agent 只读查询；输出均经过限量、脱敏和引用收窄。"""
from __future__ import annotations

import json
from pathlib import Path

from ..errors import AgentReferenceError, AgentRequestError, LocatorStaleError
from ..context import EngineContext
from .model import tool_result_text
from .index import AgentSessionIndex, IndexedSession
from .safety import (
    MAX_AGENT_DTO_BYTES,
    bounded_int,
    bounded_json,
    finalize_dto,
    record_session_id,
    redact,
    safe_project,
    string_set,
    validate_agent_edit_ops,
    validated_interval,
)
MAX_CONTENT_SEARCH_RESULTS = 50
MAX_CONTEXT_MESSAGES = 50
MAX_CONTEXT_BYTES = 64 * 1024
DEFAULT_CONTEXT_BYTES = 24 * 1024


def resolve_edit_ops(index: AgentSessionIndex, record: IndexedSession,
                     ops: list[dict]) -> list[dict]:
    """把 Agent 可见的 fml_ 定位符换成适配器原生定位符。"""
    resolved = []
    for op in ops:
        item = dict(op)
        if item.get("op") == "rewrite":
            message = index.resolve_message_locator(record, item["locator"])
            if not message.editable:
                raise AgentRequestError(
                    "目标消息不支持文本改写",
                    {"field": "locator", "locator": item["locator"],
                     "hint": "仅使用 editable=true 的消息引用"})
            item["locator"] = message.native_locator
        resolved.append(item)
    return resolved


def public_locator_error(ops: list[dict]) -> LocatorStaleError:
    locator = next((op.get("locator") for op in ops
                    if op.get("op") == "rewrite"), None)
    return LocatorStaleError(
        "消息定位信息与当前会话不匹配",
        {"field": "locator", "locator": locator,
         "hint": "重新调用 ferry_get_session_context，并原样使用 messages[].locator"})


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
    session = getattr(browser, "read_agent", browser.read)(record.canonical_ref)
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
        result["messages"][-1]["message"] if result["messages"] else None)
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
    budget = bounded_int(max_bytes, DEFAULT_CONTEXT_BYTES, 1024,
                          MAX_CONTEXT_BYTES, "max_bytes")
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
                    omitted_bytes += len(original.encode("utf-8")) - len(value.encode("utf-8"))
            elif block.kind == "tool" and block.tool:
                result = block.tool.result
                item = {"kind": "tool", "name": redact(block.tool.name, 120),
                        "op": redact(str(block.tool.op), 120) if block.tool.op else None,
                        "status": redact(result.status, 80) if result else None,
                        "input": "[omitted]", "output": "[omitted]"}
                clipped = False
                if include_tool_outputs and remaining:
                    output = redact(tool_result_text(result))
                    value, remaining, output_clipped = _take(output, remaining)
                    item["output"] = value
                    clipped = clipped or output_clipped
                if clipped:
                    message_clipped = True
                    omitted_blocks += 1
            elif block.kind == "image" and block.image:
                item = {"kind": "image", "id": redact(block.image.id, 200),
                        "mime_type": redact(block.image.mime_type, 120),
                        "filename": redact(Path(block.image.filename).name, 255)
                        if block.image.filename else None,
                        "data": "[omitted]"}
            else:
                omitted_blocks += 1
            if item is not None:
                blocks.append(item)
            if remaining == 0:
                exhausted = True
                break
        editable = _message_is_rewritable(tool, message)
        item = {"message": message_number, "turn": current_turn,
                "role": message.role, "blocks": blocks, "editable": editable,
                "complete": not message_clipped}
        item["locator"] = index.issue_message_locator(
            record, _message_native_locator(message, message_index), message.role, editable)
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
        "message_range": {"from": first,
                          "to": last_returned if messages else None},
        "next_from_message": last_returned + 1 if has_more else None,
        "messages": messages,
        "truncation": {"truncated": exhausted or omitted_blocks > 0,
                       "omitted_blocks": omitted_blocks,
                       "omitted_bytes": omitted_bytes,
                       "budget_bytes": budget},
    }
    return _fit_context_result(result, budget)


def search_session_content(tool: str, opaque_ref: str, terms,
                           roles=None, limit: int = 20, *,
                           index: AgentSessionIndex) -> dict:
    """在单个会话的可见文本中检索，返回可直接用于改写的消息引用。"""
    record = index.resolve(tool, opaque_ref)
    wanted = string_set(terms, "terms", 20, 100)
    if not wanted:
        raise AgentRequestError("terms 至少包含一个检索词", {"field": "terms"})
    allowed_roles = string_set(roles, "roles", 2, 16)
    if not allowed_roles <= {"user", "assistant"}:
        raise AgentRequestError(
            "roles 仅允许 user/assistant", {"field": "roles"})
    maximum = bounded_int(
        limit, 20, 1, MAX_CONTENT_SEARCH_RESULTS, "limit")
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
        text = "\n".join(block.text for block in message.blocks
                         if block.kind == "text" and block.text)
        folded = text.casefold()
        hit_terms = [term for term, folded_term in normalized
                     if folded_term in folded]
        if not hit_terms:
            continue
        total_matches += 1
        if len(matches) >= maximum:
            continue
        first_hit = min(folded.find(term.casefold()) for term in hit_terms)
        start = max(0, first_hit - 240)
        end = min(len(text), first_hit + 560)
        snippet = ("…" if start else "") + text[start:end] + \
            ("…" if end < len(text) else "")
        editable = _message_is_rewritable(tool, message)
        item = {
                "message": message_index + 1,
            "turn": current_turn,
            "role": message.role,
            "editable": editable,
            "locator": index.issue_message_locator(
                record, _message_native_locator(message, message_index), message.role, editable),
            "matched_terms": hit_terms,
            "snippet": redact(snippet, 900),
            "complete": start == 0 and end == len(text),
        }
        candidate = {"matches": [*matches, item], "message_count": len(session.messages),
                     "turn_count": total_turns, "total_matches": total_matches}
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
        "truncation": {"truncated": has_more,
                       "reason": "byte_budget" if byte_limited
                       else "result_limit" if has_more else None,
                       "budget_bytes": MAX_AGENT_DTO_BYTES},
    })


def session_read(tool: str, ref: str | None = None, terms=None, roles=None,
                 from_message: int = 1, limit: int = 20,
                 include_tool_outputs: bool = False,
                 max_bytes: int = DEFAULT_CONTEXT_BYTES, *,
                 index: AgentSessionIndex) -> dict:
    """读取 Engine 索引会话；只接受 scan/search 签发的 opaque ref。"""
    if not isinstance(ref, str) or not ref:
        raise AgentRequestError("必须提供 Engine 签发的 ref", {"field": "ref"})
    if terms is not None:
        result = search_session_content(
            tool, ref, terms, roles=roles, limit=limit, index=index)
        result["mode"] = "search"
    else:
        result = get_session_context(
            tool, ref, from_message=from_message, limit=limit,
            include_tool_outputs=include_tool_outputs, max_bytes=max_bytes,
            index=index)
        result["mode"] = "context"
    return result


def preview_migration(source_tool: str, opaque_ref: str, target_tool: str,
                      max_turn: int | None = None, *,
                      index: AgentSessionIndex) -> dict:
    record = index.resolve(source_tool, opaque_ref)
    if target_tool not in index.ports.adapters():
        raise AgentRequestError("未知目标 Agent", {"target_tool": target_tool})
    session = read_indexed_session(index, record)
    if max_turn is not None:
        max_turn = bounded_int(max_turn, 1, 1, 1_000_000, "max_turn")
        from ..operations.migrate import _truncate_rounds
        _truncate_rounds(session, max_turn)
    target = index.ports.adapter(target_tool).migration_target
    loss = target.plan(session)
    from ..operations.migrate import _migration_counts
    tree_count, message_count = _migration_counts(session)
    edge_count = sum(len(node.agent_edges) for node in session.walk())
    topology = {"nodes": tree_count, "edges": max(0, tree_count - 1),
                "agent_edges": edge_count, "preserved": True}
    return finalize_dto({"source_tool": source_tool, "target_tool": target_tool,
            "ref": opaque_ref, "revision": record.revision,
            "source_session_id": record_session_id(record, session),
            "message_count": message_count,
            "root_message_count": len(session.messages), "tree_count": tree_count,
            "child_count": tree_count - 1, "loss": bounded_json(loss),
            "topology": topology, "max_turn": max_turn})


def preview_edit(tool: str, opaque_ref: str, *, ops,
                 index: AgentSessionIndex) -> dict:
    record = index.resolve(tool, opaque_ref)
    adapter = index.ports.adapter(tool)
    validate_agent_edit_ops(ops)
    if len(json.dumps(ops, ensure_ascii=False, default=str).encode()) > 64 * 1024:
        raise AgentRequestError("ops 超过 64 KiB")
    from ..operations.edit import preview
    editor = adapter.editor
    native_ops = resolve_edit_ops(index, record, ops)
    try:
        result = preview(editor, record.canonical_ref, native_ops,
                         loader=getattr(editor, "load_preview", None))
    except LocatorStaleError as error:
        raise public_locator_error(ops) from error
    index.resolve(tool, opaque_ref)
    return finalize_dto({"tool": tool, "ref": opaque_ref, "mode": "edit",
            "session_id": record_session_id(record),
            "revision": redact(str(result["revision"]), 256),
            "before": bounded_json(result["before"], 12 * 1024),
            "after": bounded_json(result["after"], 12 * 1024),
            "changes": bounded_json(result["changes"], 12 * 1024)})
