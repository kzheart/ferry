"""统一写操作计划；首个垂直切片仅支持原地编辑。"""
from __future__ import annotations

import hashlib
import json
import secrets
import threading
import time
from dataclasses import dataclass

from ..domain.errors import (
    AgentReferenceError,
    AgentRequestError,
    ConcurrentModificationError,
    LocatorStaleError,
    OperationUnsupportedError,
)
from . import agent_tools, services
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
    document_revision: str
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
        operation_input = self._validate_edit_input(value)
        tool = operation_input["tool"]
        ref = operation_input["ref"]

        before = agent_tools._INDEX.resolve(tool, ref)
        preview = agent_tools.preview_edit(
            tool, ref, ops=operation_input["ops"],
        )
        after = agent_tools._INDEX.resolve(tool, ref)
        if before.revision != after.revision:
            raise ConcurrentModificationError(
                "会话在生成操作计划时已变化，请重新计划"
            )
        editor = current().adapter(tool).require("editor")
        try:
            native_ops = agent_tools.resolve_edit_ops(
                after, operation_input["ops"],
            )
        except LocatorStaleError as error:
            raise agent_tools._public_locator_error(
                operation_input["ops"],
            ) from error
        if not editor.supports_mode(native_ops, False):
            operation_names = ",".join(
                sorted({item.get("op", "?") for item in native_ops})
            )
            raise OperationUnsupportedError(
                tool, operation_names, "inplace",
            )

        input_json = _canonical(operation_input)
        preview_json = _canonical(preview)
        now = _now_ms()
        operation = OperationPlan(
            plan_id="op_" + secrets.token_urlsafe(18),
            kind="edit",
            input_json=input_json,
            preview_json=preview_json,
            input_digest=_digest_json(input_json),
            preview_digest=_digest_json(preview_json),
            base_revision=after.revision,
            document_revision=str(preview["revision"]),
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
            result = self._apply_edit(operation)
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
        if not isinstance(value, dict) or value.get("kind") != "edit":
            raise AgentRequestError(
                "当前 operation 仅支持 edit", {"kind": value.get("kind")
                if isinstance(value, dict) else None},
            )
        tool, ref, ops = value.get("tool"), value.get("ref"), value.get("ops")
        probe = value.get("probe", False)
        if not isinstance(tool, str) or not tool:
            raise AgentRequestError("operation tool 非法")
        if not isinstance(ref, str) or not ref:
            raise AgentRequestError("operation ref 非法")
        if not isinstance(probe, bool):
            raise AgentRequestError("operation probe 必须是布尔值")
        agent_tools._validate_ops(ops)
        if len(_canonical(ops).encode()) > 64 * 1024:
            raise AgentRequestError("ops 超过 64 KiB")
        return json.loads(_canonical({
            "kind": "edit", "tool": tool, "ref": ref, "ops": ops,
            "probe": probe,
        }))

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
        native_ops = agent_tools.resolve_edit_ops(record, params["ops"])
        if not editor.supports_mode(native_ops, False):
            operations = ",".join(
                sorted({item.get("op", "?") for item in native_ops})
            )
            raise OperationUnsupportedError(
                params["tool"], operations, "inplace",
            )
        from .editing import apply
        try:
            result, doc, snapshot = apply(
                editor,
                record.canonical_ref,
                native_ops,
                False,
                expected_revision=operation.document_revision,
            )
        except LocatorStaleError as error:
            raise agent_tools._public_locator_error(params["ops"]) from error
        return services._finish_mutation(
            params["tool"], editor, result, doc, snapshot, params["probe"], False,
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
