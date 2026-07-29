"""供 Ferry Agent 使用的限量会话读取。"""
from __future__ import annotations

import json

from ..adapters.contracts import NativeSessionReference
from ..errors import AgentRequestError
from .index import AgentSessionIndex, IndexedSession
from .model import native_locator, tool_result_text
from .safety import (
    MAX_AGENT_DTO_BYTES,
    bounded_int,
    bounded_json,
    finalize_dto,
    record_session_id,
    string_set,
    truncate_text,
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


def _take_json(value, remaining: int) -> tuple[object, int, bool]:
    if remaining < 32:
        return {}, max(0, remaining - 2), True
    bounded = bounded_json(value, max(128, min(remaining, 12 * 1024)))
    encoded = json.dumps(bounded, ensure_ascii=False).encode("utf-8")
    if len(encoded) <= remaining:
        return bounded, remaining - len(encoded), bounded != value
    marker = {"truncated": True}
    marker_size = len(json.dumps(marker).encode("utf-8"))
    if marker_size <= remaining:
        return marker, remaining - marker_size, True
    return {}, max(0, remaining - 2), True


def read_indexed_session(index: AgentSessionIndex, record: IndexedSession):
    browser = index.ports.adapter(record.tool).browser
    native_ref = NativeSessionReference(
        canonical_ref=record.canonical_ref,
        root=record.root,
        storage_kind=record.storage_kind,
    )
    browser.validate_read_scope(native_ref)
    session = getattr(browser, "read_agent", browser.read)(
        record.canonical_ref,
    )
    index.resolve(record.tool, record.opaque_ref)
    browser.validate_read_scope(native_ref)
    return session


def _shrink_sole_message(item: dict, truncation: dict) -> bool:
    """仅剩一条消息仍超预算时继续压小它；无可再压返回 False。

    弹掉最后一条消息会让 next_from_message 退回 from_message,调用方按
    游标翻页就会在同一条消息上死循环,所以宁可返回更狠的截断内容。
    """
    blocks = item["blocks"]
    texts = [block for block in blocks
             if block.get("kind") == "text" and block.get("text")]
    if texts:
        largest = max(texts, key=lambda b: len(b["text"].encode("utf-8")))
        encoded = largest["text"].encode("utf-8")
        clipped = encoded[:len(encoded) // 2].decode("utf-8", errors="ignore")
        truncation["omitted_bytes"] += (
            len(encoded) - len(clipped.encode("utf-8"))
        )
        largest["text"] = clipped
    elif blocks:
        blocks.pop()
        truncation["omitted_blocks"] += 1
    else:
        return False
    item["complete"] = False
    truncation["truncated"] = True
    return True


def _fit_context_result(result: dict, budget: int) -> dict:
    truncation = result["truncation"]
    while len(json.dumps(result, ensure_ascii=False).encode("utf-8")) > budget:
        messages = result["messages"]
        if len(messages) > 1:
            removed = messages.pop()
            next_message = removed["message"]
            current_next = result.get("next_from_message")
            result["next_from_message"] = min(current_next, next_message) \
                if isinstance(current_next, int) else next_message
            truncation["omitted_blocks"] += len(removed["blocks"])
            truncation["truncated"] = True
        elif messages and _shrink_sole_message(messages[0], truncation):
            continue
        else:
            result["title"] = ""
            break
    result["returned_message_count"] = len(result["messages"])
    result["message_range"]["to"] = (
        result["messages"][-1]["message"] if result["messages"] else None
    )
    return result


def _message_is_rewritable(_tool: str, message) -> bool:
    return any(block.kind == "text" for block in message.blocks)


def browser_locator_issuer(index: AgentSessionIndex, record: IndexedSession):
    """UI 浏览路径的 locator 签发器:与 Agent 读取共用同一 (ref, 原生定位,
    role) 键,保证两条路径对同一条消息拿到同一个 fml_ 引用。"""
    def issue(message, message_index: int) -> str:
        return index.issue_message_locator(
            record,
            native_locator(message, message_index),
            message.role,
            _message_is_rewritable(record.tool, message),
        )
    return issue


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
                original = block.text
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
                tool_input, remaining, input_clipped = _take_json(
                    block.tool.input, remaining,
                )
                item = {
                    "kind": "tool",
                    "name": truncate_text(block.tool.name, 120)[0],
                    "op": truncate_text(str(block.tool.op), 120)[0]
                    if block.tool.op else None,
                    "status": truncate_text(result.status, 80)[0]
                    if result else None,
                    "input": tool_input,
                    "output": "[omitted]",
                }
                clipped = input_clipped
                if include_tool_outputs and remaining:
                    output = tool_result_text(result)
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
                    "id": truncate_text(block.image.id, 200)[0],
                    "mime_type": truncate_text(block.image.mime_type, 120)[0],
                    "filename": truncate_text(
                        block.image.filename, 1024,
                    )[0] if block.image.filename else None,
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
            native_locator(message, message_index),
            message.role,
            editable,
        )
        messages.append(item)
        if exhausted:
            break
    last_returned = messages[-1]["message"] if messages else first - 1
    has_more = last_returned < len(session.messages)
    title, title_truncated = truncate_text(session.title, 200)
    project, project_truncated = truncate_text(session.cwd, 1024)
    result = {
        "tool": tool,
        "ref": opaque_ref,
        "session_id": record_session_id(record, session),
        "title": title,
        "project": project,
        "title_truncated": title_truncated,
        "project_truncated": project_truncated,
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


def _searchable_text(message, include_tool_outputs: bool) -> str:
    """检索用文本。

    编码类会话里大量内容(改过的代码、读到的文件、命令输出)只存在于工具调用
    里,只搜可见正文会让"这个会话提到过 X 吗"得出错误的否定结论,所以调用方
    要求带上工具输出时,一并纳入检索范围。
    """
    parts = []
    for block in message.blocks:
        if block.kind == "text" and block.text:
            parts.append(block.text)
        elif include_tool_outputs and block.kind == "tool" and block.tool:
            parts.append(f"[tool {block.tool.name}]")
            output = tool_result_text(block.tool.result)
            if output:
                parts.append(output)
    return "\n".join(parts)


def search_session_content(tool: str, opaque_ref: str, terms,
                           roles=None, limit: int = 20,
                           include_tool_outputs: bool = False, *,
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
        text = _searchable_text(message, include_tool_outputs)
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
                native_locator(message, message_index),
                message.role,
                editable,
            ),
            "matched_terms": hit_terms,
            "snippet": truncate_text(snippet, 900)[0],
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
        "searched_scope": (
            "visible_text_and_tool_outputs"
            if include_tool_outputs else "visible_text_only"
        ),
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
    if not isinstance(include_tool_outputs, bool):
        raise AgentRequestError("include_tool_outputs 必须是 boolean")
    if terms is not None:
        result = search_session_content(
            tool, ref, terms, roles=roles, limit=limit,
            include_tool_outputs=include_tool_outputs, index=index,
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
