"""统一写操作计划。"""
from __future__ import annotations

import json
import secrets
import threading
from concurrent.futures import Future, ThreadPoolExecutor

from ..contracts.operations import OPERATION_KINDS
from ..sessions import catalog as agent_tools
from ..sessions.index import AgentSessionIndex
from ..context import EngineContext
from ..errors import (
    AgentReferenceError,
    AgentRequestError,
    ConcurrentModificationError,
)
from . import metadata, verification as probe_mod
from .delete import SessionDeletionService
from .edit import EditOperationHandler
from .migrate import MigrationService
from .plan_store import (
    OperationPlan,
    OperationPlanStore,
    OperationState,
    canonical_json,
    digest_json,
    now_ms,
)
from .validation import (
    validate_delete_input,
    validate_edit_input,
    validate_metadata_input,
    validate_migration_input,
    validate_restore_delete_input,
)
from ..storage.database import StateDatabase


MUTATION_WORKERS = 1


class OperationService:
    def __init__(self, ports: EngineContext,
                 index: AgentSessionIndex):
        self._ports = ports
        self._index = index
        self._migration = MigrationService(ports)
        self._edit = EditOperationHandler(ports, index)
        self._lock = threading.RLock()
        # 所有写操作都在同一个持久化队列中串行执行。这样 IPC 请求可立即
        # 返回，同时不放宽已有 Adapter/原生文件的写入并发假设。
        self._executor = ThreadPoolExecutor(
            max_workers=MUTATION_WORKERS,
            thread_name_prefix="engine-operation",
        )
        self._jobs: dict[str, Future[None]] = {}
        self._plans = OperationPlanStore(ports.snapshot_dir)

    def _database(self):
        return self._plans.database()

    def plan(self, value: dict) -> dict:
        if not isinstance(value, dict):
            raise AgentRequestError("operation input 必须是 object")
        if value.get("kind") not in OPERATION_KINDS:
            raise AgentRequestError(
                "operation kind 非法", {"kind": value.get("kind")},
            )
        if value.get("kind") == "edit":
            return self._plan_edit(value)
        if value.get("kind") == "migration":
            return self._plan_migration(value)
        if value.get("kind") == "metadata":
            return self._plan_metadata(value)
        if value.get("kind") == "delete":
            return self._plan_delete(value)
        if value.get("kind") == "restore-delete":
            return self._plan_restore_delete(value)
        raise AssertionError("Operation contract kind 未绑定处理器")

    def _plan_edit(self, value: dict) -> dict:
        operation_input = validate_edit_input(value)
        tool = operation_input["tool"]
        ref = operation_input["ref"]

        before = self._index.resolve(tool, ref)
        preview = self._edit.preview(before, operation_input["ops"])
        after = self._index.resolve(tool, ref)
        if before.revision != after.revision:
            raise ConcurrentModificationError(
                "会话在生成操作计划时已变化，请重新计划"
            )
        self._edit.ensure_supported(after, operation_input["ops"])

        return self._store_plan(
            operation_input,
            preview,
            base_revision=after.revision,
            document_revision=str(preview["revision"]),
        )

    def _plan_migration(self, value: dict) -> dict:
        operation_input = validate_migration_input(
            value, self._ports.adapters(),
        )
        source_tool = operation_input["source_tool"]
        ref = operation_input["ref"]
        before = self._index.resolve(source_tool, ref)
        session = agent_tools._read_record(self._index, before)
        preview = self._migration.preview(
            source_tool,
            operation_input["target_tool"],
            before.canonical_ref,
            max_turn=operation_input.get("max_turn"),
            probe_model=operation_input.get("probe_model"),
            session=session,
        )
        try:
            after = self._index.resolve(source_tool, ref)
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

    def _plan_metadata(self, value: dict) -> dict:
        operation_input = validate_metadata_input(value)
        tool = operation_input["tool"]
        ref = operation_input["ref"]
        before_record = self._index.resolve(tool, ref)
        session_id = before_record.row.get("id")
        if not isinstance(session_id, str) or not session_id:
            raise AgentRequestError("会话缺少可用的 metadata id")
        metadata_before = metadata.list_all(self._ports).get(
            StateDatabase.metadata_key(tool, session_id), {}
        )
        operation_input["session_id"] = session_id
        operation_input["metadata_before"] = metadata_before
        preview = {
            "tool": tool,
            "ref": ref,
            "before": metadata_before,
            "after_patch": operation_input["patch"],
        }
        after_record = self._index.resolve(tool, ref)
        if before_record.revision != after_record.revision:
            raise ConcurrentModificationError(
                "会话在生成操作计划时已变化，请重新计划"
            )
        return self._store_plan(
            operation_input,
            preview,
            base_revision=after_record.revision,
            document_revision=None,
        )

    def _plan_delete(self, value: dict) -> dict:
        operation_input = validate_delete_input(
            value, self._ports.adapters(),
        )
        record = self._index.resolve(
            operation_input["tool"], operation_input["ref"],
        )
        adapter = self._ports.adapter(operation_input["tool"])
        lifecycle = adapter.lifecycle
        preview = {
            "tool": record.tool,
            "ref": record.opaque_ref,
            "session_id": agent_tools._record_session_id(record),
            "title": agent_tools._redact(str(record.row.get("title") or ""), 512),
            "undoable": bool(getattr(lifecycle, "delete_undoable", False)),
        }
        after = self._index.resolve(
            operation_input["tool"], operation_input["ref"],
        )
        if record.revision != after.revision:
            raise ConcurrentModificationError(
                "会话在生成删除计划时已变化，请重新计划"
            )
        return self._store_plan(
            operation_input,
            preview,
            base_revision=after.revision,
            document_revision=None,
        )

    def _plan_restore_delete(self, value: dict) -> dict:
        operation_input = validate_restore_delete_input(value)
        recovery = self._database().get_recovery(
            operation_input["recovery_id"],
        )
        if recovery is None or recovery["status"] != "available":
            raise AgentRequestError(
                "删除恢复记录不可用",
                {"recovery_id": operation_input["recovery_id"]},
            )
        preview = {
            "recovery_id": recovery["recovery_id"],
            "tool": recovery["tool"],
        }
        return self._store_plan(
            operation_input,
            preview,
            base_revision="available",
            document_revision=None,
        )

    def _store_plan(self, operation_input: dict, preview: dict, *,
                    base_revision: str,
                    document_revision: str | None) -> dict:
        with self._lock:
            return self._plans.create(
                operation_input,
                preview,
                base_revision=base_revision,
                document_revision=document_revision,
            )

    def apply(self, plan_id: str) -> dict:
        with self._lock:
            operation, state = self._get(plan_id)
            self._expire(operation, state)
            if state.status != "planned":
                raise AgentRequestError(
                    "operation plan 当前状态不可执行",
                    {"plan_id": plan_id, "status": state.status},
                )
            if not self._database().enqueue(plan_id, now_ms()):
                _operation, current_state = self._get(plan_id)
                raise AgentRequestError(
                    "operation plan 当前状态不可执行",
                    {"plan_id": plan_id, "status": current_state.status},
                )
            self._jobs[plan_id] = self._executor.submit(self._run, plan_id)
        return self.status(plan_id)

    def _run(self, plan_id: str) -> None:
        with self._lock:
            operation, state = self._get(plan_id)
            if state.status != "queued":
                return
            if not self._database().claim_queued(plan_id, now_ms()):
                return
        try:
            if operation.kind == "edit":
                result = self._apply_edit(operation)
            elif operation.kind == "migration":
                result = self._apply_migration(operation)
            elif operation.kind == "metadata":
                result = self._apply_metadata(operation)
            elif operation.kind == "delete":
                result = self._apply_delete(operation)
            elif operation.kind == "restore-delete":
                result = self._apply_restore_delete(operation)
            else:
                raise AgentRequestError(
                    "operation kind 非法", {"kind": operation.kind},
                )
        except Exception as error:
            with self._lock:
                self._database().fail(
                    plan_id, type(error).__name__, now_ms(),
                )
            raise

        result_json = canonical_json(result)
        with self._lock:
            self._database().finish(
                plan_id, result_json, digest_json(result_json), now_ms(),
            )

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
            if state.status not in {"planned", "queued"}:
                raise AgentRequestError(
                    "仅 planned 或 queued operation 可以取消",
                    {"plan_id": plan_id, "status": state.status},
                )
            if not self._database().cancel(plan_id, state.status, now_ms()):
                raise AgentRequestError(
                    "operation plan 当前状态不可取消",
                    {"plan_id": plan_id},
                )
            return {"plan_id": plan_id, "status": "cancelled"}

    def wait(self, plan_id: str, timeout: float | None = None) -> dict:
        """测试与进程内编排辅助：等待已排队任务，不属于 RPC surface。"""
        with self._lock:
            job = self._jobs.get(plan_id)
        if job is not None:
            job.result(timeout=timeout)
        return self.status(plan_id)

    def shutdown(self) -> None:
        """仅在 Engine 重建或测试清理时调用，确保不遗留后台写入线程。"""
        self._executor.shutdown(wait=True, cancel_futures=True)

    def audit(self, plan_id: str) -> list[dict]:
        with self._lock:
            self._get(plan_id)
            return self._database().audit(plan_id)

    def _apply_edit(self, operation: OperationPlan) -> dict:
        return self._edit.apply(operation, self._finish_mutation)

    def _finish_mutation(self, tool, editor, result, document, snapshot, probe):
        if not probe:
            return result
        try:
            report = self._ports.adapter(tool).verifier.probe_edited(
                editor, document, result)
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
        session = agent_tools._read_record(self._index, record)
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
            record = self._index.resolve(
                params["tool"], params["ref"],
            )
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
            params["tool"], params["session_id"],
            params["metadata_before"],
            params["patch"],
            self._ports,
        )
        return {"metadata": result}

    def _apply_delete(self, operation: OperationPlan) -> dict:
        params = operation.input()
        try:
            record = self._index.resolve(
                params["tool"], params["ref"],
            )
        except AgentReferenceError as error:
            raise ConcurrentModificationError(
                "会话在删除计划生成后已变化，请重新计划"
            ) from error
        if record.revision != operation.base_revision:
            raise ConcurrentModificationError(
                "会话在删除计划生成后已变化，请重新计划"
            )
        result = SessionDeletionService(self._ports).delete(
            params["tool"], record.canonical_ref)
        snapshot = result.pop("snapshot", None)
        if result.get("undoable") is True:
            if not isinstance(snapshot, str) or not snapshot:
                raise RuntimeError("可恢复删除未返回快照")
            recovery_id = "recovery_" + secrets.token_urlsafe(18)
            self._database().store_recovery(
                recovery_id, params["tool"], snapshot, now_ms(),
            )
            result["recovery_id"] = recovery_id
        return result

    def _apply_restore_delete(self, operation: OperationPlan) -> dict:
        recovery_id = operation.input()["recovery_id"]
        recovery = self._database().get_recovery(recovery_id)
        if recovery is None or recovery["status"] != "available":
            raise ConcurrentModificationError("删除恢复记录已使用或不可用")
        if not self._database().claim_recovery(recovery_id, now_ms()):
            raise ConcurrentModificationError("删除恢复记录已使用或不可用")
        try:
            result = self._restore_deleted_session(recovery["snapshot"])
        except Exception:
            self._database().release_recovery(recovery_id, now_ms())
            raise
        if not self._database().complete_recovery(recovery_id, now_ms()):
            raise RuntimeError("删除恢复状态提交失败")
        return {**result, "recovery_id": recovery_id}

    def _restore_deleted_session(self, snapshot: str) -> dict:
        return SessionDeletionService(self._ports).restore(snapshot)

    def _get(self, plan_id: str) -> tuple[OperationPlan, OperationState]:
        return self._plans.get(plan_id)

    def _expire(self, operation: OperationPlan, state: OperationState) -> None:
        self._plans.expire(operation, state)
