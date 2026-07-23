"""Codex 会话编辑后端：delete-turn/rewrite 语义消费 codex.codec。"""
from __future__ import annotations

import glob
import json
import os
import shutil
from pathlib import Path

from ...domain.errors import ConcurrentModificationError, OperationUnsupportedError, SessionNotFoundError
from ...infrastructure.snapshots import snapshot_file
from ..base.codec import positive_turn, select_span
from ..base.editing import EditBackend, EditDocument, hash_bytes, json_size, write_jsonl
from . import native as codex_native
from .codec import CODEC, TURN_INDEX


def resolve(ref: str) -> Path:
    if Path(ref).exists():
        return Path(ref)
    hits = glob.glob(os.path.expanduser(
        f"~/.codex/sessions/*/*/*/rollout-*-{ref}.jsonl"))
    if not hits:
        raise SessionNotFoundError("codex", ref)
    return Path(hits[0])


class CodexBackend(EditBackend):
    name = "codex"

    def __init__(self, store_factory=None):
        self._store_factory = store_factory

    def load(self, ref):
        return self._load(ref, recover=True)

    def load_preview(self, ref):
        return self._load(ref, recover=False)

    def _load(self, ref, *, recover):
        path = resolve(ref)
        store = (self._store_factory(path) if self._store_factory else
                 codex_native.CodexStore.for_rollout(path))
        if recover:
            codex_native.recover_transactions(store)
        raw = path.read_bytes()
        records = [json.loads(line) for line in raw.decode().splitlines()
                   if line.strip()]
        closure = codex_native.discover_closure(path, store)
        return EditDocument(self.name, ref, path, records, hash_bytes(raw), closure)

    def apply_ops(self, doc, ops):
        notes = []
        for op in ops:
            kind = op["op"]
            if kind == "delete-turn":
                span = select_span(TURN_INDEX.turns(doc.data),
                                   positive_turn(int(op["turn"])))
                notes.extend(CODEC.delete_turn(doc, span))
            elif kind == "rewrite":
                locator = op.get("locator") or op.get("uuid") or ""
                notes.extend(CODEC.rewrite_message(doc, locator, op["text"]))
            else:
                raise OperationUnsupportedError("codex", kind)
        return notes

    def validate(self, doc):
        calls, outputs = set(), set()
        metas = 0
        for record in doc.data:
            if record.get("type") == "session_meta":
                metas += 1
            payload = record.get("payload") or {}
            subtype = payload.get("type")
            call_id = payload.get("call_id")
            if subtype in ("custom_tool_call", "function_call") and call_id:
                calls.add(call_id)
            elif subtype in ("custom_tool_call_output", "function_call_output") and call_id:
                outputs.add(call_id)
            if subtype == "message":
                role = payload.get("role")
                allowed = ({"input_text", "input_image"} if role == "user"
                           else {"output_text"} if role == "assistant" else set())
                if not allowed or any(block.get("type") not in allowed
                                      for block in payload.get("content", [])):
                    raise ValueError(f"Codex {payload.get('role')} 消息内容类型错误")
        if metas < 1:
            raise ValueError("Codex 会话缺少 session_meta")
        if calls != outputs:
            raise ValueError(f"Codex 工具调用未配对: call-only={calls-outputs}, output-only={outputs-calls}")

    def stats(self, doc):
        return {"count": len(doc.data), "size": json_size(doc.data)}

    def snapshot(self, doc, reason_code="snapshot.before_edit", extra=None):
        return snapshot_file(doc.handle, reason_code, self.name, extra)

    def restore_snapshot(self, snapshot, doc):
        shutil.copy(snapshot, doc.handle)

    def commit(self, doc):
        if isinstance(doc.context, codex_native.CodexClosure) and doc.context.pruned_ids:
            raise ValueError("该轮包含 Codex 子 Agent，会话树编辑必须使用另存为")
        if hash_bytes(doc.handle.read_bytes()) != doc.revision:
            raise ConcurrentModificationError("源会话在预览后已变化，请重新预览")
        write_jsonl(doc.handle, doc.data)
        meta = next(record.get("payload", {}) for record in doc.data
                    if record.get("type") == "session_meta")
        sid = meta["id"]
        return {"session_id": sid, "saved_as": str(doc.handle),
                "resume": f"codex resume {sid}"}

    def save_copy(self, doc):
        if not isinstance(doc.context, codex_native.CodexClosure):
            raise ValueError("Codex 原生会话树未加载")
        return codex_native.clone_tree(doc.context, doc.data)

    def discard(self, result):
        path = Path(result.get("saved_as", "."))
        store = codex_native.CodexStore.for_rollout(path)
        codex_native.discard_tree(result, store)
