"""Transactional Pi v3 file editor."""
from __future__ import annotations

import json
import shutil
from pathlib import Path

from ...errors import ConcurrentModificationError, OperationUnsupportedError
from ...operations.snapshots import snapshot_file
from ..shared.codec import positive_turn, select_span
from ..shared.editing import EditBackend, EditDocument, hash_bytes, json_size, write_jsonl
from .codec import CODEC, TURN_INDEX
from .reader import _load


class PiBackend(EditBackend):
    name = "pi"
    operations = ("delete-turn", "rewrite", "replace-assistant-reply")

    def load(self, ref):
        path = Path(ref).resolve(strict=True)
        raw = path.read_bytes()
        records = [json.loads(line) for line in raw.decode().splitlines()
                   if line.strip()]
        return EditDocument(self.name, ref, path, records, hash_bytes(raw))

    def apply_ops(self, doc, ops):
        notes = []
        for op in ops:
            if op["op"] == "delete-turn":
                notes += CODEC.delete_turn(
                    doc, select_span(TURN_INDEX.turns(doc.data),
                                     positive_turn(op["turn"])))
            elif op["op"] == "rewrite":
                notes += CODEC.rewrite_message(
                    doc, op.get("locator") or op.get("uuid"), op["text"])
            else:
                raise OperationUnsupportedError("pi", op["op"])
        return notes

    def replace_reply(self, doc, turn, reply):
        return CODEC.replace_reply(doc, select_span(TURN_INDEX.turns(doc.data), turn), reply)

    def validate(self, doc):
        path = Path(doc.handle)
        if not doc.data or doc.data[0].get("type") != "session" \
                or doc.data[0].get("version") != 3:
            raise ValueError("Pi 会话缺少 v3 header")
        ids = [row.get("id") for row in doc.data[1:] if isinstance(row, dict)]
        if any(not value for value in ids) or len(ids) != len(set(ids)):
            raise ValueError("Pi entry id 无效或重复")
        known = set(ids)
        parents = {}
        calls, results = set(), set()
        for row in doc.data[1:]:
            parent = row.get("parentId")
            if parent is not None and parent not in known:
                raise ValueError("Pi parentId 指向不存在 entry")
            parents[row["id"]] = parent
            for field in ("targetId", "firstKeptEntryId", "fromId"):
                if field in row and row[field] not in known:
                    raise ValueError(f"Pi {field} 指向不存在 entry")
            message = row.get("message") or {}
            if message.get("role") == "assistant":
                required = {
                    "content", "api", "provider", "model", "usage",
                    "stopReason", "timestamp",
                }
                if not required.issubset(message):
                    raise ValueError("Pi assistant 缺少终态字段")
                calls.update(
                    part.get("id") for part in message.get("content", [])
                    if isinstance(part, dict) and part.get("type") == "toolCall"
                )
            elif message.get("role") == "toolResult":
                results.add(message.get("toolCallId"))
        if calls != results:
            raise ValueError("Pi 工具调用与结果未完整配对")
        for entry_id in known:
            seen, current = set(), entry_id
            while current is not None:
                if current in seen:
                    raise ValueError("Pi entry tree 存在环")
                seen.add(current)
                current = parents.get(current)

    def stats(self, doc):
        return {"count": len(doc.data), "size": json_size(doc.data)}

    def snapshot(self, doc, reason_code="snapshot.before_edit", extra=None):
        return snapshot_file(doc.handle, reason_code, self.name, extra)

    def restore_snapshot(self, snapshot, doc):
        shutil.copy(snapshot, doc.handle)

    def commit(self, doc):
        if hash_bytes(doc.handle.read_bytes()) != doc.revision:
            raise ConcurrentModificationError("源会话在预览后已变化，请重新预览")
        self.validate(doc)
        write_jsonl(doc.handle, doc.data)
        _load(doc.handle)
        return {"session_id": doc.data[0]["id"], "saved_as": str(doc.handle),
                "resume": f"pi --session {doc.handle}"}
