"""可扩展的原生会话编辑后端。

通用层只编排 preview/apply/save-as；每个后端负责原生记录闭包、校验与落盘。
新增 coding agent 时实现 ``EditBackend`` 并注册到 ``BACKENDS`` 即可。
"""
from __future__ import annotations

import hashlib
import json
import os
import shutil
import time
import uuid
from abc import ABC, abstractmethod
from dataclasses import dataclass
from pathlib import Path

from . import codex_native, convert, edit as claude_edit, opencode_api, rw_opencode


@dataclass
class EditDocument:
    tool: str
    ref: str
    handle: object
    data: object
    revision: str
    context: object | None = None


class EditBackend(ABC):
    """Agent 原生编辑契约；API 与 UI 不得依赖具体存储格式。"""

    name: str
    inplace = True
    save_as = True
    probe = True

    def capabilities(self) -> dict:
        operations = ["delete-turn", "truncate", "rewrite"]
        return {"tool": self.name, "operations": operations,
            "inplace": self.inplace, "save_as": self.save_as,
            "probe": self.probe,
            "operation_modes": {op: (["inplace"] if self.inplace else []) +
                                (["saveas"] if self.save_as else [])
                                for op in operations}}

    def supports_mode(self, ops: list[dict], save_as: bool) -> bool:
        mode = "saveas" if save_as else "inplace"
        modes = self.capabilities().get("operation_modes", {})
        return all(mode in modes.get(op.get("op"), []) for op in ops)

    @abstractmethod
    def load(self, ref: str) -> EditDocument: ...

    @abstractmethod
    def apply_ops(self, doc: EditDocument, ops: list[dict]) -> list[str]: ...

    @abstractmethod
    def validate(self, doc: EditDocument) -> None: ...

    @abstractmethod
    def stats(self, doc: EditDocument) -> dict: ...

    @abstractmethod
    def commit(self, doc: EditDocument) -> dict: ...

    @abstractmethod
    def save_copy(self, doc: EditDocument) -> dict: ...

    def snapshot(self, doc: EditDocument, reason="会话编辑前自动") -> Path | None:
        return None

    def restore_snapshot(self, snapshot: Path, doc: EditDocument) -> None:
        raise NotImplementedError

    def discard(self, result: dict) -> None:
        pass


def _hash_bytes(data: bytes) -> str:
    return "sha256:" + hashlib.sha256(data).hexdigest()


def _json_size(value) -> int:
    return len(json.dumps(value, ensure_ascii=False).encode())


def _shorten(text: str, threshold: int) -> str:
    if len(text) <= threshold:
        return text
    keep = max(1, threshold // 2)
    removed = max(0, len(text) - keep * 2)
    return text[:keep] + f"\n\n[...已裁剪 {removed} 字符...]\n\n" + text[-keep:]


def _write_jsonl(path: Path, records: list[dict]) -> None:
    tmp = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    tmp.write_text("\n".join(json.dumps(r, ensure_ascii=False) for r in records) + "\n")
    os.replace(tmp, path)


class ClaudeBackend(EditBackend):
    name = "claude"

    def load(self, ref):
        path = claude_edit.resolve(ref)
        raw = path.read_bytes()
        return EditDocument(self.name, ref, path, claude_edit.load(path),
                            _hash_bytes(raw))

    def apply_ops(self, doc, ops):
        notes = []
        args = type("Args", (), {})
        for op in ops:
            kind = op["op"]
            if kind == "delete-turn":
                a = args(); a.turn = op["turn"]
                doc.data = claude_edit.op_delete_turn(doc.data, a)
                notes.append(f"删除第 {a.turn} 轮")
            elif kind == "truncate":
                a = args(); a.threshold = op.get("threshold", 4096)
                doc.data = claude_edit.op_truncate(doc.data, a)
                notes.append(f"裁剪超过 {a.threshold} 字符的工具输出")
            elif kind == "rewrite":
                a = args(); a.uuid = op.get("locator") or op.get("uuid")
                a.text = op["text"]
                doc.data = claude_edit.op_rewrite(doc.data, a)
                notes.append("改写 1 条消息")
            else:
                raise ValueError(f"未知操作: {kind}")
        return notes

    def validate(self, doc):
        claude_edit.check_invariants(doc.data)

    def stats(self, doc):
        return {"count": len(doc.data), "size": _json_size(doc.data)}

    def snapshot(self, doc, reason="会话编辑前自动"):
        return claude_edit.backup(doc.handle, reason=reason, tool=self.name)

    def restore_snapshot(self, snapshot, doc):
        shutil.copy(snapshot, doc.handle)

    def commit(self, doc):
        if _hash_bytes(doc.handle.read_bytes()) != doc.revision:
            raise RuntimeError("源会话在预览后已变化，请重新预览")
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


class CodexBackend(EditBackend):
    name = "codex"
    save_as = True

    def __init__(self, store_factory=None):
        self._store_factory = store_factory

    def load(self, ref):
        path = Path(convert.resolve_ref("codex", ref))
        raw = path.read_bytes()
        records = [json.loads(line) for line in raw.decode().splitlines()
                   if line.strip()]
        store = (self._store_factory(path) if self._store_factory else
                 codex_native.CodexStore.for_rollout(path))
        codex_native.recover_transactions(store)
        closure = codex_native.discover_closure(path, store)
        return EditDocument(self.name, ref, path, records, _hash_bytes(raw), closure)

    @staticmethod
    def _message_records(records):
        out = []
        skipped = ("<environment_context>", "<user_instructions>",
                   "<ENVIRONMENT_CONTEXT>", "<turn_aborted>")
        for index, record in enumerate(records):
            payload = record.get("payload") or {}
            if record.get("type") != "response_item" or payload.get("type") != "message":
                continue
            text = "\n".join(str(block.get("text", "")) for block in
                             payload.get("content", []) if isinstance(block, dict))
            if payload.get("role") == "user" and text.strip().startswith(skipped):
                continue
            if text.strip():
                out.append((index, record))
        return out

    @classmethod
    def _turn_starts(cls, records):
        return [index for index, record in cls._message_records(records)
                if record["payload"].get("role") == "user"]

    def apply_ops(self, doc, ops):
        notes = []
        for op in ops:
            kind = op["op"]
            if kind == "delete-turn":
                starts = self._turn_starts(doc.data)
                turn = int(op["turn"])
                if not 1 <= turn <= len(starts):
                    raise ValueError(f"轮次超界: 共 {len(starts)} 轮")
                lo = starts[turn - 1]
                hi = starts[turn] if turn < len(starts) else len(doc.data)
                removed = doc.data[lo:hi]
                del doc.data[lo:hi]
                pruned = set()
                if isinstance(doc.context, codex_native.CodexClosure):
                    pruned = codex_native.prune_referenced_subtrees(doc.context, removed)
                detail = f"，同时移除 {len(pruned)} 个子会话" if pruned else ""
                notes.append(f"删除第 {turn} 轮{detail}")
            elif kind == "truncate":
                threshold = int(op.get("threshold", 4096))
                changed = 0
                for record in doc.data:
                    payload = record.get("payload") or {}
                    if payload.get("type") not in ("custom_tool_call_output",
                                                    "function_call_output"):
                        continue
                    output = payload.get("output")
                    if isinstance(output, str) and len(output) > threshold:
                        payload["output"] = _shorten(output, threshold)
                        changed += 1
                notes.append(f"裁剪了 {changed} 条超长工具输出")
            elif kind == "rewrite":
                locator = op.get("locator") or op.get("uuid") or ""
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
                        record = self._message_records(doc.data)[wanted][1]
                    except (ValueError, IndexError):
                        pass
                if record is None:
                    raise ValueError("Codex 消息定位符已失效，请刷新会话")
                payload = record["payload"]
                if payload.get("role") != "user":
                    raise ValueError("目前只允许改写用户消息")
                payload["content"] = [{"type": "input_text", "text": op["text"]}]
                notes.append("改写 1 条消息")
            else:
                raise ValueError(f"未知操作: {kind}")
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
                expected = "input_text" if payload.get("role") == "user" else "output_text"
                if any(block.get("type") != expected for block in payload.get("content", [])):
                    raise ValueError(f"Codex {payload.get('role')} 消息内容类型错误")
        if metas < 1:
            raise ValueError("Codex 会话缺少 session_meta")
        if calls != outputs:
            raise ValueError(f"Codex 工具调用未配对: call-only={calls-outputs}, output-only={outputs-calls}")

    def stats(self, doc):
        return {"count": len(doc.data), "size": _json_size(doc.data)}

    def snapshot(self, doc, reason="会话编辑前自动"):
        return claude_edit.backup(doc.handle, reason=reason, tool=self.name)

    def restore_snapshot(self, snapshot, doc):
        shutil.copy(snapshot, doc.handle)

    def commit(self, doc):
        if isinstance(doc.context, codex_native.CodexClosure) and doc.context.pruned_ids:
            raise ValueError("该轮包含 Codex 子 Agent，会话树编辑必须使用另存为")
        if _hash_bytes(doc.handle.read_bytes()) != doc.revision:
            raise RuntimeError("源会话在预览后已变化，请重新预览")
        _write_jsonl(doc.handle, doc.data)
        meta = next(record.get("payload", {}) for record in doc.data
                    if record.get("type") == "session_meta")
        sid = meta.get("id") or meta.get("session_id") or doc.handle.stem
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


class OpenCodeBackend(EditBackend):
    name = "opencode"
    inplace = True

    def __init__(self, api_factory=None):
        self._api_factory = api_factory or (lambda cwd: opencode_api.OpenCodeApi(cwd))

    def capabilities(self):
        result = super().capabilities()
        result["operation_modes"] = {
            "rewrite": ["inplace", "saveas"],
            "truncate": ["inplace", "saveas"],
            # OpenCode 1.18.3 没有批量事务 API，整轮删除只能另存。
            "delete-turn": ["saveas"],
        }
        return result

    def load(self, ref):
        tree = rw_opencode.read(ref)
        payload = tree.meta.get("opencode_export")
        if not isinstance(payload, dict):
            payload = rw_opencode._oc_export(ref)
        raw = json.dumps(payload, ensure_ascii=False, sort_keys=True).encode()
        return EditDocument(self.name, ref, ref, rw_opencode._clone(payload), _hash_bytes(raw),
                            {"original": rw_opencode._clone(payload), "tree": tree})

    @staticmethod
    def _messages(payload):
        return [m for m in payload.get("messages", []) if any(
            p.get("type") in ("text", "tool") for p in m.get("parts", []))]

    def apply_ops(self, doc, ops):
        notes = []
        for op in ops:
            kind = op["op"]
            messages = self._messages(doc.data)
            if kind == "delete-turn":
                users = [m for m in messages if m["info"].get("role") == "user"]
                turn = int(op["turn"])
                if not 1 <= turn <= len(users):
                    raise ValueError(f"轮次超界: 共 {len(users)} 轮")
                user_id = users[turn - 1]["info"].get("id")
                remove_ids = {user_id}
                remove_ids.update(m["info"].get("id") for m in messages
                                  if m["info"].get("parentID") == user_id)
                removed_children = {
                    ((part.get("state") or {}).get("metadata") or {}).get("sessionId")
                    for message in doc.data.get("messages", [])
                    if message["info"].get("id") in remove_ids
                    for part in message.get("parts", []) if part.get("tool") == "task"
                } - {None}
                doc.data["messages"] = [m for m in doc.data.get("messages", [])
                                        if m["info"].get("id") not in remove_ids]
                tree = (doc.context or {}).get("tree")
                if tree is not None and removed_children:
                    tree.children = [child for child in tree.children
                                     if child.source_id not in removed_children]
                    tree.agent_edges = [edge for edge in tree.agent_edges
                                        if edge.child_session_id not in removed_children]
                notes.append(f"删除第 {turn} 轮")
            elif kind == "truncate":
                threshold = int(op.get("threshold", 4096))
                changed = 0
                for message in doc.data.get("messages", []):
                    for part in message.get("parts", []):
                        if part.get("type") != "tool":
                            continue
                        state = part.get("state") or {}
                        output = state.get("output")
                        if isinstance(output, str) and len(output) > threshold:
                            clipped = _shorten(output, threshold)
                            state["output"] = clipped
                            metadata = state.get("metadata") or {}
                            if isinstance(metadata.get("output"), str):
                                metadata["output"] = clipped
                            metadata["truncated"] = True
                            state["metadata"] = metadata
                            changed += 1
                notes.append(f"裁剪了 {changed} 条超长工具输出")
            elif kind == "rewrite":
                locator = op.get("locator") or op.get("uuid") or ""
                message = next((m for m in messages
                                if m["info"].get("id") == locator), None)
                if message is None and str(locator).startswith("index:"):
                    try:
                        wanted = int(str(locator).removeprefix("index:"))
                        message = messages[wanted]
                    except (ValueError, IndexError):
                        pass
                if message is None:
                    raise ValueError("OpenCode 消息定位符已失效，请刷新会话")
                if message["info"].get("role") != "user":
                    raise ValueError("目前只允许改写用户消息")
                text_parts = [p for p in message.get("parts", [])
                              if p.get("type") == "text"]
                if not text_parts:
                    raise ValueError("该用户消息没有可改写的文本")
                text_parts[0]["text"] = op["text"]
                for part in text_parts[1:]:
                    part["text"] = ""
                notes.append("改写 1 条消息")
            else:
                raise ValueError(f"未知操作: {kind}")
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
            if info.get("role") == "assistant" and not info.get("finish"):
                raise ValueError("OpenCode assistant 消息缺少 finish")
            for part in message.get("parts", []):
                if part.get("messageID") != mid or part.get("sessionID") != sid:
                    raise ValueError("OpenCode part 外键不一致")

    def stats(self, doc):
        return {"count": len(doc.data.get("messages", [])),
                "size": _json_size(doc.data)}

    @staticmethod
    def _part_map(payload):
        return {part["id"]: (message["info"]["id"], part)
                for message in payload.get("messages", [])
                for part in message.get("parts", []) if part.get("id")}

    def snapshot(self, doc, reason="会话编辑前自动"):
        claude_edit.BACKUP_DIR.mkdir(parents=True, exist_ok=True)
        path = claude_edit.BACKUP_DIR / f"{doc.ref}-{time.time_ns()}.jsonl"
        original = (doc.context or {}).get("original", doc.data)
        path.write_text(json.dumps(original, ensure_ascii=False) + "\n")
        path.with_suffix(".meta.json").write_text(json.dumps({
            "reason": reason, "tool": self.name, "source": doc.ref,
        }, ensure_ascii=False))
        return path

    def restore_snapshot(self, snapshot, doc):
        original = json.loads(Path(snapshot).read_text().splitlines()[0])
        current = rw_opencode._oc_export(doc.ref)
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
        restored = self._part_map(rw_opencode._oc_export(doc.ref))
        if any(restored.get(part_id, (None, None))[1] != part
               for part_id, (_, part) in original_parts.items()):
            raise RuntimeError("OpenCode 快照恢复后静态校验失败")

    def commit(self, doc):
        fresh = rw_opencode._oc_export(doc.ref)
        fresh_raw = json.dumps(fresh, ensure_ascii=False, sort_keys=True).encode()
        if _hash_bytes(fresh_raw) != doc.revision:
            raise RuntimeError("源会话在预览后已变化，请重新预览")
        original = (doc.context or {}).get("original") or fresh
        before = self._part_map(original)
        after = self._part_map(doc.data)
        before_messages = {m["info"].get("id") for m in original.get("messages", [])}
        after_messages = {m["info"].get("id") for m in doc.data.get("messages", [])}
        if before_messages != after_messages or set(before) != set(after):
            raise ValueError("OpenCode 删除整轮需要官方批量事务 API，请改用另存为")
        changed = [(part_id, after[part_id][0], before[part_id][1], after[part_id][1])
                   for part_id in before if before[part_id][1] != after[part_id][1]]
        cwd = doc.data.get("info", {}).get("directory") or "."
        applied = []
        with self._api_factory(cwd) as client:
            caps = client.capabilities()
            if not caps.get("patch_part"):
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
        return {"session_id": doc.ref, "saved_as": str(rw_opencode.OPENCODE_DB),
                "resume": f"cd {cwd} && opencode -s {doc.ref}",
                "updated_parts": len(changed)}

    def save_copy(self, doc):
        cwd = doc.data["info"].get("directory") or "."
        tree = (doc.context or {}).get("tree")
        if tree is None:
            raise RuntimeError("OpenCode 原生会话树未加载")
        tree.meta["opencode_export"] = rw_opencode._clone(doc.data)
        new_sid, dest = rw_opencode.write(tree, cwd=cwd)
        return {"session_id": new_sid, "saved_as": str(dest),
                "tree_count": sum(1 for _ in tree.walk()),
                "resume": f"cd {cwd} && opencode -s {new_sid}"}

    def discard(self, result):
        try:
            tree = rw_opencode.read(result["session_id"])
            ids = [node.source_id for node in reversed(list(tree.walk()))]
        except Exception:
            ids = [result["session_id"]]
        for sid in ids:
            rw_opencode._oc(["session", "delete", sid])


BACKENDS = {backend.name: backend for backend in (
    ClaudeBackend(), CodexBackend(), OpenCodeBackend())}


def backend(tool: str) -> EditBackend:
    try:
        return BACKENDS[tool]
    except KeyError as error:
        raise ValueError(f"{tool} 尚未实现会话编辑后端") from error


def preview(tool: str, ref: str, ops: list[dict]) -> dict:
    impl = backend(tool)
    doc = impl.load(ref)
    before = impl.stats(doc)
    notes = impl.apply_ops(doc, ops)
    impl.validate(doc)
    return {"before": before, "after": impl.stats(doc), "notes": notes,
            "revision": doc.revision, "capabilities": impl.capabilities()}


def apply(tool: str, ref: str, ops: list[dict], save_as: bool) -> tuple[dict, EditBackend, EditDocument, Path | None]:
    impl = backend(tool)
    if not impl.supports_mode(ops, save_as):
        mode = "另存为" if save_as else "原地编辑"
        raise ValueError(f"{tool} 不支持以{mode}执行当前操作组合")
    doc = impl.load(ref)
    notes = impl.apply_ops(doc, ops)
    impl.validate(doc)
    snapshot = None if save_as else impl.snapshot(doc)
    try:
        result = impl.save_copy(doc) if save_as else impl.commit(doc)
        result.update(ok=True, notes=notes, revision=doc.revision)
        if snapshot:
            result["snapshot"] = str(snapshot)
        return result, impl, doc, snapshot
    except Exception:
        if snapshot:
            impl.restore_snapshot(snapshot, doc)
        raise
