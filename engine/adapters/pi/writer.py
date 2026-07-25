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
from .reader import read


OP_NAMES = {
    CanonicalOp.SHELL_EXEC: "bash", CanonicalOp.FS_READ: "read",
    CanonicalOp.FS_WRITE: "write", CanonicalOp.FS_EDIT: "edit",
    CanonicalOp.FS_SEARCH: "grep", CanonicalOp.FS_GLOB: "find",
}
OP_FIDELITY = {op: "native" for op in OP_NAMES} | {
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
    name = OP_NAMES[tool.op]
    if name == "bash":
        return name, {"command": value["command"],
                      **({"timeout": value["timeout_ms"] / 1000}
                         if "timeout_ms" in value else {})}
    if name in {"read", "write"}:
        return name, {"path": value["file_path"],
                      **{key: value[key] for key in ("content", "offset", "limit")
                         if key in value}}
    if name == "edit":
        return name, {"path": value["file_path"], "edits": [{
            "oldText": value["old"], "newText": value["new"],
        }]}
    if name == "grep":
        return name, {"pattern": value["query"],
                      **({"path": value["path"]} if "path" in value else {}),
                      **({"glob": value["glob"]} if "glob" in value else {}),
                      **({"limit": value["max_results"]}
                         if "max_results" in value else {})}
    return name, {"pattern": value["pattern"],
                  **({"path": value["path"]} if "path" in value else {})}


def _records(session, cwd, sid, parent_session=None):
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
                try:
                    name, arguments = _native_input(block.tool)
                except (KeyError, TypeError):
                    content.append({"type": "text", "text":
                                    f"[Tool {block.tool.name}] {json.dumps(block.tool.input, ensure_ascii=False)}"})
                    continue
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
        records = _records(node, node_cwd, sid, parent_session)
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
