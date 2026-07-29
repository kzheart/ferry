"""规范会话读取、树装配与 RPC DTO。"""

from ..adapters.shared.migration import assemble_tree
from ..errors import SessionAssetNotFoundError
from ..context import EngineContext
from .model import tool_result_text
from .tool_ops import CanonicalOp


def _tool_view(call):
    value = call.input if isinstance(call.input, dict) else str(call.input)
    name = call.name
    if call.op == CanonicalOp.TOOL_INVOKE and isinstance(call.input, dict):
        name = str(call.input.get("name") or name)
        value = call.input.get("input", value)
    return name, value


def read_tree(tool_name: str, ref: str, ports: EngineContext):
    tool = ports.adapter(tool_name)
    return assemble_tree(tool.browser, ref, ports.cache_factory())


DEFAULT_BROWSER_MESSAGE_LIMIT = 30


def _messages(messages, offset: int = 0, locator_issuer=None):
    result = []
    for index, message in enumerate(messages):
        blocks = []
        for block in message.blocks:
            if block.kind == "text":
                blocks.append({"kind": "text", "text": block.text, "size": len(block.text)})
            elif block.kind == "tool":
                call = block.tool
                name, value = _tool_view(call)
                output = tool_result_text(call.result)
                blocks.append({"kind": "tool", "name": name, "op": call.op,
                    "input": value, "output": output, "size": len(output)})
            elif block.kind == "image" and block.image:
                blocks.append({"kind": "image", "image": {
                    "id": block.image.id, "mime_type": block.image.mime_type,
                    "filename": block.image.filename}})
        entry = {"index": offset + index, "role": message.role, "blocks": blocks}
        if locator_issuer is not None:
            entry["locator"] = locator_issuer(message, offset + index)
        elif message.source_id:
            entry["locator"] = message.source_id
        else:
            entry["locator"] = f"index:{index}"
        result.append(entry)
    return result


def _context_compactions(session, messages):
    turn = 0
    turn_by_message = {}
    for message in messages:
        if message.role == "user":
            turn += 1
        if message.source_id:
            turn_by_message[message.source_id] = turn

    result = []
    for sequence, compaction in enumerate(session.context_compactions, start=1):
        result.append({
            "id": compaction.id,
            "source": compaction.source,
            "sequence": sequence,
            "after_turn": turn_by_message.get(compaction.after_message_id, 0),
            "after_message_locator": compaction.after_message_id,
            "event_locator": compaction.event_locator,
            "created_at": compaction.created_at,
            "trigger": compaction.trigger,
            "state": compaction.state,
            "summary": {
                "status": compaction.summary_status,
                "text": compaction.summary_text,
                "locator": compaction.summary_message_id,
            },
            "tail": {
                "status": compaction.tail_status,
                "start_locator": compaction.tail_start_locator,
                "start_message_index": compaction.tail_start_message_index,
            },
            "metrics": dict(compaction.metrics),
        })
    return result


def _context_status(compactions):
    if not compactions:
        return {"state": "full", "compaction_count": 0,
                "summary_status": "not_applicable"}
    states = {item["state"] for item in compactions}
    state = ("in_progress" if "in_progress" in states
             else "incomplete" if "incomplete" in states
             else "compacted")
    latest = compactions[-1]
    return {
        "state": state,
        "compaction_count": len(compactions),
        "summary_status": latest["summary"]["status"],
    }


def _page_messages(messages, start: int, limit: int | None):
    first = max(0, start - 1)
    if limit is None:
        return first, len(messages), messages[first:]
    last = min(len(messages), first + max(1, limit))
    # 分页只在下一条用户消息前截断，避免把同一轮的 AI 回复拆开。
    while last < len(messages) and messages[last].role != "user":
        last += 1
    return first, last, messages[first:last]


def session_json(session, *, from_message: int = 1,
                 message_limit: int | None = None,
                 include_messages: bool = True,
                 include_tree: bool = True,
                 tree_count: int | None = None,
                 child_count: int | None = None,
                 total_count: int | None = None,
                 locator_issuer=None):
    children = [session_json(child) for child in session.children] \
        if include_tree else []
    edges = [{name: getattr(edge, name) for name in (
        "parent_session_id", "child_session_id", "source_call_id",
        "spawn_message_id", "result_message_id", "agent_id", "agent_path",
        "agent_type", "prompt", "status", "association", "confidence",
    )} for edge in session.agent_edges]
    internal_summary_ids = {
        compaction.summary_message_id
        for compaction in session.context_compactions
        if compaction.summary_message_id
    }
    display_messages = [
        message for message in session.messages
        if message.source_id not in internal_summary_ids
    ]
    first, last, page = _page_messages(
        display_messages, from_message, message_limit,
    )
    messages = _messages(page, first, locator_issuer)
    context_compactions = _context_compactions(session, display_messages)
    turns = []
    turn_offset = sum(
        message.role == "user" for message in display_messages[:first]
    )
    current = None
    for message in messages:
        if message["role"] == "user":
            current = {"turn": turn_offset + len(turns) + 1, "user": message,
                       "turn_locator": message["locator"],
                       "assistant_reply": {"items": []}}
            turns.append(current)
        elif message["role"] == "assistant" and current is not None:
            for block in message["blocks"]:
                if block["kind"] == "text":
                    current["assistant_reply"]["items"].append(
                        {"kind": "text", "text": block["text"]})
                elif block["kind"] == "tool":
                    current["assistant_reply"]["items"].append({
                        "kind": "tool", "name": block["name"],
                        "input": block["input"], "output": block["output"]})
    return {"tool": session.source_tool, "id": session.source_id,
        "title": session.title, "dir": session.cwd,
        "root_id": session.root_id or session.source_id, "parent_id": session.parent_id,
        "agent_id": session.agent_id, "agent_path": session.agent_path,
        "agent_type": session.agent_type,
        "count": total_count if total_count is not None else len(display_messages),
        "root_message_count": len(display_messages),
        "returned_message_count": len(messages),
        "message_range": {
            "from": first + 1 if messages else None,
            "to": last if messages else None,
        },
        "next_from_message": last + 1 if last < len(display_messages) else None,
        "context": _context_status(context_compactions),
        "context_compactions": context_compactions,
        "child_count": child_count if child_count is not None else len(children),
        "tree_count": tree_count if tree_count is not None
        else 1 + sum(child["tree_count"] for child in children),
        "loss": list(session.loss),
        "messages": messages if include_messages else [], "turns": turns,
        "children": children,
        "agent_edges": edges}


def show(tool: str, ref: str, ports: EngineContext, *,
         from_message: int = 1,
         message_limit: int | None = DEFAULT_BROWSER_MESSAGE_LIMIT,
         include_messages: bool = False,
         tree_count: int | None = None,
         child_count: int | None = None,
         total_count: int | None = None,
         locator_issuer=None) -> dict:
    return session_json(
        read_tree(tool, ref, ports),
        from_message=from_message,
        message_limit=message_limit,
        include_messages=include_messages,
        include_tree=False,
        tree_count=tree_count,
        child_count=child_count,
        total_count=total_count,
        locator_issuer=locator_issuer,
    )


def session_asset(tool: str, ref: str, asset_id: str, ports: EngineContext) -> dict:
    for session in read_tree(tool, ref, ports).walk():
        for message in session.messages:
            for block in message.blocks:
                if block.kind == "image" and block.image and block.image.id == asset_id:
                    return {"mime_type": block.image.mime_type, "data": block.image.data,
                            "filename": block.image.filename}
    raise SessionAssetNotFoundError(asset_id)
