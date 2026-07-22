"""Agent 写操作门禁：不可变提议、一次性审批、CAS 应用与审计。"""
from __future__ import annotations

import hashlib
import hmac
import json
import os
import secrets
import threading
import time
from dataclasses import dataclass
from pathlib import Path

from ..domain.errors import AgentApprovalError, AgentRequestError
from . import agent_tools, services
from .ports import current

PROTOCOL = "ferry-mutation/v1"
OPERATION_TTL_MS = 10 * 60 * 1000
APPROVAL_TTL_MS = 2 * 60 * 1000


def _now_ms() -> int:
    return int(time.time() * 1000)


def _canonical(value) -> str:
    return json.dumps(value, ensure_ascii=False, sort_keys=True,
                      separators=(",", ":"), allow_nan=False)


def _digest(value) -> str:
    return hashlib.sha256(_canonical(value).encode()).hexdigest()


@dataclass(frozen=True)
class Operation:
    operation_id: str
    kind: str
    params: dict
    parameter_hash: str
    base_revision: str
    preview: dict
    preview_digest: str
    run_id: str
    created_at: int
    expires_at: int


@dataclass
class Approval:
    operation_id: str
    run_id: str
    parameter_hash: str
    token_digest: str
    expires_at: int
    consumed: bool = False


class MutationGateway:
    def __init__(self):
        self._operations: dict[str, Operation] = {}
        self._approvals: dict[str, Approval] = {}
        self._lock = threading.RLock()

    @staticmethod
    def _audit_path() -> Path:
        return Path(current().snapshot_dir()) / "agent-operation-audit.jsonl"

    def _audit(self, event: dict) -> None:
        path = self._audit_path()
        path.parent.mkdir(parents=True, exist_ok=True)
        line = _canonical({"protocol": PROTOCOL, **event}) + "\n"
        fd = os.open(path, os.O_APPEND | os.O_CREAT | os.O_WRONLY, 0o600)
        with os.fdopen(fd, "ab") as stream:
            stream.write(line.encode())
            stream.flush()
            os.fsync(stream.fileno())

    def propose(self, kind: str, params: dict, preview: dict,
                base_revision: str, run_id: str) -> dict:
        if not isinstance(run_id, str) or not 1 <= len(run_id) <= 200:
            raise AgentRequestError("run_id 非法")
        now = _now_ms()
        safe_preview = agent_tools._bounded_json(preview, 40 * 1024)
        operation = Operation(
            operation_id="op_" + secrets.token_urlsafe(18),
            kind=kind,
            params=json.loads(_canonical(params)),
            parameter_hash=_digest(params),
            base_revision=base_revision,
            preview=json.loads(_canonical(safe_preview)),
            preview_digest=_digest(safe_preview),
            run_id=run_id,
            created_at=now,
            expires_at=now + OPERATION_TTL_MS,
        )
        with self._lock:
            self._operations[operation.operation_id] = operation
            self._audit({
                "event": "proposed",
                "operation_id": operation.operation_id,
                "kind": operation.kind,
                "parameter_hash": operation.parameter_hash,
                "base_revision": operation.base_revision,
                "preview_digest": operation.preview_digest,
                "created_at": operation.created_at,
                "expires_at": operation.expires_at,
                "run_id": operation.run_id,
            })
        return self._public(operation)

    @staticmethod
    def _public(operation: Operation) -> dict:
        return agent_tools._finalize_dto({
            "operation_id": operation.operation_id,
            "kind": operation.kind,
            "summary": _summary(operation),
            "affected_refs": [operation.params["ref"]],
            "preview": operation.preview,
            "risk": "low" if operation.kind == "metadata" else "medium",
            "base_revision": operation.base_revision,
            "parameter_hash": operation.parameter_hash,
            "expires_at": operation.expires_at,
        })

    def authorize(self, operation_id: str, run_id: str) -> dict:
        if not isinstance(run_id, str) or not 1 <= len(run_id) <= 200:
            raise AgentApprovalError("run_id 非法")
        with self._lock:
            operation = self._get_live(operation_id)
            if operation.run_id != run_id:
                raise AgentApprovalError("审批 run 与 proposal 发起 run 不一致")
            token = "apv_" + secrets.token_urlsafe(32)
            approval = Approval(
                operation_id=operation_id,
                run_id=run_id,
                parameter_hash=operation.parameter_hash,
                token_digest=hashlib.sha256(token.encode()).hexdigest(),
                expires_at=min(operation.expires_at, _now_ms() + APPROVAL_TTL_MS),
            )
            self._approvals[operation_id] = approval
            self._audit({
                "event": "approved",
                "operation_id": operation_id,
                "run_id": run_id,
                "parameter_hash": operation.parameter_hash,
                "expires_at": approval.expires_at,
                "at": _now_ms(),
            })
        return {"operation_id": operation_id, "approval_token": token,
                "expires_at": approval.expires_at}

    def detail(self, operation_id: str) -> dict:
        """仅供受信任 UI 审批路径读取，不注册为模型工具。"""
        with self._lock:
            operation = self._get_live(operation_id)
            return {
                "operation_id": operation.operation_id,
                "kind": operation.kind,
                "params": json.loads(_canonical(operation.params)),
                "parameter_hash": operation.parameter_hash,
                "base_revision": operation.base_revision,
                "preview": json.loads(_canonical(operation.preview)),
                "preview_digest": operation.preview_digest,
                "run_id": operation.run_id,
                "created_at": operation.created_at,
                "expires_at": operation.expires_at,
            }

    def apply(self, operation_id: str, run_id: str, token: str) -> dict:
        with self._lock:
            operation = self._get_live(operation_id)
            approval = self._approvals.get(operation_id)
            supplied = hashlib.sha256(str(token).encode()).hexdigest()
            if (approval is None or approval.consumed
                    or approval.run_id != run_id
                    or approval.parameter_hash != operation.parameter_hash
                    or approval.expires_at < _now_ms()
                    or not hmac.compare_digest(approval.token_digest, supplied)):
                raise AgentApprovalError("审批凭证无效、过期或已消费")
            approval.consumed = True
            self._audit({"event": "applying", "operation_id": operation_id,
                         "run_id": run_id, "at": _now_ms()})
        try:
            with _APPLY_LOCK:
                result = _apply_operation(operation)
        except Exception as error:
            self._audit({"event": "failed", "operation_id": operation_id,
                         "error_type": type(error).__name__, "at": _now_ms()})
            raise
        self._audit({"event": "applied", "operation_id": operation_id,
                     "result_digest": _digest(result), "at": _now_ms()})
        return {"operation_id": operation_id, "status": "applied", "result": result}

    def _get_live(self, operation_id: str) -> Operation:
        operation = self._operations.get(operation_id)
        if operation is None:
            raise AgentApprovalError("操作不存在或已因重启失效")
        if operation.expires_at < _now_ms():
            raise AgentApprovalError("操作已过期")
        return operation

    def status(self, operation_id: str) -> dict:
        if not isinstance(operation_id, str) or not operation_id.startswith("op_"):
            raise AgentRequestError("operation_id 非法")
        events = []
        try:
            lines = self._audit_path().read_text().splitlines()
        except OSError:
            lines = []
        for line in lines:
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue
            if event.get("operation_id") == operation_id:
                events.append({key: value for key, value in event.items()
                               if key not in {"params"}})
        if not events:
            raise AgentRequestError("operation 不存在")
        return {"operation_id": operation_id,
                "status": events[-1].get("event"), "events": events}


def _summary(operation: Operation) -> str:
    params = operation.params
    if operation.kind == "migration":
        return f"将 {params['source_tool']} 会话迁移到 {params['target_tool']}"
    if operation.kind == "metadata":
        return "修改会话元数据"
    return "应用已预览的会话编辑"


def _apply_operation(operation: Operation) -> dict:
    params = operation.params
    record = agent_tools._INDEX.resolve(params["tool"], params["ref"])
    if record.revision != operation.base_revision:
        raise AgentApprovalError("会话 revision 已变化，请重新提议")
    if operation.kind == "migration":
        session = agent_tools._read_record(record)
        result = services.migrate(
            params["source_tool"], params["target_tool"], record.canonical_ref,
            max_turn=params.get("max_turn"), probe=False, _session=session)
        if result.get("rolled_back") or not result.get("validation", {}).get(
                "structure", {}).get("ok"):
            raise RuntimeError("迁移写入后的结构校验失败")
        return result
    if operation.kind == "edit":
        plugin = current().adapter(params["tool"])
        editor = plugin.require("editor")
        if params.get("ops") is not None:
            from .editing import apply
            result, doc, snapshot = apply(
                editor, record.canonical_ref, params["ops"],
                params["save_as"], expected_revision=params["document_revision"])
        else:
            from .authoring import apply
            result, doc, snapshot = apply(
                editor, plugin.require("authoring"), record.canonical_ref,
                params["turn"], params["reply"], params["save_as"],
                revision=params["document_revision"])
        return services._finish_mutation(
            params["tool"], editor, result, doc, snapshot, False,
            params["save_as"])
    if operation.kind == "metadata":
        session_id = record.row.get("id")
        if not isinstance(session_id, str) or not session_id:
            raise AgentRequestError("会话缺少可用的 metadata id")
        result = services.session_meta_compare_and_set(
            session_id, params["metadata_before"], params["patch"])
        return {"metadata": result}
    raise AgentRequestError("未知 mutation kind")


_GATEWAY = MutationGateway()
_APPLY_LOCK = threading.RLock()


def reset_gateway() -> None:
    global _GATEWAY
    _GATEWAY = MutationGateway()


def propose_migration(source_tool: str, ref: str, target_tool: str,
                      run_id: str, max_turn: int | None = None) -> dict:
    preview = agent_tools.preview_migration(
        source_tool, ref, target_tool, max_turn=max_turn)
    params = {"tool": source_tool, "source_tool": source_tool, "ref": ref,
              "target_tool": target_tool, "max_turn": max_turn}
    return _GATEWAY.propose(
        "migration", params, preview, preview["revision"], run_id)


def propose_edit(tool: str, ref: str, *, ops=None, turn=None, reply=None,
                 save_as: bool = True, run_id: str) -> dict:
    if not isinstance(save_as, bool):
        raise AgentRequestError("save_as 必须是 boolean")
    if not save_as:
        raise AgentRequestError("Agent 编辑仅允许另存副本")
    preview = agent_tools.preview_edit(
        tool, ref, ops=ops, turn=turn, reply=reply)
    record = agent_tools._INDEX.resolve(tool, ref)
    params = {"tool": tool, "ref": ref, "ops": ops, "turn": turn,
              "reply": reply, "save_as": save_as,
              "document_revision": preview["revision"]}
    return _GATEWAY.propose("edit", params, preview, record.revision, run_id)


def propose_metadata_change(tool: str, ref: str, patch: dict,
                            run_id: str) -> dict:
    if not isinstance(patch, dict) or not patch or not set(patch) <= services.META_FIELDS:
        raise AgentRequestError("metadata patch 字段非法")
    agent_tools._validate_json_shape(patch, max_depth=3, max_nodes=50)
    if "name" in patch and (not isinstance(patch["name"], str)
                            or len(patch["name"]) > 200):
        raise AgentRequestError("metadata name 非法")
    for field in ("pinned", "archived"):
        if field in patch and not isinstance(patch[field], bool):
            raise AgentRequestError(f"metadata {field} 必须是 boolean")
    if "tags" in patch:
        tags = patch["tags"]
        if (not isinstance(tags, list) or len(tags) > 20
                or not all(isinstance(tag, str) and 1 <= len(tag) <= 64
                           for tag in tags)):
            raise AgentRequestError("metadata tags 非法")
    if len(_canonical(patch).encode()) > 4096:
        raise AgentRequestError("metadata patch 超过 4 KiB")
    record = agent_tools._INDEX.resolve(tool, ref)
    before = services.session_meta_list().get(record.row.get("id"), {})
    preview = {"tool": tool, "ref": ref, "before": before,
               "after_patch": patch}
    return _GATEWAY.propose(
        "metadata", {"tool": tool, "ref": ref, "patch": patch,
                     "metadata_before": before},
        preview, record.revision, run_id)


def authorize(operation_id: str, run_id: str) -> dict:
    return _GATEWAY.authorize(operation_id, run_id)


def detail(operation_id: str) -> dict:
    return _GATEWAY.detail(operation_id)


def apply(operation_id: str, run_id: str, approval_token: str) -> dict:
    return _GATEWAY.apply(operation_id, run_id, approval_token)


def status(operation_id: str) -> dict:
    return _GATEWAY.status(operation_id)
