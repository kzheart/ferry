"""统一写操作计划。"""
from __future__ import annotations

import json
import secrets
import threading
from concurrent.futures import Future, ThreadPoolExecutor

from ..sessions import catalog as agent_tools
from ..context import EngineContext
from ..operations.types import AssistantReply
from ..errors import (
    AgentReferenceError,
    AgentRequestError,
    ConcurrentModificationError,
    LocatorStaleError,
    OperationUnsupportedError,
)
from . import metadata, verification as probe_mod
from .delete import SessionDeletionService
from .edit import apply_mutation, preview_mutation
from .migrate import MigrationService
from .plan_store import (
    OperationPlan,
    OperationPlanStore,
    OperationState,
    canonical_json,
    digest_json,
    now_ms,
)
from ..storage.database import StateDatabase


MUTATION_WORKERS = 1


class OperationService:
    def __init__(self, ports: EngineContext,
                 index: agent_tools.AgentSessionIndex):
        self._ports = ports
        self._index = index
        self._migration = MigrationService(ports)
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
        raise AgentRequestError(
            "operation kind 非法", {"kind": value.get("kind")},
        )

    def _plan_edit(self, value: dict) -> dict:
        operation_input = self._validate_edit_input(value)
        tool = operation_input["tool"]
        ref = operation_input["ref"]

        before = self._index.resolve(tool, ref)
        preview = self._preview_edit(before, operation_input["ops"])
        after = self._index.resolve(tool, ref)
        if before.revision != after.revision:
            raise ConcurrentModificationError(
                "会话在生成操作计划时已变化，请重新计划"
            )
        adapter = self._ports.adapter(tool)
        editor = adapter.editor
        try:
            native_ops = self._resolve_ops(after, operation_input["ops"])
        except LocatorStaleError as error:
            raise self._public_locator_error(operation_input["ops"]) from error
        self._require_inplace_support(adapter, editor, native_ops)

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
        operation_input = self._validate_metadata_input(value)
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
        operation_input = self._validate_delete_input(value)
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
        operation_input = self._validate_restore_delete_input(value)
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
        if len(canonical_json(ops).encode()) > 64 * 1024:
            raise AgentRequestError("ops 超过 64 KiB")
        return json.loads(canonical_json({
            "kind": "edit", "tool": tool, "ref": ref, "ops": ops,
            "probe": probe,
        }))

    def _validate_migration_input(self, value: dict) -> dict:
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
        adapters = self._ports.adapters()
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
        return json.loads(canonical_json(result))

    @staticmethod
    def _validate_metadata_input(value: dict) -> dict:
        allowed = {"kind", "tool", "ref", "patch"}
        unknown = set(value) - allowed
        if unknown:
            raise AgentRequestError(
                "metadata operation 包含未知字段",
                {"fields": sorted(unknown)},
            )
        tool = value.get("tool")
        ref = value.get("ref")
        patch = value.get("patch")
        if not isinstance(tool, str) or not 1 <= len(tool) <= 64:
            raise AgentRequestError("metadata tool 非法")
        if (not isinstance(ref, str) or not 1 <= len(ref) <= 512
                or any(ord(character) < 33 for character in ref)):
            raise AgentRequestError("metadata ref 非法")
        allowed_fields = {"name", "pinned", "archived", "tags"}
        if not isinstance(patch, dict) or not patch or not set(patch) <= allowed_fields:
            raise AgentRequestError("metadata patch 字段非法")
        agent_tools._validate_json_shape(patch, max_depth=3, max_nodes=50)
        if ("name" in patch and
                (not isinstance(patch["name"], str)
                 or len(patch["name"]) > 200)):
            raise AgentRequestError("metadata name 非法")
        for field in ("pinned", "archived"):
            if field in patch and not isinstance(patch[field], bool):
                raise AgentRequestError(f"metadata {field} 必须是 boolean")
        if "tags" in patch:
            tags = patch["tags"]
            if (not isinstance(tags, list) or len(tags) > 20
                    or not all(
                        isinstance(tag, str) and 1 <= len(tag) <= 64
                        for tag in tags
                    )):
                raise AgentRequestError("metadata tags 非法")
        if len(canonical_json(patch).encode()) > 4096:
            raise AgentRequestError("metadata patch 超过 4 KiB")
        return json.loads(canonical_json({
            "kind": "metadata",
            "tool": tool,
            "ref": ref,
            "patch": patch,
        }))

    def _validate_delete_input(self, value: dict) -> dict:
        allowed = {"kind", "tool", "ref"}
        unknown = set(value) - allowed
        if unknown:
            raise AgentRequestError(
                "delete operation 包含未知字段",
                {"fields": sorted(unknown)},
            )
        tool = value.get("tool")
        ref = value.get("ref")
        if tool not in self._ports.adapters():
            raise AgentRequestError("delete tool 非法")
        if (not isinstance(ref, str) or not 1 <= len(ref) <= 512
                or any(ord(character) < 33 for character in ref)):
            raise AgentRequestError("delete ref 非法")
        return {"kind": "delete", "tool": tool, "ref": ref}

    @staticmethod
    def _validate_restore_delete_input(value: dict) -> dict:
        if set(value) != {"kind", "recovery_id"}:
            raise AgentRequestError("restore-delete operation 参数非法")
        recovery_id = value.get("recovery_id")
        if (not isinstance(recovery_id, str)
                or not recovery_id.startswith("recovery_")
                or not 16 <= len(recovery_id) <= 128
                or not all(
                    character.isalnum() or character in "_-"
                    for character in recovery_id
                )):
            raise AgentRequestError("recovery_id 非法")
        return {
            "kind": "restore-delete",
            "recovery_id": recovery_id,
        }

    def _apply_edit(self, operation: OperationPlan) -> dict:
        params = operation.input()
        try:
            record = self._index.resolve(
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
        adapter = self._ports.adapter(params["tool"])
        editor = adapter.editor
        native_ops = self._resolve_ops(record, params["ops"])
        self._require_inplace_support(adapter, editor, native_ops)
        try:
            if not any(
                op["op"] == "replace-assistant-reply" for op in native_ops
            ):
                from .edit import apply
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
                    self._mutate(editor, native_ops),
                    expected_revision=operation.document_revision,
                )
        except LocatorStaleError as error:
            raise self._public_locator_error(params["ops"]) from error
        return self._finish_mutation(
            params["tool"], editor, result, doc, snapshot, params["probe"],
        )

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

    @staticmethod
    def _validate_ops(ops) -> list[dict]:
        if not isinstance(ops, list) or not ops or len(ops) > 50:
            raise AgentRequestError("ops 必须是 1 到 50 项的数组")
        agent_tools._validate_json_shape(ops)
        ordinary = []
        normalized = []
        replaced_turns = []
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
            if turn_key in replaced_turns:
                raise AgentRequestError(
                    "同一轮次不能在一次编辑中重复替换",
                    {"field": "ops.turn"},
                )
            replaced_turns.append(turn_key)
            normalized.append({
                "op": "replace-assistant-reply",
                "turn": turn,
                "reply": reply.to_dict(),
            })
        if ordinary:
            agent_tools._validate_ops(ordinary)
        return normalized

    def _resolve_ops(self, record, ops: list[dict]) -> list[dict]:
        resolved = []
        for op in ops:
            if op["op"] == "replace-assistant-reply":
                resolved.append(dict(op))
            else:
                resolved.extend(agent_tools.resolve_edit_ops(self._index, record, [op]))
        return resolved

    @staticmethod
    def _require_inplace_support(adapter, editor, ops: list[dict]):
        ordinary = [
            op for op in ops if op["op"] != "replace-assistant-reply"
        ]
        replacements = [
            op for op in ops if op["op"] == "replace-assistant-reply"
        ]
        if ordinary and not all(
                op["op"] in editor.operations for op in ordinary):
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
    def _mutate(editor, ops: list[dict]):
        def mutate(doc):
            changes = []
            for op in ops:
                if op["op"] == "replace-assistant-reply":
                    changes.extend(editor.replace_reply(
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
                record.tool, record.opaque_ref, ops=ops, index=self._index,
            )
        adapter = self._ports.adapter(record.tool)
        editor = adapter.editor
        native_ops = self._resolve_ops(record, ops)
        self._require_inplace_support(
            adapter, editor, native_ops,
        )
        try:
            result = preview_mutation(
                editor,
                record.canonical_ref,
                self._mutate(editor, native_ops),
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
        return self._plans.get(plan_id)

    def _expire(self, operation: OperationPlan, state: OperationState) -> None:
        self._plans.expire(operation, state)
