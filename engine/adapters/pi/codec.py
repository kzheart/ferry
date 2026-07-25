"""Pi v3 linear active-branch edit codec."""
from __future__ import annotations

import copy
import time
import uuid

from ...errors import LocatorStaleError, OperationUnsupportedError
from ...events import event
from ...operations.types import TextItem
from ..shared.codec import TurnSpan
from ..shared.editing import reject_replacement_spawn


def active_indexes(records) -> list[int]:
    valid = [(index, row) for index, row in enumerate(records[1:], 1)
             if isinstance(row, dict) and isinstance(row.get("id"), str)
             and "parentId" in row]
    by_id = {row["id"]: (index, row) for index, row in valid}
    out, seen = [], set()
    current = valid[-1] if valid else None
    while current and current[1]["id"] not in seen:
        out.append(current[0])
        seen.add(current[1]["id"])
        current = by_id.get(current[1].get("parentId"))
    return list(reversed(out))


class PiTurnIndex:
    def visible_messages(self, records):
        return [(index, records[index]) for index in active_indexes(records)
                if records[index].get("type") == "message"
                and (records[index].get("message") or {}).get("role")
                in {"user", "assistant", "bashExecution"}]

    def turns(self, records):
        active = active_indexes(records)
        starts = [index for index in active
                  if records[index].get("type") == "message"
                  and (records[index].get("message") or {}).get("role") == "user"]
        return [TurnSpan(
            ordinal + 1, records[start]["id"], start,
            starts[ordinal + 1] if ordinal + 1 < len(starts)
            else (active[-1] + 1 if active else len(records)),
        ) for ordinal, start in enumerate(starts)]


def _entry(parent, message):
    stamp = time.strftime("%Y-%m-%dT%H:%M:%S.000Z", time.gmtime())
    return {"type": "message", "id": uuid.uuid4().hex[:12],
            "parentId": parent, "timestamp": stamp, "message": message}


class PiEditCodec:
    def replace_reply(self, doc, span, reply):
        reject_replacement_spawn(reply)
        user = doc.data[span.start]
        active = active_indexes(doc.data)
        start_pos = active.index(span.start)
        end_pos = active.index(span.end) if span.end in active else len(active)
        target_indexes = set(active[start_pos + 1:end_pos])
        old = [doc.data[index] for index in sorted(target_indexes)]
        removed = {row.get("id") for row in old if isinstance(row, dict)}
        parent, compiled, content = user["id"], [], []
        now = int(time.time() * 1000)
        for item in reply.items:
            if isinstance(item, TextItem):
                content.append({"type": "text", "text": item.text})
                continue
            call_id = "call_" + uuid.uuid4().hex[:16]
            content.append({"type": "toolCall", "id": call_id,
                            "name": item.name,
                            "arguments": copy.deepcopy(item.input)})
            assistant = _entry(parent, {
                "role": "assistant", "content": content,
                "api": "ferry", "provider": "ferry", "model": "migrated",
                "usage": {"input": 0, "output": 0, "cacheRead": 0,
                          "cacheWrite": 0, "totalTokens": 0,
                          "cost": {"input": 0, "output": 0, "cacheRead": 0,
                                   "cacheWrite": 0, "total": 0}},
                "stopReason": "toolUse", "timestamp": now,
            })
            result = _entry(assistant["id"], {
                "role": "toolResult", "toolCallId": call_id,
                "toolName": item.name,
                "content": [{"type": "text", "text": item.output}],
                "isError": False, "timestamp": now,
            })
            compiled.extend((assistant, result))
            parent, content = result["id"], []
        if content:
            assistant = _entry(parent, {
                "role": "assistant", "content": content,
                "api": "ferry", "provider": "ferry", "model": "migrated",
                "usage": {"input": 0, "output": 0, "cacheRead": 0,
                          "cacheWrite": 0, "totalTokens": 0,
                          "cost": {"input": 0, "output": 0, "cacheRead": 0,
                                   "cacheWrite": 0, "total": 0}},
                "stopReason": "stop", "timestamp": now,
            })
            compiled.append(assistant)
            parent = assistant["id"]
        insert_at = min(target_indexes, default=span.start + 1)
        rebuilt = []
        for index, row in enumerate(doc.data):
            if index == insert_at:
                rebuilt.extend(compiled)
            if index not in target_indexes:
                rebuilt.append(row)
        if insert_at >= len(doc.data):
            rebuilt.extend(compiled)
        doc.data = rebuilt
        for row in doc.data:
            if isinstance(row, dict) and row.get("parentId") in removed:
                row["parentId"] = parent
        return [event("edit.reply_replaced", turn=span.ordinal,
                      items=len(reply.items))]

    def delete_turn(self, doc, span):
        active = active_indexes(doc.data)
        start_pos = active.index(span.start)
        end_pos = active.index(span.end) if span.end in active else len(active)
        target_indexes = set(active[start_pos:end_pos])
        removed_rows = [doc.data[index] for index in target_indexes]
        removed = {row.get("id") for row in removed_rows if isinstance(row, dict)}
        parent_by_id = {
            row.get("id"): row.get("parentId") for row in removed_rows
            if isinstance(row, dict)
        }
        doc.data = [row for index, row in enumerate(doc.data)
                    if index not in target_indexes]
        def surviving(value):
            seen = set()
            while value in removed and value not in seen:
                seen.add(value)
                value = parent_by_id.get(value)
            return value
        for row in doc.data[1:]:
            if not isinstance(row, dict):
                continue
            if row.get("parentId") in removed:
                row["parentId"] = surviving(row.get("parentId"))
            for field in ("targetId", "firstKeptEntryId", "fromId"):
                if row.get(field) in removed:
                    replacement = surviving(row.get(field))
                    if replacement is None:
                        row.pop(field, None)
                    else:
                        row[field] = replacement
        return [event("edit.turn_deleted", turn=span.ordinal)]

    def rewrite_message(self, doc, locator, text):
        row = next((row for row in doc.data
                    if isinstance(row, dict) and row.get("id") == locator), None)
        if row is None:
            raise LocatorStaleError(params={"locator": locator})
        message = row.get("message") or {}
        if message.get("role") not in {"user", "assistant"}:
            raise OperationUnsupportedError("pi", "rewrite", str(message.get("role")))
        content = message.get("content")
        if isinstance(content, str):
            message["content"] = text
        elif isinstance(content, list):
            index = next((i for i, part in enumerate(content)
                          if part.get("type") == "text"), None)
            if index is None:
                raise OperationUnsupportedError("pi", "rewrite", "no-text")
            message["content"][index] = {"type": "text", "text": text}
        else:
            raise OperationUnsupportedError("pi", "rewrite", "no-text")
        return [event("edit.message_rewritten", count=1)]


TURN_INDEX = PiTurnIndex()
CODEC = PiEditCodec()
