"""Canonical session writer for current Pi v3 JSONL."""
from __future__ import annotations

import json
import os
import time
import uuid
from pathlib import Path

from ...sessions.model import tool_result_text
from ...sessions.tool_ops import CanonicalOp
from ...system.paths import pi_session_roots
from ..shared.narration import narrate
from .dialect import DIALECT
from .reader import read


OP_FIDELITY = {op: "native" for op in DIALECT.write_ops()} | {
    CanonicalOp.TOOL_INVOKE: "native",
    CanonicalOp.FS_PATCH: "degrade", CanonicalOp.WEB_FETCH: "degrade",
    CanonicalOp.WEB_SEARCH: "degrade", CanonicalOp.AGENT_SPAWN: "degrade",
}


def _stamp():
    return time.strftime("%Y-%m-%dT%H:%M:%S.000Z", time.gmtime())


def _native_input(tool):
    value = tool.input
    if tool.op == CanonicalOp.TOOL_INVOKE:
        return str(value["name"]), value["input"]
    return DIALECT.render(tool.op, value)


def _tool_native(tool, session, message, tool_decider):
    """返回 (name, arguments)；None 表示该调用降级为叙述文本。"""
    if tool_decider is None:
        try:
            return _native_input(tool)
        except (KeyError, TypeError):
            return None
    decision = tool_decider(tool, session, message)
    if decision.rendered is None:
        return None
    return (str(decision.rendered.get("name") or tool.name),
            decision.rendered.get("input", tool.input))


def _records(session, cwd, sid, parent_session=None, tool_decider=None):
    header = {"type": "session", "version": 3, "id": sid,
              "timestamp": _stamp(), "cwd": cwd}
    if parent_session:
        header["parentSession"] = parent_session
    records, parent = [header], None
    for message in session.messages:
        content, tools = [], []
        for block in message.blocks:
            if block.kind == "text":
                content.append({"type": "text", "text": block.text})
            elif block.kind == "thinking" and message.role == "assistant":
                content.append({"type": "thinking", "thinking": block.text})
            elif block.kind == "image" and block.image:
                content.append({"type": "image", "data": block.image.data,
                                "mimeType": block.image.mime_type})
            elif block.kind == "tool" and block.tool:
                native = _tool_native(block.tool, session, message, tool_decider)
                if native is None:
                    content.append({"type": "text",
                                    "text": narrate(block.tool)})
                    continue
                name, arguments = native
                call_id = block.tool.source_call_id or "call_" + uuid.uuid4().hex[:16]
                content.append({"type": "toolCall", "id": call_id,
                                "name": name, "arguments": arguments})
                tools.append((block.tool, call_id))
        entry_id = uuid.uuid4().hex[:12]
        native = {"role": message.role, "content": content,
                  "timestamp": int(time.time() * 1000)}
        if message.role == "assistant":
            native.update(api="ferry", provider=session.model_provider or "ferry",
                          model=session.model or "migrated",
                          usage={"input": 0, "output": 0, "cacheRead": 0,
                                 "cacheWrite": 0, "totalTokens": 0,
                                 "cost": {"input": 0, "output": 0,
                                          "cacheRead": 0, "cacheWrite": 0,
                                          "total": 0}},
                          stopReason="toolUse" if tools else "stop")
        records.append({"type": "message", "id": entry_id,
                        "parentId": parent, "timestamp": _stamp(),
                        "message": native})
        parent = entry_id
        for tool, call_id in tools:
            result_id = uuid.uuid4().hex[:12]
            records.append({"type": "message", "id": result_id,
                            "parentId": parent, "timestamp": _stamp(),
                            "message": {"role": "toolResult",
                                "toolCallId": call_id, "toolName": tool.name,
                                "content": [{"type": "text",
                                             "text": tool_result_text(tool.result)}],
                                "isError": bool(tool.result and tool.result.status == "error"),
                                "timestamp": int(time.time() * 1000)}})
            parent = result_id
    return records


def write(session, cwd: str, root: Path | None = None, tool_decider=None):
    root = Path(root) if root else pi_session_roots()[0]
    root.mkdir(parents=True, exist_ok=True)

    def publish(node, node_cwd, parent_session=None):
        sid = str(uuid.uuid4())
        filename_stamp = time.strftime("%Y-%m-%dT%H-%M-%S", time.gmtime())
        path = root / f"{filename_stamp}_{sid}.jsonl"
        temp = root / f".{sid}.{os.getpid()}.tmp"
        records = _records(node, node_cwd, sid, parent_session, tool_decider)
        temp.write_text("\n".join(json.dumps(row, ensure_ascii=False)
                                  for row in records) + "\n")
        read(str(temp))
        from .probe import _probe_path

        report = _probe_path(str(temp), node_cwd)
        if report["status"] != "passed":
            temp.unlink(missing_ok=True)
            raise RuntimeError("Pi RPC 无法加载生成会话")
        read(str(temp))
        os.replace(temp, path)
        for child in node.children:
            publish(child, child.cwd or node_cwd, str(path))
        return sid, path

    return publish(session, cwd)
