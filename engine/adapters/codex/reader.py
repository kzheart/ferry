"""Codex reader: current rollout JSONL → canonical session model."""

import json
from pathlib import Path

from ...errors import AgentFormatChangedError
from ...sessions.model import (
    Block,
    ContextCompaction,
    Message,
    Session,
    ToolCall,
)
from ...sessions.reasoning import codex_summary_text
from ...sessions.tool_ops import CanonicalOp
from ..shared.media import image_from_data_url
from . import tool_calls, tool_results, topology

_SKIP_USER_PREFIX = (
    "<environment_context>",
    "<user_instructions>",
    "<ENVIRONMENT_CONTEXT>",
    "<turn_aborted>",
)


def _load_records(path: Path) -> list[dict]:
    records = []
    for line_number, line in enumerate(path.read_text().splitlines(), start=1):
        if not line.strip():
            continue
        try:
            value = json.loads(line)
        except json.JSONDecodeError as error:
            records.append(
                {
                    "type": "__resume_harness_malformed_jsonl__",
                    "line_number": line_number,
                    "error": error.msg,
                }
            )
            continue
        if isinstance(value, dict):
            records.append(value)
        else:
            records.append(
                {
                    "type": "__resume_harness_malformed_record__",
                    "line_number": line_number,
                    "error": "record is not an object",
                }
            )
    return records


def _codex_compaction(
    record: dict, ordinal: int, after_message_id: str | None
) -> ContextCompaction:
    payload = record.get("payload") or {}
    summary = payload.get("message")
    summary = summary.strip() if isinstance(summary, str) else ""
    replacement = payload.get("replacement_history")
    replacement = replacement if isinstance(replacement, list) else []
    encrypted = any(
        isinstance(item, dict)
        and item.get("type") == "compaction"
        and isinstance(item.get("encrypted_content"), str)
        and bool(item["encrypted_content"])
        for item in replacement
    )
    summary_status = "available" if summary else "protected" if encrypted else "missing"
    window_id = payload.get("window_id")
    return ContextCompaction(
        id=str(window_id or f"record:{ordinal}"),
        source="codex",
        after_message_id=after_message_id,
        event_locator=f"record:{ordinal}",
        created_at=record.get("timestamp"),
        state="completed",
        summary_status=summary_status,
        summary_text=summary,
        source_meta={
            "replacement_history_present": bool(replacement),
            "replacement_item_count": len(replacement),
            "window_number": payload.get("window_number"),
            "first_window_id": payload.get("first_window_id"),
            "previous_window_id": payload.get("previous_window_id"),
            "window_id": window_id,
        },
    )


def _read_one(path: Path, meta: dict | None = None) -> Session:
    lines = _load_records(Path(path))
    response_payload_types = {
        "message",
        "reasoning",
        "function_call",
        "function_call_output",
        "custom_tool_call",
        "custom_tool_call_output",
    }
    for line_number, record in enumerate(lines, start=1):
        record_type = record.get("type")
        if record_type in response_payload_types:
            raise AgentFormatChangedError(
                "codex",
                f"jsonl[{line_number}].type",
                "response_item with payload.type",
                record_type,
            )
    meta = meta or next(
        (
            record.get("payload") or {}
            for record in lines
            if record.get("type") == "session_meta"
        ),
        {},
    )
    ident = topology.identity(meta, path.stem)
    sess = Session(source_tool="codex", source_id=ident["id"], cwd=meta.get("cwd", ""))
    sess.root_id = ident["root_id"]
    sess.parent_id = ident["parent_id"]
    sess.forked_from_id = ident["forked_from_id"]
    sess.agent_id = ident["agent_id"]
    sess.agent_path = ident["agent_path"]
    sess.agent_type = ident["agent_type"]
    sess.agent_nickname = ident["agent_nickname"]
    sess.agent_role = ident["agent_role"]
    sess.model_provider = ident["model_provider"]
    sess.model = ident["model"]
    sess.depth = ident["depth"]
    sess.parent_association = "parent-metadata" if ident["parent_id"] else None
    for record in lines:
        if record.get("type") in {
            "__resume_harness_malformed_jsonl__",
            "__resume_harness_malformed_record__",
        }:
            sess.lose(
                "session.malformed_record",
                line_number=record["line_number"],
                error=record["error"],
            )
    pending: dict[str, ToolCall] = {}
    cur_tools: list[Block] = []  # 未落消息的工具块,附到下一条 assistant
    cur_reasoning: list[Block] = []  # 可见 reasoning 降级为 text,附到下一条 assistant

    def flush_pending_into(blocks, message_source_id: str | None = None):
        nonlocal cur_tools, cur_reasoning
        for block in cur_tools:
            if (
                block.tool
                and message_source_id
                and block.tool.source_message_id is None
            ):
                block.tool.source_message_id = message_source_id
        blocks[:0] = cur_reasoning + cur_tools
        cur_tools = []
        cur_reasoning = []

    for ordinal, record in enumerate(lines):
        record_type = record.get("type")
        if record_type == "compacted":
            after_message_id = next(
                (
                    message.source_id
                    for message in reversed(sess.messages)
                    if message.source_id
                ),
                None,
            )
            sess.context_compactions.append(
                _codex_compaction(record, ordinal, after_message_id)
            )
            continue
        if record_type == "response_item":
            p = record.get("payload") or {}
        else:
            continue
        pt = p.get("type")
        if pt == "message":
            content = p.get("content", [])
            if isinstance(content, str):
                content = [
                    {
                        "type": "input_text"
                        if p.get("role") == "user"
                        else "output_text",
                        "text": content,
                    }
                ]
            texts = [
                c.get("text", "")
                for c in content
                if isinstance(c, dict)
                and c.get("type") in ("input_text", "output_text")
            ]
            text = "\n".join(t for t in texts if t)
            image_blocks = []
            for content_index, item in enumerate(content):
                if not isinstance(item, dict):
                    continue
                if item.get("type") != "input_image":
                    continue
                image = image_from_data_url(
                    f"record:{ordinal}:image:{content_index}", item.get("image_url", "")
                )
                if image is None:
                    sess.lose("migration.unknown_block_dropped", kind="input_image")
                else:
                    image_blocks.append(Block("image", image=image))
            role = p.get("role")
            if role == "user" and text.strip().startswith(_SKIP_USER_PREFIX):
                continue
            if role == "user" and (cur_tools or cur_reasoning):
                pending_blocks = []
                source_id = f"record:{ordinal}"
                flush_pending_into(pending_blocks, source_id)
                sess.messages.append(
                    Message(
                        role="assistant",
                        blocks=pending_blocks,
                        source_id=source_id,
                        created_at=record.get("timestamp"),
                    )
                )
            if (
                not text.strip()
                and not image_blocks
                and not cur_tools
                and not cur_reasoning
            ):
                continue
            blocks = ([Block("text", text)] if text.strip() else []) + image_blocks
            if role == "assistant":
                flush_pending_into(blocks, f"record:{ordinal}")
            sess.messages.append(
                Message(
                    role=role,
                    blocks=blocks,
                    source_id=f"record:{ordinal}",
                    created_at=record.get("timestamp"),
                )
            )
        elif pt in ("custom_tool_call", "function_call"):
            if pt == "function_call":
                tc = tool_calls.parse_function_call(p)
            elif p.get("name") == "spawn_agent":
                tc = ToolCall(
                    name="spawn_agent",
                    op=CanonicalOp.AGENT_SPAWN,
                    input=tool_calls.spawn_input(
                        tool_calls.json_args(p.get("input", ""))
                    ),
                )
            else:
                tc = tool_calls.parse_custom_call(p, sess)
            tc.source_call_id = p.get("call_id")
            if tc.op == CanonicalOp.AGENT_SPAWN:
                tc.source_message_id = next(
                    (
                        message.source_id
                        for message in reversed(sess.messages)
                        if message.role in {"user", "assistant"}
                    ),
                    None,
                )
            pending[p.get("call_id")] = tc
            cur_tools.append(Block("tool", tool=tc))
        elif pt in ("custom_tool_call_output", "function_call_output"):
            tc = pending.pop(p.get("call_id"), None)
            if tc is not None:
                tc.result = tool_results.parse_result(p.get("output", ""))
                tc.source_result_id = p.get("id")
            else:
                sess.lose("session.orphan_tool_result", call_id=p.get("call_id"))
        elif pt == "reasoning":
            text = codex_summary_text(p)
            if text is not None:
                cur_reasoning.append(Block("text", text))
                sess.lose(
                    "migration.reasoning_metadata_dropped",
                    metadata_kind="encrypted_content",
                )
            else:
                sess.lose(
                    "migration.reasoning_dropped", metadata_kind="encrypted_content"
                )
        else:
            sess.lose("migration.unknown_block_dropped", kind=pt)
    if cur_tools or cur_reasoning:
        blocks = []
        flush_pending_into(blocks)
        sess.messages.append(Message(role="assistant", blocks=blocks))
    candidates = [
        compaction
        for compaction in sess.context_compactions
        if compaction.source_meta.get("replacement_history_present")
    ]
    if candidates:
        candidates[-1].source_meta["active"] = True
    return sess


def read(path: str, sessions_dir: str | Path | None = None) -> Session:
    """Read one rollout and recursively load its descendants from the same root."""
    rollout = Path(path).expanduser().resolve()
    return topology.read_tree(rollout, _read_one, sessions_dir)
