"""规范会话读取、树装配与 RPC DTO。"""

from ..adapters.base.migration import assemble_tree
from .ports import current


def read_tree(tool_name: str, ref: str):
    ports = current()
    tool = ports.adapter(tool_name)
    return assemble_tree(tool.browser, ref, ports.cache_factory())


def _messages(messages):
    result = []
    for index, message in enumerate(messages):
        blocks = []
        for block in message.blocks:
            if block.kind == "text":
                blocks.append({"kind": "text", "text": block.text, "size": len(block.text)})
            elif block.kind == "tool":
                call = block.tool
                value = call.input if isinstance(call.input, dict) else str(call.input)
                blocks.append({"kind": "tool", "name": call.name, "op": call.op,
                    "input": value, "output": call.output, "size": len(call.output or "")})
        entry = {"index": index, "role": message.role, "blocks": blocks}
        if message.raw and isinstance(message.raw[0], dict) and message.raw[0].get("uuid"):
            entry.update(uuid=message.raw[0]["uuid"], locator=message.raw[0]["uuid"])
        elif message.source_id:
            entry["locator"] = message.source_id
        else:
            entry["locator"] = f"index:{index}"
        result.append(entry)
    return result


def session_json(session):
    children = [session_json(child) for child in session.children]
    edges = [{name: getattr(edge, name) for name in (
        "parent_session_id", "child_session_id", "source_call_id",
        "spawn_message_id", "result_message_id", "agent_id", "agent_path",
        "agent_type", "prompt", "status", "meta")} for edge in session.agent_edges]
    messages = _messages(session.messages)
    turns = []
    current = None
    for message in messages:
        if message["role"] == "user":
            current = {"turn": len(turns) + 1, "user": message,
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
        "agent_type": session.agent_type, "count": len(messages),
        "child_count": len(children), "tree_count": 1 + sum(child["tree_count"] for child in children),
        "loss": list(session.loss), "messages": messages, "turns": turns,
        "children": children,
        "agent_edges": edges}


def show(tool: str, ref: str) -> dict:
    return session_json(read_tree(tool, ref))
