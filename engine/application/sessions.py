"""规范会话读取、树装配与 RPC DTO。"""

from ..adapters.registry import adapter
from ..infrastructure.scan_cache import ScanCache


def _walk_meta(nodes):
    for node in nodes:
        yield node
        yield from _walk_meta(node.get("children", []))


def read_tree(tool_name: str, ref: str):
    tool = adapter(tool_name)
    path = tool.resolve_ref(ref)
    session = tool.reader(path)
    if session.children:
        return session
    roots = tool.scanner(ScanCache())
    target = next((node for node in _walk_meta(roots)
        if node["id"] == session.source_id or (node.get("path") and node["path"] == str(path))), None)
    if target is None:
        return session

    def attach(current, meta, root_id):
        current.source_id = meta["id"]
        current.root_id = root_id
        current.parent_id = meta.get("parent_id")
        current.title = current.title or meta.get("title", "")
        current.cwd = current.cwd or meta.get("dir", "")
        existing = {child.source_id: child for child in current.children}
        children = []
        for child_meta in meta.get("children", []):
            child = existing.get(child_meta["id"])
            if child is None:
                child = tool.reader(child_meta.get("path") or child_meta["id"])
            attach(child, child_meta, root_id)
            children.append(child)
        current.children = children

    attach(session, target, target.get("root_id") or target["id"])
    return session


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
    return {"tool": session.source_tool, "id": session.source_id,
        "title": session.title, "dir": session.cwd,
        "root_id": session.root_id or session.source_id, "parent_id": session.parent_id,
        "agent_id": session.agent_id, "agent_path": session.agent_path,
        "agent_type": session.agent_type, "count": len(messages),
        "child_count": len(children), "tree_count": 1 + sum(child["tree_count"] for child in children),
        "loss": list(session.loss), "messages": messages, "children": children,
        "agent_edges": edges}


def show(tool: str, ref: str) -> dict:
    return session_json(read_tree(tool, ref))
