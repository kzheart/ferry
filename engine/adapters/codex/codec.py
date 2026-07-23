"""Codex 原生 rollout 的唯一轮次解析与编辑编解码。

轮次定义：非环境上下文前缀、正文非空的用户 message 到下一条之前。
"""
from __future__ import annotations

import json
import secrets
from datetime import datetime, timezone

from ...domain.edit import TextItem
from ...domain.events import event
from ...domain.errors import LocatorStaleError, OperationUnsupportedError
from ..base.editing import (
    is_spawn_name,
    reject_replacement_spawn,
    reject_target_spawn,
    replace_at_first,
)
from ..base.codec import TurnSpan
from . import native as codex_native

_SKIP_USER_PREFIX = ("<environment_context>", "<user_instructions>",
                     "<ENVIRONMENT_CONTEXT>", "<turn_aborted>")


def _message_text(payload) -> str:
    return "\n".join(str(block.get("text", "")) for block in
                     payload.get("content", []) if isinstance(block, dict))


def _is_reply_record(record) -> bool:
    if record.get("type") != "response_item":
        return False
    payload = record.get("payload") or {}
    subtype = payload.get("type")
    return ((subtype == "message" and payload.get("role") == "assistant") or
            subtype in {"custom_tool_call", "function_call",
                        "custom_tool_call_output", "function_call_output"})


def _is_spawn(record) -> bool:
    payload = record.get("payload") or {}
    return payload.get("type") in {"custom_tool_call", "function_call"} and \
        is_spawn_name(payload.get("name"))


class CodexTurnIndex:
    def visible_messages(self, records) -> list[tuple[int, dict]]:
        out = []
        for index, record in enumerate(records):
            payload = record.get("payload") or {}
            if record.get("type") != "response_item" or payload.get("type") != "message":
                continue
            text = _message_text(payload)
            if payload.get("role") == "user" and text.strip().startswith(_SKIP_USER_PREFIX):
                continue
            if text.strip():
                out.append((index, record))
        return out

    def turns(self, records) -> list[TurnSpan]:
        starts = [index for index, record in self.visible_messages(records)
                  if record["payload"].get("role") == "user"]
        spans = []
        for ordinal, start in enumerate(starts, 1):
            end = starts[ordinal] if ordinal < len(starts) else len(records)
            spans.append(TurnSpan(ordinal, f"record:{start}", start, end))
        return spans


class CodexEditCodec:
    def replace_reply(self, doc, span: TurnSpan, reply) -> list[str]:
        records = doc.data
        old = records[span.start + 1:span.end]
        reject_replacement_spawn(reply)
        if any(_is_spawn(record) for record in old):
            reject_target_spawn("codex")
        now = datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")
        compiled = []
        for item in reply.items:
            if isinstance(item, TextItem):
                compiled.append({"timestamp": now, "type": "response_item", "payload": {
                    "type": "message", "id": "msg_" + secrets.token_hex(12),
                    "role": "assistant", "content": [{"type": "output_text", "text": item.text}],
                    "phase": "final_answer"}})
            else:
                call_id = "call_" + secrets.token_urlsafe(18)[:24]
                arguments = (json.dumps(item.input, ensure_ascii=False)
                             if isinstance(item.input, dict) else item.input)
                compiled.extend((
                    {"timestamp": now, "type": "response_item", "payload": {
                        "type": "function_call", "id": "fc_" + secrets.token_hex(12),
                        "name": item.name, "arguments": arguments, "call_id": call_id,
                        "status": "completed"}},
                    {"timestamp": now, "type": "response_item", "payload": {
                        "type": "function_call_output", "id": "fco_" + secrets.token_hex(12),
                        "call_id": call_id, "output": item.output}},
                ))
        records[span.start + 1:span.end] = replace_at_first(
            old, _is_reply_record, compiled)
        return [event("edit.reply_replaced", turn=span.ordinal,
                      items=len(reply.items))]

    def delete_turn(self, doc, span: TurnSpan) -> list[str]:
        removed = doc.data[span.start:span.end]
        del doc.data[span.start:span.end]
        pruned = set()
        if isinstance(doc.context, codex_native.CodexClosure):
            pruned = codex_native.prune_referenced_subtrees(doc.context, removed)
        return [event("edit.turn_deleted", turn=span.ordinal,
                      pruned_children=len(pruned))]

    def rewrite_message(self, doc, locator: str, text: str) -> list[str]:
        record = None
        if str(locator).startswith("record:"):
            try:
                ordinal = int(str(locator).removeprefix("record:"))
                candidate = doc.data[ordinal]
                payload = candidate.get("payload") or {}
                if candidate.get("type") == "response_item" and payload.get("type") == "message":
                    record = candidate
            except (ValueError, IndexError):
                pass
        elif str(locator).startswith("index:"):
            try:
                wanted = int(str(locator).removeprefix("index:"))
                record = TURN_INDEX.visible_messages(doc.data)[wanted][1]
            except (ValueError, IndexError):
                pass
        if record is None:
            raise LocatorStaleError("Codex 消息定位符已失效，请刷新会话",
                                    {"locator": locator})
        payload = record["payload"]
        role = payload.get("role")
        if role not in {"user", "assistant"}:
            raise OperationUnsupportedError("codex", "rewrite", str(role))
        content = payload.get("content") or []
        text_types = {"input_text", "output_text"}
        first = next((index for index, item in enumerate(content)
                      if item.get("type") in text_types), None)
        if first is None:
            raise OperationUnsupportedError("codex", "rewrite", "no-text")
        rewritten = [item for item in content if item.get("type") not in text_types]
        rewritten.insert(first, {"type": "input_text" if role == "user" else "output_text",
                                 "text": text})
        payload["content"] = rewritten
        return [event("edit.message_rewritten", count=1)]


TURN_INDEX = CodexTurnIndex()
CODEC = CodexEditCodec()
