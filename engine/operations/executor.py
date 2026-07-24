"""已审批操作的原生写入执行。"""
from __future__ import annotations

import secrets
from collections.abc import Callable

from ..context import EngineContext
from ..errors import (
    AgentReferenceError,
    AgentRequestError,
    ConcurrentModificationError,
)
from ..sessions import agent_read
from ..sessions.index import AgentSessionIndex
from . import metadata, verification as probe_mod
from .delete import SessionDeletionService
from .edit import EditOperationHandler
from .migrate import MigrationService
from .plan_store import OperationPlan, now_ms


class OperationExecutor:
    def __init__(
        self,
        ports: EngineContext,
        index: AgentSessionIndex,
        migration: MigrationService,
        edit: EditOperationHandler,
        database: Callable,
    ):
        self._ports = ports
        self._index = index
        self._migration = migration
        self._edit = edit
        self._database = database

    def execute(self, operation: OperationPlan) -> dict:
        handlers = {
            "edit": self._apply_edit,
            "migration": self._apply_migration,
            "metadata": self._apply_metadata,
            "delete": self._apply_delete,
            "restore-delete": self._apply_restore_delete,
        }
        handler = handlers.get(operation.kind)
        if handler is None:
            raise AgentRequestError(
                "operation kind 非法", {"kind": operation.kind},
            )
        return handler(operation)

    def _apply_edit(self, operation: OperationPlan) -> dict:
        return self._edit.apply(operation, self._finish_mutation)

    def _finish_mutation(self, tool, editor, result, document, snapshot, probe):
        if not probe:
            return result
        try:
            report = self._ports.adapter(tool).verifier.probe_edited(
                editor, document, result,
            )
        except probe_mod.ProbeTimeout as error:
            report = probe_mod.timeout_report(tool, error)
        result["probe"] = report
        if report["status"] == "passed":
            return result
        if snapshot:
            editor.restore_snapshot(snapshot, document)
            result.update(ok=False, error="隔离探针未通过,已自动还原快照")
        return result

    def _apply_migration(self, operation: OperationPlan) -> dict:
        params = operation.input()
        try:
            record = self._index.resolve(
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
        session = agent_read.read_indexed_session(self._index, record)
        result = self._migration.apply(
            params["source_tool"],
            params["target_tool"],
            record.canonical_ref,
            probe=params["probe"],
            max_turn=params.get("max_turn"),
            probe_model=params.get("probe_model"),
            session=session,
        )
        structure = result.get("validation", {}).get("structure", {})
        if result.get("rolled_back") or structure.get("ok") is not True:
            raise RuntimeError("迁移写入后的结构校验失败，产物已回滚")
        return result

    def _apply_metadata(self, operation: OperationPlan) -> dict:
        params = operation.input()
        try:
            record = self._index.resolve(params["tool"], params["ref"])
        except AgentReferenceError as error:
            raise ConcurrentModificationError(
                "会话在元数据计划生成后已变化，请重新计划"
            ) from error
        if record.revision != operation.base_revision:
            raise ConcurrentModificationError(
                "会话在元数据计划生成后已变化，请重新计划"
            )
        if record.row.get("id") != params["session_id"]:
            raise ConcurrentModificationError(
                "会话标识在元数据计划生成后已变化，请重新计划"
            )
        result = metadata.compare_and_set_entry(
            params["tool"],
            params["session_id"],
            params["metadata_before"],
            params["patch"],
            self._ports,
        )
        return {"metadata": result}

    def _apply_delete(self, operation: OperationPlan) -> dict:
        params = operation.input()
        try:
            record = self._index.resolve(params["tool"], params["ref"])
        except AgentReferenceError as error:
            raise ConcurrentModificationError(
                "会话在删除计划生成后已变化，请重新计划"
            ) from error
        if record.revision != operation.base_revision:
            raise ConcurrentModificationError(
                "会话在删除计划生成后已变化，请重新计划"
            )
        result = SessionDeletionService(self._ports).delete(
            params["tool"], record.canonical_ref,
        )
        snapshot = result.pop("snapshot", None)
        if result.get("undoable") is True:
            if not isinstance(snapshot, str) or not snapshot:
                raise RuntimeError("可恢复删除未返回快照")
            recovery_id = "recovery_" + secrets.token_urlsafe(18)
            self._database().operations.store_recovery(
                recovery_id, params["tool"], snapshot, now_ms(),
            )
            result["recovery_id"] = recovery_id
        return result

    def _apply_restore_delete(self, operation: OperationPlan) -> dict:
        recovery_id = operation.input()["recovery_id"]
        recovery = self._database().operations.get_recovery(recovery_id)
        if recovery is None or recovery["status"] != "available":
            raise ConcurrentModificationError("删除恢复记录已使用或不可用")
        if not self._database().operations.claim_recovery(
            recovery_id, now_ms(),
        ):
            raise ConcurrentModificationError("删除恢复记录已使用或不可用")
        try:
            result = SessionDeletionService(self._ports).restore(
                recovery["snapshot"],
            )
        except Exception:
            self._database().operations.release_recovery(
                recovery_id, now_ms(),
            )
            raise
        if not self._database().operations.complete_recovery(
            recovery_id, now_ms(),
        ):
            raise RuntimeError("删除恢复状态提交失败")
        return {**result, "recovery_id": recovery_id}
