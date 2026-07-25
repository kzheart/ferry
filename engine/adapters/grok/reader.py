"""Grok current ACP/update bundle to canonical session."""
from __future__ import annotations

import json
from pathlib import Path

from ...sessions.model import (
    Block, ContextCompaction, Message, Session, ToolCall, ToolResult,
    ToolResultBlock,
)
from ...sessions.tool_ops import CanonicalOp
from .rewind import filter_rewind_updates
from .store import load_grok_bundle
from .updates import aggregate_updates


def _result(value, status):
    if isinstance(value, str):
        blocks = [ToolResultBlock("text", text=value)]
    elif value is None:
        blocks = []
    else:
        blocks = [ToolResultBlock("json", data=value)]
    mapped = {"completed": "success", "failed": "error",
              "pending": "pending"}.get(status, "unknown")
    return ToolResult(mapped, blocks)


def _tool(data):
    return ToolCall(
        name=data["name"], op=CanonicalOp.TOOL_INVOKE,
        input={"namespace": "grok", "name": data["name"],
               "input": data["input"]},
        result=_result(data["output"], data["status"])
        if data["output"] is not None else None,
        source_call_id=data["id"],
    )


def _chat_messages(bundle, session):
    calls = {}
    for index, row in enumerate(bundle.chat):
        role = row.get("type") or row.get("role")
        source_id = str(row.get("id") or f"chat:{index}")
        if role == "user":
            content = row.get("content")
            parts = content if isinstance(content, list) else [
                {"type": "text", "text": str(content or "")},
            ]
            blocks = [Block("text", str(part.get("text") or ""))
                      for part in parts if isinstance(part, dict)
                      and part.get("type") == "text"]
            session.messages.append(Message("user", blocks, source_id=source_id))
        elif role == "assistant":
            blocks = [Block("text", str(row.get("content") or ""))]
            for native in row.get("tool_calls") or []:
                call = ToolCall(
                    str(native.get("name") or "tool"), CanonicalOp.TOOL_INVOKE,
                    {"namespace": "grok",
                     "name": str(native.get("name") or "tool"),
                     "input": native.get("arguments") or {}},
                    source_call_id=native.get("id"),
                )
                calls[call.source_call_id] = call
                blocks.append(Block("tool", tool=call))
            session.messages.append(Message("assistant", blocks, source_id=source_id))
        elif role == "reasoning" and row.get("content"):
            if session.messages and session.messages[-1].role == "assistant":
                session.messages[-1].blocks.append(
                    Block("thinking", str(row["content"])))
        elif role == "tool_result":
            call = calls.get(row.get("tool_call_id"))
            if call:
                call.result = _result(row.get("content"), "completed")


def read(path: str) -> Session:
    bundle = load_grok_bundle(Path(path))
    summary, info = bundle.summary, bundle.summary["info"]
    session = Session(
        "grok", info["id"], info["cwd"],
        title=str(summary.get("generated_title") or
                  summary.get("session_summary") or ""),
        parent_id=summary.get("parent_session_id"),
        model=summary.get("current_model_id"),
    )
    for diagnostic in bundle.diagnostics:
        session.lose("session.malformed_record", **diagnostic)
    if not bundle.updates:
        _chat_messages(bundle, session)
        return session
    for prompt in aggregate_updates(filter_rewind_updates(bundle.updates)):
        if prompt["user"]:
            session.messages.append(Message(
                "user", [Block("text", "".join(prompt["user"]))],
                source_id=f"{prompt['id']}:user",
            ))
        blocks = []
        for item in prompt["blocks"]:
            if item["kind"] == "tool":
                blocks.append(Block(
                    "tool", tool=_tool(prompt["tools"][item["id"]]),
                ))
            elif item["text"]:
                blocks.append(Block(item["kind"], item["text"]))
        if blocks:
            session.messages.append(Message(
                "assistant", blocks, source_id=f"{prompt['id']}:assistant",
            ))
        compaction = prompt.get("compaction")
        if compaction:
            session.context_compactions.append(ContextCompaction(
                id=str(compaction.get("id") or f"{prompt['id']}:compaction"),
                source="grok",
                after_message_id=(session.messages[-1].source_id
                                  if session.messages else None),
                summary_status=("available" if compaction.get("summary")
                                else "missing"),
                summary_text=str(compaction.get("summary") or ""),
                metrics={"tokens_before": compaction["tokensBefore"]}
                if compaction.get("tokensBefore") is not None else {},
            ))
        if prompt["unknown"]:
            session.lose(
                "migration.unknown_block_dropped", source="grok",
                block_type="session_update", count=len(prompt["unknown"]),
            )
    return session
