"""会话编辑计划与写入处理。"""

import json

from ..context import EngineContext
from ..errors import (
    AgentReferenceError,
    AgentRequestError,
    ConcurrentModificationError,
    LocatorStaleError,
    OperationUnsupportedError,
)
from ..sessions.index import AgentSessionIndex
from ..sessions.safety import (
    bounded_json,
    finalize_dto,
    record_session_id,
    redact,
)
from .plan_store import OperationPlan
from .types import AssistantReply
from .validation import validate_ops


def preview_mutation(editor, ref: str, mutate, loader=None) -> dict:
    doc = (loader or editor.load)(ref)
    before = editor.stats(doc)
    changes = mutate(doc)
    editor.validate(doc)
    return {"before": before, "after": editor.stats(doc), "changes": changes,
            "revision": doc.revision}


def apply_mutation(editor, ref: str, mutate,
                   expected_revision: str | None = None):
    doc = editor.load(ref)
    if expected_revision is not None and doc.revision != expected_revision:
        raise ConcurrentModificationError("源会话在预览后已变化，请重新预览")
    before = editor.stats(doc)
    changes = mutate(doc)
    editor.validate(doc)
    # 快照记下它救的是哪次编辑，还原界面才能说清「会失去什么」
    snapshot = editor.snapshot(
        doc, extra={"changes": changes, "before": before, "after": editor.stats(doc)})
    if not snapshot:
        raise RuntimeError("原地编辑无法创建恢复快照，已取消写入")
    try:
        result = editor.commit(doc)
        result.update(ok=True, changes=changes,
                      revision=editor.saved_revision(result, doc))
        if snapshot:
            result["snapshot"] = str(snapshot)
        return result, doc, snapshot
    except ConcurrentModificationError:
        raise
    except Exception:
        editor.restore_snapshot(snapshot, doc)
        raise


def preview(editor, ref: str, ops: list[dict], loader=None) -> dict:
    return preview_mutation(
        editor, ref, lambda doc: editor.apply_ops(doc, ops), loader=loader)


def apply(editor, ref: str, ops: list[dict],
          expected_revision: str | None = None):
    if not all(op.get("op") in editor.operations for op in ops):
        raise OperationUnsupportedError(
            editor.name, ",".join(op.get("op", "?") for op in ops),
            "inplace")
    return apply_mutation(
        editor, ref, lambda doc: editor.apply_ops(doc, ops),
        expected_revision=expected_revision)


class EditOperationHandler:
    def __init__(self, ports: EngineContext, index: AgentSessionIndex):
        self._ports = ports
        self._index = index

    def ensure_supported(self, record, ops: list[dict]) -> list[dict]:
        try:
            native_ops = self.resolve_ops(record, ops)
        except LocatorStaleError as error:
            raise self.public_locator_error(ops) from error
        adapter = self._ports.adapter(record.tool)
        self.require_inplace_support(adapter, adapter.editor, native_ops)
        return native_ops

    def apply(self, operation: OperationPlan, finish_mutation) -> dict:
        params = operation.input()
        try:
            record = self._index.resolve(params["tool"], params["ref"])
        except AgentReferenceError as error:
            raise ConcurrentModificationError(
                "会话在操作计划生成后已变化，请重新计划"
            ) from error
        if record.revision != operation.base_revision:
            raise ConcurrentModificationError(
                "会话在操作计划生成后已变化，请重新计划"
            )
        adapter = self._ports.adapter(params["tool"])
        editor = adapter.editor
        native_ops = self.ensure_supported(record, params["ops"])
        try:
            if not any(
                item["op"] == "replace-assistant-reply"
                for item in native_ops
            ):
                result, document, snapshot = apply(
                    editor,
                    record.canonical_ref,
                    native_ops,
                    expected_revision=operation.document_revision,
                )
            else:
                result, document, snapshot = apply_mutation(
                    editor,
                    record.canonical_ref,
                    self.mutation(editor, native_ops),
                    expected_revision=operation.document_revision,
                )
        except LocatorStaleError as error:
            raise self.public_locator_error(params["ops"]) from error
        return finish_mutation(
            params["tool"],
            editor,
            result,
            document,
            snapshot,
            params["probe"],
        )

    def resolve_ops(self, record, ops: list[dict]) -> list[dict]:
        resolved = []
        for operation in ops:
            if operation["op"] == "replace-assistant-reply":
                resolved.append(dict(operation))
                continue
            item = dict(operation)
            if item["op"] == "rewrite":
                message = self._index.resolve_message_locator(
                    record, item["locator"],
                )
                if not message.editable:
                    raise AgentRequestError(
                        "目标消息不支持文本改写",
                        {
                            "field": "locator",
                            "locator": item["locator"],
                            "hint": "仅使用 editable=true 的消息引用",
                        },
                    )
                item["locator"] = message.native_locator
            resolved.append(item)
        return resolved

    @staticmethod
    def require_inplace_support(adapter, editor, ops: list[dict]):
        ordinary = [
            operation
            for operation in ops
            if operation["op"] != "replace-assistant-reply"
        ]
        replacements = [
            operation
            for operation in ops
            if operation["op"] == "replace-assistant-reply"
        ]
        if ordinary and not all(
            operation["op"] in editor.operations
            for operation in ordinary
        ):
            operation_names = ",".join(
                sorted({item["op"] for item in ordinary})
            )
            raise OperationUnsupportedError(
                adapter.id, operation_names, "inplace",
            )
        if replacements and "replace-assistant-reply" not in editor.operations:
            raise OperationUnsupportedError(
                adapter.id, "replace-assistant-reply", "inplace",
            )
        return editor

    @staticmethod
    def mutation(editor, ops: list[dict]):
        def mutate(document):
            changes = []
            for operation in ops:
                if operation["op"] == "replace-assistant-reply":
                    changes.extend(editor.replace_reply(
                        document,
                        operation["turn"],
                        AssistantReply.from_dict(operation["reply"]),
                    ))
                else:
                    changes.extend(editor.apply_ops(document, [operation]))
            return changes
        return mutate

    def preview(self, record, ops: list[dict]) -> dict:
        ops = validate_ops(ops)
        if len(json.dumps(
            ops, ensure_ascii=False, default=str,
        ).encode()) > 64 * 1024:
            raise AgentRequestError("ops 超过 64 KiB")
        has_replacement = any(
            operation["op"] == "replace-assistant-reply"
            for operation in ops
        )
        adapter = self._ports.adapter(record.tool)
        editor = adapter.editor
        native_ops = self.ensure_supported(record, ops)
        try:
            result = preview_mutation(
                editor,
                record.canonical_ref,
                (
                    self.mutation(editor, native_ops)
                    if has_replacement
                    else lambda document: editor.apply_ops(
                        document, native_ops,
                    )
                ),
                loader=getattr(editor, "load_preview", None),
            )
        except LocatorStaleError as error:
            raise self.public_locator_error(ops) from error
        return finalize_dto({
            "tool": record.tool,
            "ref": record.opaque_ref,
            "mode": "edit",
            "session_id": record_session_id(record),
            "revision": redact(str(result["revision"]), 256),
            "before": bounded_json(result["before"], 12 * 1024),
            "after": bounded_json(result["after"], 12 * 1024),
            "changes": bounded_json(result["changes"], 12 * 1024),
        })

    @staticmethod
    def public_locator_error(ops: list[dict]) -> LocatorStaleError:
        authored = next((
            operation
            for operation in ops
            if operation.get("op") == "replace-assistant-reply"
            and isinstance(operation.get("turn"), str)
        ), None)
        if authored is None:
            locator = next((
                operation.get("locator")
                for operation in ops
                if operation.get("op") == "rewrite"
            ), None)
            return LocatorStaleError(
                "消息定位信息与当前会话不匹配",
                {
                    "field": "locator",
                    "locator": locator,
                    "hint": (
                        "重新调用 ferry_get_session_context，"
                        "并原样使用 messages[].locator"
                    ),
                },
            )
        return LocatorStaleError(
            "轮次定位信息与当前会话不匹配",
            {
                "field": "turn",
                "locator": authored["turn"],
                "hint": "重新读取会话，并原样使用 turns[].turn_locator",
            },
        )
