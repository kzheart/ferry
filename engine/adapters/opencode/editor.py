"""OpenCode 会话编辑后端：经官方 HTTP API 原地更新。"""
from __future__ import annotations

import copy
import json
from dataclasses import dataclass
from pathlib import Path

from ...errors import ConcurrentModificationError, OperationUnsupportedError
from ...operations.snapshots import snapshot_payload
from ..shared.editing import EditBackend, hash_bytes, json_size
from . import api as opencode_api
from . import reader as opencode_reader
from . import store as opencode_store
from .codec import CODEC


@dataclass(slots=True)
class OpenCodeDocument:
    tool: str
    ref: str
    handle: str
    data: dict
    revision: str
    original: dict
    tree: object


class OpenCodeBackend(EditBackend):
    name = "opencode"
    operations = ("rewrite",)

    def __init__(self, api_factory=None):
        self._api_factory = api_factory or (lambda cwd: opencode_api.OpenCodeApi(cwd))

    def load(self, ref):
        payload = opencode_store.load_native_payload(ref)
        tree = opencode_reader.read(ref)
        return self._document(ref, payload, tree)

    def load_preview(self, ref):
        payload = opencode_store.load_native_payload(ref)
        tree = opencode_reader.read_preview(ref)
        return self._document(ref, payload, tree)

    def _document(self, ref, payload, tree):
        raw = json.dumps(payload, ensure_ascii=False, sort_keys=True).encode()
        return OpenCodeDocument(
            tool=self.name,
            ref=ref,
            handle=ref,
            data=copy.deepcopy(payload),
            revision=hash_bytes(raw),
            original=copy.deepcopy(payload),
            tree=tree,
        )

    def apply_ops(self, doc, ops):
        notes = []
        for op in ops:
            kind = op["op"]
            if kind == "delete-turn":
                raise OperationUnsupportedError(
                    "opencode", "delete-turn", "inplace")
            elif kind == "rewrite":
                locator = op.get("locator") or op.get("uuid") or ""
                notes.extend(CODEC.rewrite_message(doc, locator, op["text"]))
            else:
                raise OperationUnsupportedError("opencode", kind)
        return notes

    def validate(self, doc):
        sid = doc.data.get("info", {}).get("id")
        message_ids = set()
        for message in doc.data.get("messages", []):
            info = message.get("info") or {}
            mid = info.get("id")
            if not mid or mid in message_ids:
                raise ValueError("OpenCode message id 缺失或重复")
            message_ids.add(mid)
            if info.get("sessionID") != sid:
                raise ValueError("OpenCode message.sessionID 不一致")
            if (info.get("role") == "assistant" and not info.get("finish")
                    and not info.get("error")):
                raise ValueError("OpenCode assistant 消息缺少 finish/error 终态")
            for part in message.get("parts", []):
                if part.get("messageID") != mid or part.get("sessionID") != sid:
                    raise ValueError("OpenCode part 外键不一致")

    def stats(self, doc):
        return {"count": len(doc.data.get("messages", [])),
                "size": json_size(doc.data)}

    @staticmethod
    def _part_map(payload):
        return {part["id"]: (message["info"]["id"], part)
                for message in payload.get("messages", [])
                for part in message.get("parts", []) if part.get("id")}

    def snapshot(self, doc, reason_code="snapshot.before_edit", extra=None):
        return snapshot_payload(
            doc.ref, json.dumps(doc.original, ensure_ascii=False) + "\n",
            reason_code, self.name, doc.ref, extra,
        )

    def restore_snapshot(self, snapshot, doc):
        original = json.loads(Path(snapshot).read_text())
        current = opencode_store.export_session(doc.ref)
        current_parts = self._part_map(current)
        original_parts = self._part_map(original)
        if set(current_parts) != set(original_parts):
            raise RuntimeError("OpenCode 快照包含消息增删，当前官方 API 无法安全原地恢复")
        cwd = original.get("info", {}).get("directory") or "."
        applied = []
        with self._api_factory(cwd) as client:
            try:
                for part_id, (message_id, part) in original_parts.items():
                    current_part = current_parts[part_id][1]
                    if current_part != part:
                        client.patch_part(doc.ref, message_id, part)
                        applied.append((message_id, current_part))
            except Exception:
                rollback_errors = []
                for message_id, current_part in reversed(applied):
                    try:
                        client.patch_part(doc.ref, message_id, current_part)
                    except Exception as error:
                        rollback_errors.append(str(error))
                if rollback_errors:
                    raise RuntimeError("OpenCode 快照恢复失败且补偿回滚不完整: " +
                                       "; ".join(rollback_errors))
                raise
        restored = self._part_map(opencode_store.export_session(doc.ref))
        if any(restored.get(part_id, (None, None))[1] != part
               for part_id, (_, part) in original_parts.items()):
            raise RuntimeError("OpenCode 快照恢复后静态校验失败")

    def commit(self, doc):
        fresh = opencode_store.export_session(doc.ref)
        fresh_raw = json.dumps(fresh, ensure_ascii=False, sort_keys=True).encode()
        if hash_bytes(fresh_raw) != doc.revision:
            raise ConcurrentModificationError("源会话在预览后已变化，请重新预览")
        original = doc.original
        before = self._part_map(original)
        after = self._part_map(doc.data)
        before_messages = {m["info"].get("id") for m in original.get("messages", [])}
        after_messages = {m["info"].get("id") for m in doc.data.get("messages", [])}
        if before_messages != after_messages or set(before) != set(after):
            raise ValueError("OpenCode 当前不支持安全原地删除整轮")
        changed = [(part_id, after[part_id][0], before[part_id][1], after[part_id][1])
                   for part_id in before if before[part_id][1] != after[part_id][1]]
        cwd = doc.data.get("info", {}).get("directory") or "."
        applied = []
        with self._api_factory(cwd) as client:
            if not client.supports_part_patch():
                raise RuntimeError("当前 OpenCode server 不支持官方 part 更新 API")
            if hasattr(client, "assert_idle"):
                client.assert_idle(doc.ref)
            try:
                for part_id, message_id, old_part, new_part in changed:
                    client.patch_part(doc.ref, message_id, new_part)
                    applied.append((message_id, old_part))
            except Exception:
                rollback_errors = []
                for message_id, old_part in reversed(applied):
                    try:
                        client.patch_part(doc.ref, message_id, old_part)
                    except Exception as error:
                        rollback_errors.append(str(error))
                if rollback_errors:
                    raise RuntimeError("OpenCode API 更新失败且补偿回滚不完整: " +
                                       "; ".join(rollback_errors))
                raise
        return {"session_id": doc.ref, "saved_as": str(opencode_store.DB_PATH),
                "resume": f"cd {cwd} && opencode -s {doc.ref}",
                "updated_parts": len(changed)}

    def saved_revision(self, result, doc):
        payload = opencode_store.export_session(result["session_id"])
        raw = json.dumps(payload, ensure_ascii=False, sort_keys=True).encode()
        return hash_bytes(raw)
