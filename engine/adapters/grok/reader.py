"""Grok current ACP/update bundle to canonical session."""
from __future__ import annotations

import json
from pathlib import Path

from ...sessions.model import (
    Block, ContextCompaction, Message, Session, ToolCall, ToolResult,
    ToolResultBlock,
)
from ...sessions.tool_ops import CanonicalOp
from .dialect import DIALECT
from .rewind import filter_rewind_updates
from .store import load_grok_bundle
from .updates import aggregate_updates


def _bytes_text(value):
    """Grok 把命令输出编码成字节数组([97,110,...]),解回 UTF-8 文本。"""
    if isinstance(value, list):
        if not value:
            return ""
        if all(isinstance(item, int) and 0 <= item < 256 for item in value):
            return bytes(value).decode("utf-8", "replace")
        return None
    return value if isinstance(value, str) else None


def _unwrap_output(value):
    """rawOutput 类型信封 -> (语义文本, 元数据)；解不开返回 (None, {})。

    信封里除文本外只有回显输入或传输元数据,拆出文本不构成信息损失,
    却能让结果以原生 text 块迁移而不是 json 投影。
    """
    if not isinstance(value, dict):
        return None, {}
    kind = value.get("type")
    if kind == "ReadFile":
        content = (value.get("FileContent") or {}).get("content")
        if isinstance(content, str):
            return content, {}
    elif kind == "ListDir":
        content = (value.get("Content") or {}).get("content")
        if isinstance(content, str):
            return content, {}
    elif kind == "Text":
        if isinstance(value.get("text"), str):
            return value["text"], {}
    elif kind == "Todo":
        summary = (value.get("TodosUpdated") or {}).get("summary_for_prompt")
        if isinstance(summary, str):
            return summary, {}
    elif kind == "SearchReplace":
        # EditsApplied 只回显 old/new 输入,编辑成功本身没有输出文本。
        if isinstance(value.get("EditsApplied"), dict):
            return "", {}
    elif kind == "Bash":
        text = _bytes_text(value.get("output"))
        if text is None and isinstance(value.get("output_for_prompt"), str):
            text = value["output_for_prompt"]
        if text is not None:
            meta = {}
            if isinstance(value.get("exit_code"), int) and \
                    not isinstance(value.get("exit_code"), bool):
                meta["exit_code"] = value["exit_code"]
            if isinstance(value.get("truncated"), bool):
                meta["truncated"] = value["truncated"]
            return text, meta
    elif kind == "GrepSearch":
        text = _bytes_text(value.get("stdout"))
        if text is not None:
            meta = {}
            if isinstance(value.get("exit_code"), int) and \
                    not isinstance(value.get("exit_code"), bool):
                meta["exit_code"] = value["exit_code"]
            stderr = _bytes_text(value.get("stderr"))
            if stderr:
                meta["stderr"] = stderr
            return text, meta
    return None, {}


def _result(value, status):
    text, meta = _unwrap_output(value)
    if text is not None:
        blocks = [ToolResultBlock("text", text=text)] if text else []
    elif isinstance(value, str):
        blocks = [ToolResultBlock("text", text=value)]
    elif value is None:
        blocks = []
    else:
        blocks = [ToolResultBlock("json", data=value)]
    mapped = {"completed": "success", "failed": "error",
              "pending": "pending"}.get(status, "unknown")
    if mapped == "success" and meta.get("exit_code") not in (None, 0):
        mapped = "error"
    return ToolResult(mapped, blocks,
                      stderr=meta.get("stderr"),
                      exit_code=meta.get("exit_code"),
                      truncated=meta.get("truncated"))


def _normalize(name: str, raw):
    """Grok 的 arguments 可能是 dict 也可能是 JSON 字符串,先解包再归一。"""
    decoded = raw
    if isinstance(raw, str):
        try:
            decoded = json.loads(raw)
        except json.JSONDecodeError:
            decoded = raw
    parsed = DIALECT.parse(name, decoded)
    if parsed is None:
        return CanonicalOp.TOOL_INVOKE, {
            "namespace": "grok", "name": name, "input": raw,
        }
    return parsed


def _tool(data):
    op, value = _normalize(data["name"], data["input"])
    return ToolCall(
        name=data["name"], op=op, input=value,
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
                name = str(native.get("name") or "tool")
                op, value = _normalize(name, native.get("arguments") or {})
                call = ToolCall(
                    name, op, value,
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
