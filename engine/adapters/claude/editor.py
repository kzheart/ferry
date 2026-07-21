"""Claude 会话编辑后端：delete-turn/rewrite 语义消费 claude.codec。"""
from __future__ import annotations

import shutil
import uuid
from pathlib import Path

from ...domain.errors import ConcurrentModificationError, OperationUnsupportedError
from ..base.codec import positive_turn, select_span
from ..base.editing import EditBackend, EditDocument, hash_bytes, json_size
from . import editing as claude_edit
from .codec import CODEC, TURN_INDEX


class ClaudeBackend(EditBackend):
    name = "claude"

    def load(self, ref):
        path = claude_edit.resolve(ref)
        raw = path.read_bytes()
        return EditDocument(self.name, ref, path, claude_edit.load(path),
                            hash_bytes(raw))

    def apply_ops(self, doc, ops):
        notes = []
        for op in ops:
            kind = op["op"]
            if kind == "delete-turn":
                span = select_span(TURN_INDEX.turns(doc.data),
                                   positive_turn(op["turn"]))
                notes.extend(CODEC.delete_turn(doc, span))
            elif kind == "rewrite":
                locator = op.get("locator") or op.get("uuid")
                notes.extend(CODEC.rewrite_message(doc, locator, op["text"]))
            else:
                raise OperationUnsupportedError("claude", kind)
        return notes

    def validate(self, doc):
        claude_edit.check_invariants(doc.data)

    def stats(self, doc):
        return {"count": len(doc.data), "size": json_size(doc.data)}

    def snapshot(self, doc, reason_code="snapshot.before_edit", extra=None):
        return claude_edit.backup(doc.handle, reason_code=reason_code,
                                  tool=self.name, extra=extra)

    def restore_snapshot(self, snapshot, doc):
        shutil.copy(snapshot, doc.handle)

    def commit(self, doc):
        if hash_bytes(doc.handle.read_bytes()) != doc.revision:
            raise ConcurrentModificationError("源会话在预览后已变化，请重新预览")
        claude_edit.save(doc.handle, doc.data)
        cwd = next((r.get("cwd") for r in doc.data if r.get("cwd")), ".")
        return {"session_id": doc.handle.stem, "saved_as": str(doc.handle),
                "resume": f"cd {cwd} && claude --resume {doc.handle.stem}"}

    def save_copy(self, doc):
        new_id = str(uuid.uuid4())
        for record in doc.data:
            if "sessionId" in record:
                record["sessionId"] = new_id
        path = doc.handle.with_name(f"{new_id}.jsonl")
        claude_edit.save(path, doc.data)
        cwd = next((r.get("cwd") for r in doc.data if r.get("cwd")), ".")
        return {"session_id": new_id, "saved_as": str(path),
                "resume": f"cd {cwd} && claude --resume {new_id}"}

    def discard(self, result):
        Path(result["saved_as"]).unlink(missing_ok=True)
