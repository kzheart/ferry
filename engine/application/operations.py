"""统一写操作计划。"""
from __future__ import annotations

import hashlib
import json
import secrets
import threading
import time
from dataclasses import dataclass

from ..domain.authoring import AssistantReply
from ..domain.errors import (
    AgentReferenceError,
    AgentRequestError,
    ConcurrentModificationError,
    LocatorStaleError,
    OperationUnsupportedError,
)
from . import agent_tools, services
from .editing import apply_mutation, preview_mutation
from .ports import current


PLAN_TTL_MS = 10 * 60 * 1000


def _now_ms() -> int:
    return int(time.time() * 1000)


def _canonical(value) -> str:
    return json.dumps(
        value, ensure_ascii=False, sort_keys=True, separators=(",", ":"),
        allow_nan=False,
    )


def _digest_json(value_json: str) -> str:
    return hashlib.sha256(value_json.encode()).hexdigest()


@dataclass(frozen=True)
class OperationPlan:
    plan_id: str
    kind: str
    input_json: str
    preview_json: str
    input_digest: str
    preview_digest: str
    base_revision: str
    document_revision: str | None
    created_at: int
    expires_at: int

    def input(self) -> dict:
        return json.loads(self.input_json)

    def preview(self) -> dict:
        return json.loads(self.preview_json)


@dataclass
class OperationState:
    status: str = "planned"
    result_json: str | None = None
    error_type: str | None = None
    updated_at: int = 0


class OperationService:
    def __init__(self):
        self._plans: dict[str, OperationPlan] = {}
        self._states: dict[str, OperationState] = {}
        self._lock = threading.RLock()

    def plan(self, value: dict) -> dict:
        if not isinstance(value, dict):
            raise AgentRequestError("operation input 必须是 object")
        if value.get("kind") == "edit":
            return self._plan_edit(value)
        if value.get("kind") == "migration":
            return self._plan_migration(value)
        raise AgentRequestError(
            "operation kind 非法", {"kind": value.get("kind")},
        )

    def _plan_edit(self, value: dict) -> dict:
        operation_input = self._validate_edit_input(value)
        tool = operation_input["tool"]
        ref = operation_input["ref"]

        before = agent_tools._INDEX.resolve(tool, ref)
        preview = self._preview_edit(before, operation_input["ops"])
        after = agent_tools._INDEX.resolve(tool, ref)
        if before.revision != after.revision:
            raise ConcurrentModificationError(
                "会话在生成操作计划时已变化，请重新计划"
            )
        plugin = current().adapter(tool)
        editor = plugin.require("editor")
        try:
            native_ops = self._resolve_ops(after, operation_input["ops"])
        except LocatorStaleError as error:
            raise self._public_locator_error(operation_input["ops"]) from error
        self._require_inplace_support(plugin, editor, native_ops)

        return self._store_plan(
            operation_input,
            preview,
            base_revision=after.revision,
            document_revision=str(preview["revision"]),
        )

    def _plan_migration(self, value: dict) -> dict:
        operation_input = self._validate_migration_input(value)
        source_tool = operation_input["source_tool"]
        ref = operation_input["ref"]
        before = agent_tools._INDEX.resolve(source_tool, ref)
        session = agent_tools._read_record(before)
        preview = services.migrate(
            source_tool,
            operation_input["target_tool"],
            before.canonical_ref,
            dry_run=True,
            probe=operation_input["probe"],
            max_turn=operation_input.get("max_turn"),
            probe_model=operation_input.get("probe_model"),
            _session=session,
        )
        try:
            after = agent_tools._INDEX.resolve(source_tool, ref)
        except AgentReferenceError as error:
            raise ConcurrentModificationError(
                "会话在生成操作计划时已变化，请重新计划"
            ) from error
        if before.revision != after.revision:
            raise ConcurrentModificationError(
                "会话在生成操作计划时已变化，请重新计划"
            )
        return self._store_plan(
            operation_input,
            preview,
            base_revision=after.revision,
            document_revision=None,
        )

    def _store_plan(self, operation_input: dict, preview: dict, *,
                    base_revision: str,
                    document_revision: str | None) -> dict:
        input_json = _canonical(operation_input)
        preview_json = _canonical(preview)
        now = _now_ms()
        operation = OperationPlan(
            plan_id="op_" + secrets.token_urlsafe(18),
            kind=operation_input["kind"],
            input_json=input_json,
            preview_json=preview_json,
            input_digest=_digest_json(input_json),
            preview_digest=_digest_json(preview_json),
            base_revision=base_revision,
            document_revision=document_revision,
            created_at=now,
            expires_at=now + PLAN_TTL_MS,
        )
        with self._lock:
            self._plans[operation.plan_id] = operation
            self._states[operation.plan_id] = OperationState(updated_at=now)
        return self._public_plan(operation)

    def apply(self, plan_id: str) -> dict:
        with self._lock:
            operation, state = self._get(plan_id)
            self._expire(operation, state)
            if state.status != "planned":
                raise AgentRequestError(
                    "operation plan 当前状态不可执行",
                    {"plan_id": plan_id, "status": state.status},
                )
            state.status = "applying"
            state.updated_at = _now_ms()

        try:
            if operation.kind == "edit":
                result = self._apply_edit(operation)
            elif operation.kind == "migration":
                result = self._apply_migration(operation)
            else:
                raise AgentRequestError(
                    "operation kind 非法", {"kind": operation.kind},
                )
        except Exception as error:
            with self._lock:
                state.status = "failed"
                state.error_type = type(error).__name__
                state.updated_at = _now_ms()
            raise

        result_json = _canonical(result)
        with self._lock:
            state.status = "applied"
            state.result_json = result_json
            state.updated_at = _now_ms()
        return {
            "plan_id": plan_id,
            "status": "applied",
            "result": json.loads(result_json),
        }

    def status(self, plan_id: str) -> dict:
        with self._lock:
            operation, state = self._get(plan_id)
            self._expire(operation, state)
            result = {
                "plan_id": plan_id,
                "kind": operation.kind,
                "status": state.status,
                "created_at": operation.created_at,
                "expires_at": operation.expires_at,
                "updated_at": state.updated_at,
            }
            if state.error_type:
                result["error_type"] = state.error_type
            if state.result_json is not None:
                result["result"] = json.loads(state.result_json)
            return result

    def cancel(self, plan_id: str) -> dict:
        with self._lock:
            operation, state = self._get(plan_id)
            self._expire(operation, state)
            if state.status != "planned":
                raise AgentRequestError(
                    "仅 planned operation 可以取消",
                    {"plan_id": plan_id, "status": state.status},
                )
            state.status = "cancelled"
            state.updated_at = _now_ms()
            return {"plan_id": plan_id, "status": state.status}

    @staticmethod
    def _validate_edit_input(value) -> dict:
        allowed = {"kind", "tool", "ref", "ops", "probe"}
        if set(value) - allowed:
            raise AgentRequestError(
                "edit operation 包含未知字段",
                {"fields": sorted(set(value) - allowed)},
            )
        tool, ref, ops = value.get("tool"), value.get("ref"), value.get("ops")
        probe = value.get("probe", False)
        if not isinstance(tool, str) or not tool:
            raise AgentRequestError("operation tool 非法")
        if not isinstance(ref, str) or not ref:
            raise AgentRequestError("operation ref 非法")
        if not isinstance(probe, bool):
            raise AgentRequestError("operation probe 必须是布尔值")
        ops = OperationService._validate_ops(ops)
        if len(_canonical(ops).encode()) > 64 * 1024:
            raise AgentRequestError("ops 超过 64 KiB")
        return json.loads(_canonical({
            "kind": "edit", "tool": tool, "ref": ref, "ops": ops,
            "probe": probe,
        }))

    @staticmethod
    def _validate_migration_input(value: dict) -> dict:
        allowed = {
            "kind", "source_tool", "ref", "target_tool",
            "max_turn", "probe", "probe_model",
        }
        unknown = set(value) - allowed
        if unknown:
            raise AgentRequestError(
                "migration operation 包含未知字段",
                {"fields": sorted(unknown)},
            )
        source_tool = value.get("source_tool")
        target_tool = value.get("target_tool")
        ref = value.get("ref")
        if not isinstance(source_tool, str) or not 1 <= len(source_tool) <= 64:
            raise AgentRequestError("migration source_tool 非法")
        if not isinstance(target_tool, str) or not 1 <= len(target_tool) <= 64:
            raise AgentRequestError("migration target_tool 非法")
        adapters = current().adapters()
        if source_tool not in adapters or target_tool not in adapters:
            raise AgentRequestError("migration Agent 非法")
        if source_tool == target_tool:
            raise AgentRequestError("migration 源和目标不能相同")
        if (not isinstance(ref, str) or not 1 <= len(ref) <= 512
                or any(ord(character) < 33 for character in ref)):
            raise AgentRequestError("migration ref 非法")
        probe = value.get("probe", False)
        if not isinstance(probe, bool):
            raise AgentRequestError("migration probe 必须是布尔值")
        max_turn = value.get("max_turn")
        if max_turn is not None and (
                isinstance(max_turn, bool) or not isinstance(max_turn, int)
                or not 1 <= max_turn <= 1_000_000):
            raise AgentRequestError("migration max_turn 非法")
        probe_model = value.get("probe_model")
        if probe_model is not None and (
                not isinstance(probe_model, str)
                or not 1 <= len(probe_model) <= 512
                or any(ord(character) < 32 for character in probe_model)):
            raise AgentRequestError("migration probe_model 非法")
        result = {
            "kind": "migration",
            "source_tool": source_tool,
            "ref": ref,
            "target_tool": target_tool,
            "probe": probe,
        }
        if max_turn is not None:
            result["max_turn"] = max_turn
        if probe_model is not None:
            result["probe_model"] = probe_model
        return json.loads(_canonical(result))

    def _apply_edit(self, operation: OperationPlan) -> dict:
        params = operation.input()
        try:
            record = agent_tools._INDEX.resolve(
                params["tool"], params["ref"],
            )
        except AgentReferenceError as error:
            raise ConcurrentModificationError(
                "会话在操作计划生成后已变化，请重新计划"
            ) from error
        if record.revision != operation.base_revision:
            raise ConcurrentModificationError(
                "会话在操作计划生成后已变化，请重新计划"
            )
        plugin = current().adapter(params["tool"])
        editor = plugin.require("editor")
        native_ops = self._resolve_ops(record, params["ops"])
        compiler = self._require_inplace_support(
            plugin, editor, native_ops,
        )
        try:
            if compiler is None:
                from .editing import apply
                result, doc, snapshot = apply(
                    editor,
                    record.canonical_ref,
                    native_ops,
                    expected_revision=operation.document_revision,
                )
            else:
                result, doc, snapshot = apply_mutation(
                    editor,
                    record.canonical_ref,
                    self._mutate(editor, compiler, native_ops),
                    expected_revision=operation.document_revision,
                )
        except LocatorStaleError as error:
            raise self._public_locator_error(params["ops"]) from error
        return services._finish_mutation(
            params["tool"], editor, result, doc, snapshot, params["probe"],
        )

    def _apply_migration(self, operation: OperationPlan) -> dict:
        params = operation.input()
        try:
            record = agent_tools._INDEX.resolve(
                params["source_tool"], params["ref"],
            )
        except AgentReferenceError as error:
            raise ConcurrentModificationError(
                "会话在迁移计划生成后已变化，请重新计划"
            ) from error
        if record.revision != operation.base_revision:
            raise ConcurrentModificationError(
                "会话在迁移计划生成后已变化，请重新计划"
            )
        session = agent_tools._read_record(record)
        result = services.migrate(
            params["source_tool"],
            params["target_tool"],
            record.canonical_ref,
            probe=params["probe"],
            max_turn=params.get("max_turn"),
            probe_model=params.get("probe_model"),
            _session=session,
        )
        structure = result.get("validation", {}).get("structure", {})
        if result.get("rolled_back") or structure.get("ok") is not True:
            raise RuntimeError("迁移写入后的结构校验失败，产物已回滚")
        return result

    @staticmethod
    def _validate_ops(ops) -> list[dict]:
        if not isinstance(ops, list) or not ops or len(ops) > 50:
            raise AgentRequestError("ops 必须是 1 到 50 项的数组")
        agent_tools._validate_json_shape(ops)
        ordinary = []
        normalized = []
        authored_turns = []
        for op in ops:
            if not isinstance(op, dict):
                raise AgentRequestError("每个 edit op 必须是 object")
            if op.get("op") != "replace-assistant-reply":
                ordinary.append(op)
                normalized.append(op)
                continue
            if set(op) != {"op", "turn", "reply"}:
                raise AgentRequestError(
                    "replace-assistant-reply 参数非法"
                )
            turn = op["turn"]
            if (isinstance(turn, bool) or
                    not isinstance(turn, (int, str)) or
                    (isinstance(turn, int) and turn < 1) or
                    (isinstance(turn, str) and not 1 <= len(turn) <= 512)):
                raise AgentRequestError(
                    "replace-assistant-reply turn 参数非法"
                )
            reply = AssistantReply.from_dict(op["reply"])
            turn_key = (type(turn).__name__, turn)
            if turn_key in authored_turns:
                raise AgentRequestError(
                    "同一轮次不能在一次编辑中重复替换",
                    {"field": "ops.turn"},
                )
            authored_turns.append(turn_key)
            normalized.append({
                "op": "replace-assistant-reply",
                "turn": turn,
                "reply": reply.to_dict(),
            })
        if ordinary:
            agent_tools._validate_ops(ordinary)
        return normalized

    @staticmethod
    def _resolve_ops(record, ops: list[dict]) -> list[dict]:
        resolved = []
        for op in ops:
            if op["op"] == "replace-assistant-reply":
                resolved.append(dict(op))
            else:
                resolved.extend(agent_tools.resolve_edit_ops(record, [op]))
        return resolved

    @staticmethod
    def _require_inplace_support(plugin, editor, ops: list[dict]):
        ordinary = [
            op for op in ops if op["op"] != "replace-assistant-reply"
        ]
        authored = [
            op for op in ops if op["op"] == "replace-assistant-reply"
        ]
        modes = editor.capabilities().get("operation_modes", {})
        if ordinary and not all(
                "inplace" in modes.get(op["op"], []) for op in ordinary):
            operation_names = ",".join(
                sorted({item["op"] for item in ordinary})
            )
            raise OperationUnsupportedError(
                plugin.id, operation_names, "inplace",
            )
        if not authored:
            return None
        compiler = plugin.require("authoring")
        if not compiler.capabilities().get("inplace"):
            raise OperationUnsupportedError(
                compiler.name, "replace-assistant-reply", "inplace",
            )
        return compiler

    @staticmethod
    def _mutate(editor, compiler, ops: list[dict]):
        def mutate(doc):
            changes = []
            for op in ops:
                if op["op"] == "replace-assistant-reply":
                    changes.extend(compiler.replace(
                        doc,
                        op["turn"],
                        AssistantReply.from_dict(op["reply"]),
                    ))
                else:
                    changes.extend(editor.apply_ops(doc, [op]))
            return changes
        return mutate

    def _preview_edit(self, record, ops: list[dict]) -> dict:
        if not any(
                op["op"] == "replace-assistant-reply" for op in ops):
            return agent_tools.preview_edit(
                record.tool, record.opaque_ref, ops=ops,
            )
        plugin = current().adapter(record.tool)
        editor = plugin.require("editor")
        native_ops = self._resolve_ops(record, ops)
        compiler = self._require_inplace_support(
            plugin, editor, native_ops,
        )
        try:
            result = preview_mutation(
                editor,
                record.canonical_ref,
                self._mutate(editor, compiler, native_ops),
                loader=getattr(editor, "load_preview", None),
            )
        except LocatorStaleError as error:
            raise self._public_locator_error(ops) from error
        return agent_tools._finalize_dto({
            "tool": record.tool,
            "ref": record.opaque_ref,
            "mode": "edit",
            "session_id": agent_tools._record_session_id(record),
            "revision": agent_tools._redact(str(result["revision"]), 256),
            "before": agent_tools._bounded_json(result["before"], 12 * 1024),
            "after": agent_tools._bounded_json(result["after"], 12 * 1024),
            "changes": agent_tools._bounded_json(result["changes"], 12 * 1024),
            "capabilities": agent_tools._bounded_json({
                "editing": editor.capabilities(),
                "authoring": compiler.capabilities(),
            }, 12 * 1024),
        })

    @staticmethod
    def _public_locator_error(ops: list[dict]) -> LocatorStaleError:
        authored = next((
            op for op in ops
            if op.get("op") == "replace-assistant-reply"
            and isinstance(op.get("turn"), str)
        ), None)
        if authored is None:
            return agent_tools._public_locator_error(ops)
        return LocatorStaleError(
            "轮次定位信息与当前会话不匹配",
            {"field": "turn", "locator": authored["turn"],
             "hint": "重新读取会话，并原样使用 turns[].turn_locator"},
        )

    def _get(self, plan_id: str) -> tuple[OperationPlan, OperationState]:
        if not isinstance(plan_id, str) or not plan_id.startswith("op_"):
            raise AgentRequestError("plan_id 非法")
        operation = self._plans.get(plan_id)
        state = self._states.get(plan_id)
        if operation is None or state is None:
            raise AgentRequestError("operation plan 不存在或已因重启失效")
        return operation, state

    @staticmethod
    def _expire(operation: OperationPlan, state: OperationState) -> None:
        if state.status == "planned" and operation.expires_at < _now_ms():
            state.status = "expired"
            state.updated_at = _now_ms()

    @staticmethod
    def _public_plan(operation: OperationPlan) -> dict:
        params = operation.input()
        return {
            "plan_id": operation.plan_id,
            "kind": operation.kind,
            "status": "planned",
            "preview": operation.preview(),
            "risk": "high",
            "affected_refs": [params["ref"]],
            "base_revision": operation.base_revision,
            "document_revision": operation.document_revision,
            "input_digest": operation.input_digest,
            "preview_digest": operation.preview_digest,
            "created_at": operation.created_at,
            "expires_at": operation.expires_at,
        }


_SERVICE = OperationService()


def reset_service() -> None:
    global _SERVICE
    _SERVICE = OperationService()


def plan(value: dict) -> dict:
    return _SERVICE.plan(value)


def apply(plan_id: str) -> dict:
    return _SERVICE.apply(plan_id)


def status(plan_id: str) -> dict:
    return _SERVICE.status(plan_id)


def cancel(plan_id: str) -> dict:
    return _SERVICE.cancel(plan_id)
