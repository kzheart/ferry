"""Claude 原生会话的唯一轮次解析与编辑编解码。

reader DTO、delete-turn、rewrite、replace-reply 全部消费本模块的
``TURN_INDEX``；轮次定义：非 sidechain、非 isMeta、非 tool_result
载体、且含可见内容的用户消息，到下一条这样的消息之前。
"""
from __future__ import annotations

import copy
import secrets
import uuid as uuid_mod
from datetime import datetime, timezone

from ...operations.types import TextItem
from ...events import event
from ...errors import LocatorStaleError, OperationUnsupportedError
from ...sessions.reasoning import visible_text
from ..shared.editing import (
    is_spawn_name,
    reject_replacement_spawn,
    reject_target_spawn,
    replace_at_first,
)
from ..shared.codec import TurnSpan
from .editing import relink


def _visible_user_content(content) -> bool:
    if isinstance(content, str):
        return True
    return isinstance(content, list) and any(
        isinstance(item, dict) and (
            item.get("type") in {"text", "tool_use"} or
            item.get("type") == "thinking" and
            visible_text(item.get("thinking")) is not None)
        for item in content)


def _is_tool_carrier(content) -> bool:
    return isinstance(content, list) and any(
        isinstance(item, dict) and item.get("type") == "tool_result"
        for item in content)


def _is_reply_record(record) -> bool:
    if record.get("isSidechain"):
        return False
    if record.get("type") == "assistant":
        return True
    content = (record.get("message") or {}).get("content")
    return record.get("type") == "user" and _is_tool_carrier(content)


class ClaudeTurnIndex:
    def visible_messages(self, records) -> list[tuple[int, dict]]:
        out = []
        for index, record in enumerate(records):
            if (record.get("isSidechain") or record.get("isMeta") or
                    record.get("type") not in {"user", "assistant"}):
                continue
            content = (record.get("message") or {}).get("content")
            if record.get("type") == "user" and _is_tool_carrier(content):
                continue
            out.append((index, record))
        return out

    def turns(self, records) -> list[TurnSpan]:
        starts = []
        for index, record in enumerate(records):
            if (record.get("type") != "user" or record.get("isSidechain") or
                    record.get("isMeta")):
                continue
            content = (record.get("message") or {}).get("content")
            if not _is_tool_carrier(content) and _visible_user_content(content):
                starts.append(index)
        spans = []
        for ordinal, start in enumerate(starts, 1):
            end = starts[ordinal] if ordinal < len(starts) else len(records)
            locator = str(records[start].get("uuid") or f"record:{start}")
            spans.append(TurnSpan(ordinal, locator, start, end))
        return spans


class ClaudeEditCodec:
    def _record(self, template, record_type, parent, content, stop_reason=None):
        record = {key: copy.deepcopy(value) for key, value in template.items()
                  if key not in {"uuid", "parentUuid", "promptId", "type", "message",
                                 "toolUseResult", "timestamp"}}
        record.update({"uuid": str(uuid_mod.uuid4()), "parentUuid": parent,
                       "type": record_type, "isSidechain": False})
        message = {"role": "assistant" if record_type == "assistant" else "user",
                   "content": content}
        if record_type == "assistant":
            message.update(type="message", stop_reason=stop_reason or "end_turn")
        record["message"] = message
        record["timestamp"] = datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")
        return record

    def replace_reply(self, doc, span: TurnSpan, reply) -> list[str]:
        records = doc.data
        old = records[span.start + 1:span.end]
        reject_replacement_spawn(reply)
        if any(is_spawn_name(item.get("name")) for record in old
               if _is_reply_record(record)
               for item in ((record.get("message") or {}).get("content") or [])
               if isinstance(item, dict) and item.get("type") == "tool_use"):
            reject_target_spawn("claude")
        removed_ids = {record.get("uuid") for record in old
                       if _is_reply_record(record) and record.get("uuid")}
        user = records[span.start]
        template = next((record for record in old
                         if record.get("type") == "assistant" and
                         not record.get("isSidechain")), user)
        parent = user.get("uuid")
        compiled = []
        content = []
        for item in reply.items:
            if isinstance(item, TextItem):
                content.append({"type": "text", "text": item.text})
            else:
                call_id = "toolu_" + secrets.token_urlsafe(18)[:24]
                content.append({
                    "type": "tool_use", "id": call_id, "name": item.name,
                    "input": copy.deepcopy(item.input),
                })
                call = self._record(template, "assistant", parent, content, "tool_use")
                result = self._record(template, "user", call["uuid"], [{
                    "type": "tool_result", "tool_use_id": call_id,
                    "content": item.output,
                }])
                compiled.extend((call, result))
                parent = result["uuid"]
                content = []
        if content:
            final = self._record(template, "assistant", parent, content)
            compiled.append(final)
            parent = final["uuid"]
        records[span.start + 1:span.end] = replace_at_first(
            old, _is_reply_record, compiled)
        for record in records[span.start + 1:]:
            if record.get("parentUuid") in removed_ids:
                record["parentUuid"] = parent
        return [event("edit.reply_replaced", turn=span.ordinal,
                      items=len(reply.items))]

    def delete_turn(self, doc, span: TurnSpan) -> list[str]:
        records = doc.data
        removed = records[span.start:span.end]
        removed_uuids = {r["uuid"] for r in removed if "uuid" in r}
        kept = records[:span.start] + records[span.end:]
        relink(kept, removed_uuids)
        doc.data = kept
        return [event("edit.turn_deleted", turn=span.ordinal)]

    def rewrite_message(self, doc, locator: str, text: str) -> list[str]:
        record = next((item for item in doc.data
                       if item.get("uuid") == locator), None)
        if record is None:
            raise LocatorStaleError("Claude 消息定位符已失效，请刷新会话",
                                    {"locator": locator})
        message = record.get("message") or {}
        role = message.get("role") or record.get("type")
        if role not in {"user", "assistant"}:
            raise OperationUnsupportedError("claude", "rewrite", str(role))
        content = message.get("content")
        if isinstance(content, str):
            message["content"] = text
        elif isinstance(content, list):
            first = next((index for index, item in enumerate(content)
                          if item.get("type") == "text"), None)
            if first is None:
                raise OperationUnsupportedError("claude", "rewrite", "no-text")
            rewritten = [item for item in content if item.get("type") != "text"]
            rewritten.insert(first, {"type": "text", "text": text})
            message["content"] = rewritten
        else:
            raise OperationUnsupportedError("claude", "rewrite", "no-text")
        return [event("edit.message_rewritten", count=1)]


TURN_INDEX = ClaudeTurnIndex()
CODEC = ClaudeEditCodec()
